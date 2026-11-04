//! Win32 calls of the desktop integration (ADR 0020): the main window's visibility, the
//! single instance's pipe, and the double click and screen of the notification area's icon;
//! DPAPI, which encrypts the cookie jar (ADR 0022); and Windows' display language (ADR 0014). The only `unsafe` of the `peeby` crate,
//! each block with its `// SAFETY:`.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::time::Duration;

use windows::Win32::Foundation::{
  E_INVALIDARG, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, HWND, LocalFree, POINT,
};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::Security::Cryptography::{
  CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{
  ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
  PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindow};
use windows::core::{HSTRING, PCWSTR};

/// The pipes' folder. A pipe is named after the profile and the user: pipe names are
/// machine-wide, and another user's Peeby (or a debug build) is another instance.
const PIPE: &str = r"\\.\pipe\";
/// A launch's arguments, at most.
const MAX_MESSAGE: u64 = 64 * 1024;

/// Hides the window `hwnd` (a value from GPUI's `raw_window_handle`).
pub fn hide(hwnd: isize) {
  // SAFETY: `ShowWindow` accepts any value: an `hwnd` that is no longer a window only fails.
  unsafe {
    let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE);
  }
}

/// Shows the window `hwnd` again, restored if it was minimized. GPUI's `activate_window`
/// brings it to the front afterwards.
pub fn show(hwnd: isize) {
  let hwnd = HWND(hwnd as *mut _);
  // SAFETY: as in `hide`, any value is accepted.
  unsafe {
    let command = if IsIconic(hwnd).as_bool() {
      SW_RESTORE
    } else {
      SW_SHOW
    };
    let _ = ShowWindow(hwnd, command);
  }
}

/// The primary language of Windows' display language for the user (`PRIMARYLANGID` of
/// `GetUserDefaultUILanguage`), e.g. `0x0c` for French.
pub fn ui_primary_language() -> u16 {
  // SAFETY: no arguments; it only reads a setting.
  unsafe { GetUserDefaultUILanguage() & 0x3ff }
}

/// The longest time between the two clicks of a double click (the user's mouse setting).
pub fn double_click_time() -> Duration {
  // SAFETY: no arguments; it only reads a setting.
  Duration::from_millis(unsafe { GetDoubleClickTime() }.into())
}

/// The screen at the physical point `(x, y)`, or the nearest one: its handle (GPUI's
/// `DisplayId`) and its scale factor (1 at 96 dpi).
pub fn monitor_at(x: i32, y: i32) -> (u64, f32) {
  // SAFETY: any point is accepted; `MONITOR_DEFAULTTONEAREST` always returns a screen.
  let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
  let (mut dpi_x, mut dpi_y) = (96, 96);
  // SAFETY: `monitor` comes from `MonitorFromPoint`, the outputs are local; on failure they
  // keep 96.
  let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
  (monitor.0 as u64, dpi_x as f32 / 96.)
}

/// This process and the other launches of Peeby.
pub enum Instance {
  /// The first one: `listen` hands over what the next launches send.
  First(Listener),
  /// Another Peeby runs: what this launch has to say was sent to it.
  Running,
}

/// The first instance's end of the pipe, waiting for the next launches.
pub struct Listener {
  handle: HANDLE,
  name: String,
}

// SAFETY: a pipe handle is a kernel object, usable from any thread; only the listening thread
// uses it once the listener moves there.
unsafe impl Send for Listener {}

/// Claims the single instance, or sends `message` (this launch's arguments, one a line) to
/// the instance that runs. `None` when the pipe cannot be made at all: this process goes on,
/// alone.
pub fn claim(message: &str) -> Option<Instance> {
  let name = pipe_name();
  match create_pipe(&name, true) {
    Ok(handle) => Some(Instance::First(Listener { handle, name })),
    Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
      // The other instance's pipe exists: it may be between two connections, so try a few times.
      for _ in 0..20 {
        if let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(&name) {
          return match pipe.write_all(message.as_bytes()) {
            Ok(()) => Some(Instance::Running),
            Err(e) => {
              tracing::warn!("cannot reach the running Peeby: {e}");
              None
            }
          };
        }
        std::thread::sleep(Duration::from_millis(50));
      }
      // Going on alone would rewrite the same cookie jar and spend the same Twitch refresh
      // token: better not to start.
      tracing::error!("another Peeby runs but does not answer: quit it first");
      Some(Instance::Running)
    }
    Err(e) => {
      tracing::warn!("cannot claim the single instance: {e}");
      None
    }
  }
}

impl Listener {
  /// Waits for the next launches, forever (a thread of its own), and calls `on_message` with
  /// what each one sent. The next end of the pipe is made before reading the last one, so the
  /// pipe never disappears: a launch in between would believe it is the first.
  pub fn listen(self, on_message: impl Fn(String)) {
    let mut current = self.handle;
    loop {
      // SAFETY: `current` is a pipe end this loop made and owns; a client that connected
      // between `CreateNamedPipeW` and here gives `ERROR_PIPE_CONNECTED`, a success.
      let connected = unsafe { ConnectNamedPipe(current, None) };
      if let Err(e) = connected
        && e.code() != ERROR_PIPE_CONNECTED.to_hresult()
      {
        tracing::warn!("single instance pipe: {e}");
      }
      let next = create_pipe(&self.name, false);
      // SAFETY: the handle is owned here and moves into the `File`, which closes it.
      let mut pipe = unsafe { File::from_raw_handle(current.0 as RawHandle) };
      let mut message = String::new();
      if let Err(e) = (&mut pipe).take(MAX_MESSAGE).read_to_string(&mut message) {
        tracing::warn!("single instance pipe: {e}");
      }
      drop(pipe);
      on_message(message);
      match next {
        Ok(handle) => current = handle,
        Err(e) => {
          tracing::warn!("single instance pipe closed: {e}");
          return;
        }
      }
    }
  }
}

/// This user's pipe: [`PIPE`], the profile and the user name, letters and digits only.
fn pipe_name() -> String {
  let user: String = std::env::var("USERNAME")
    .unwrap_or_default()
    .chars()
    .filter(char::is_ascii_alphanumeric)
    .collect();
  format!("{PIPE}{}-{user}", crate::settings::PROFILE)
}

/// A new end of the pipe `name`; `first` fails with `ERROR_ACCESS_DENIED` when it exists.
fn create_pipe(name: &str, first: bool) -> windows::core::Result<HANDLE> {
  let name = HSTRING::from(name);
  let mut open = PIPE_ACCESS_INBOUND;
  if first {
    open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
  }
  // SAFETY: `name` outlives the call and is null-terminated; no security attributes (the
  // default ones: only this user, the administrators and the system may write).
  let handle = unsafe {
    CreateNamedPipeW(
      PCWSTR(name.as_ptr()),
      open,
      PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
      PIPE_UNLIMITED_INSTANCES,
      0,
      4096,
      0,
      None,
    )
  };
  if handle.is_invalid() {
    return Err(windows::core::Error::from_thread());
  }
  Ok(handle)
}

/// Encrypts `data` with DPAPI for the current Windows user: only this user, on this machine,
/// can decrypt it ([`unprotect`] with the same `entropy`). For the cookie jar (ADR 0022).
pub fn protect(data: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
  let input = blob(data)?;
  let entropy = blob(entropy)?;
  let mut output = CRYPT_INTEGER_BLOB::default();
  // SAFETY: `input` and `entropy` point into slices that outlive the call, which only reads
  // them. On success `output` holds a buffer DPAPI allocated, copied then freed by `take`.
  unsafe {
    CryptProtectData(
      &input,
      PCWSTR::null(),
      Some(&entropy),
      None,
      None,
      CRYPTPROTECT_UI_FORBIDDEN,
      &mut output,
    )?;
    Ok(take(output))
  }
}

/// Decrypts what [`protect`] gave, for the same user and `entropy`.
pub fn unprotect(data: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
  let input = blob(data)?;
  let entropy = blob(entropy)?;
  let mut output = CRYPT_INTEGER_BLOB::default();
  // SAFETY: as in `protect`; no description is asked for.
  unsafe {
    CryptUnprotectData(
      &input,
      None,
      Some(&entropy),
      None,
      None,
      CRYPTPROTECT_UI_FORBIDDEN,
      &mut output,
    )?;
    Ok(take(output))
  }
}

/// A DPAPI view of `bytes`, which DPAPI only reads (hence the `cast_mut`).
fn blob(bytes: &[u8]) -> windows::core::Result<CRYPT_INTEGER_BLOB> {
  Ok(CRYPT_INTEGER_BLOB {
    cbData: u32::try_from(bytes.len()).map_err(|_| windows::core::Error::from(E_INVALIDARG))?,
    pbData: bytes.as_ptr().cast_mut(),
  })
}

/// Copies a buffer DPAPI returned, then frees it.
///
/// # Safety
/// `blob` must come from a successful `CryptProtectData`/`CryptUnprotectData` and not be used
/// afterwards.
unsafe fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
  if blob.pbData.is_null() {
    return Vec::new();
  }
  // SAFETY: DPAPI wrote `cbData` bytes at `pbData` (the caller's contract).
  let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
  // SAFETY: the buffer was allocated with `LocalAlloc` by DPAPI, which leaves freeing it to us.
  unsafe {
    let _ = LocalFree(Some(HLOCAL(blob.pbData.cast())));
  }
  bytes
}

#[cfg(test)]
mod tests {
  use super::{protect, unprotect};

  #[test]
  fn dpapi_round_trip() {
    let secret = b".youtube.com\tTRUE\t/\tTRUE\t0\tSID\tvalue\n";
    let sealed = protect(secret, b"entropy").unwrap();
    assert!(!sealed.windows(5).any(|w| w == b"value"));
    assert_eq!(unprotect(&sealed, b"entropy").unwrap(), secret);
    assert!(unprotect(&sealed, b"other").is_err());
  }
}
