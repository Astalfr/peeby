//! Minimal hand-written libmpv FFI, loaded at runtime with `libloading`.
//!
//! Only what Peeby uses. Values come from the pinned headers in third_party/mpv/include/mpv/.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

/// Opaque `mpv_handle`.
#[repr(C)]
pub struct Handle {
  _private: [u8; 0],
}

/// Opaque `mpv_render_context`.
#[repr(C)]
pub struct RenderContext {
  _private: [u8; 0],
}

/// `mpv_render_param`.
#[repr(C)]
pub struct RenderParam {
  pub ty: c_int,
  pub data: *mut c_void,
}

/// `mpv_event`.
#[repr(C)]
pub struct Event {
  pub event_id: c_int,
  pub error: c_int,
  pub reply_userdata: u64,
  pub data: *mut c_void,
}

pub const RENDER_PARAM_INVALID: c_int = 0;
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const RENDER_PARAM_SW_SIZE: c_int = 17;
pub const RENDER_PARAM_SW_FORMAT: c_int = 18;
pub const RENDER_PARAM_SW_STRIDE: c_int = 19;
pub const RENDER_PARAM_SW_POINTER: c_int = 20;
pub const RENDER_PARAM_D3D11_INIT_PARAMS: c_int = 21;
pub const RENDER_PARAM_D3D11_FBO: c_int = 22;
pub const RENDER_UPDATE_FRAME: u64 = 1;

/// `mpv_d3d11_init_params` (render_d3d11.h, not shipped in the dev package; taken from mpv@e471496c56).
#[repr(C)]
pub struct D3d11InitParams {
  pub device: *mut c_void,
}

/// `mpv_d3d11_fbo`.
#[repr(C)]
pub struct D3d11Fbo {
  pub tex: *mut c_void,
}

/// Which render API the render context uses.
pub enum RenderBackend {
  /// `MPV_RENDER_API_TYPE_SW`.
  Software,
  /// `MPV_RENDER_API_TYPE_D3D11` on this `ID3D11Device*` (mpv adds its own reference).
  D3d11 { device: *mut c_void },
  /// No render context (`vo=null`): an mpv only queried, e.g. for its audio devices.
  None,
}

pub const FORMAT_NONE: c_int = 0;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_DOUBLE: c_int = 5;

pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

/// `mpv_event_property`.
#[repr(C)]
struct EventProperty {
  name: *const c_char,
  format: c_int,
  data: *mut c_void,
}

/// `mpv_event_end_file` (leading fields only; it is only read, never allocated by us).
#[repr(C)]
struct EventEndFile {
  reason: c_int,
  error: c_int,
}

/// The mpv client API version the FFI below was written against (client.h 2.6).
pub const EXPECTED_API_MAJOR: u64 = 2;

/// A decoded mpv event.
#[derive(Debug)]
pub enum MpvEvent {
  /// Timeout or wakeup.
  None,
  /// The core is shutting down.
  Shutdown,
  /// The file is open and its tracks are known.
  FileLoaded,
  /// Playback of the file ended. `reason`: 0 EOF, 2 stop, 3 quit, 4 error.
  EndFile { reason: i32, error: i32 },
  /// An observed `DOUBLE` property changed (`None`: unavailable).
  Double(String, Option<f64>),
  /// An observed `FLAG` property changed.
  Flag(String, Option<bool>),
  /// An observed property without value changed (read it with [`Mpv::property`]).
  Changed(String),
  /// Any other event id.
  Other(c_int),
}
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;

type UpdateCallback = unsafe extern "C" fn(*mut c_void);

/// Function table resolved from `libmpv-2.dll`.
pub struct Api {
  _lib: libloading::Library,
  pub create: unsafe extern "C" fn() -> *mut Handle,
  pub initialize: unsafe extern "C" fn(*mut Handle) -> c_int,
  pub terminate_destroy: unsafe extern "C" fn(*mut Handle),
  pub client_api_version: unsafe extern "C" fn() -> std::ffi::c_ulong,
  pub set_option_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
  pub set_property_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
  pub observe_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int,
  pub get_property_string: unsafe extern "C" fn(*mut Handle, *const c_char) -> *mut c_char,
  pub command: unsafe extern "C" fn(*mut Handle, *mut *const c_char) -> c_int,
  pub wait_event: unsafe extern "C" fn(*mut Handle, f64) -> *mut Event,
  pub wakeup: unsafe extern "C" fn(*mut Handle),
  pub free: unsafe extern "C" fn(*mut c_void),
  pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
  pub render_context_create:
    unsafe extern "C" fn(*mut *mut RenderContext, *mut Handle, *mut RenderParam) -> c_int,
  pub render_context_set_update_callback:
    unsafe extern "C" fn(*mut RenderContext, Option<UpdateCallback>, *mut c_void),
  pub render_context_update: unsafe extern "C" fn(*mut RenderContext) -> u64,
  pub render_context_render: unsafe extern "C" fn(*mut RenderContext, *mut RenderParam) -> c_int,
  pub render_context_free: unsafe extern "C" fn(*mut RenderContext),
}

macro_rules! sym {
  ($lib:expr, $name:literal) => {
    // SAFETY: the symbol type matches the prototype in the pinned mpv headers.
    *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{}: {e}", $name))?
  };
}

impl Api {
  /// Loads libmpv from `path` and resolves every function the spike uses.
  pub fn load(path: &Path) -> Result<Self, String> {
    // SAFETY: loading libmpv runs its DLL initializers, which have no preconditions.
    let lib =
      unsafe { libloading::Library::new(path) }.map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Self {
      create: sym!(lib, "mpv_create"),
      initialize: sym!(lib, "mpv_initialize"),
      terminate_destroy: sym!(lib, "mpv_terminate_destroy"),
      client_api_version: sym!(lib, "mpv_client_api_version"),
      set_option_string: sym!(lib, "mpv_set_option_string"),
      set_property_string: sym!(lib, "mpv_set_property_string"),
      observe_property: sym!(lib, "mpv_observe_property"),
      get_property_string: sym!(lib, "mpv_get_property_string"),
      command: sym!(lib, "mpv_command"),
      wait_event: sym!(lib, "mpv_wait_event"),
      wakeup: sym!(lib, "mpv_wakeup"),
      free: sym!(lib, "mpv_free"),
      error_string: sym!(lib, "mpv_error_string"),
      render_context_create: sym!(lib, "mpv_render_context_create"),
      render_context_set_update_callback: sym!(lib, "mpv_render_context_set_update_callback"),
      render_context_update: sym!(lib, "mpv_render_context_update"),
      render_context_render: sym!(lib, "mpv_render_context_render"),
      render_context_free: sym!(lib, "mpv_render_context_free"),
      _lib: lib,
    })
  }

  /// Fails if the DLL speaks another major version of the client API than this FFI.
  pub fn check_version(&self) -> Result<(), String> {
    // SAFETY: no arguments, returns a constant.
    let version = unsafe { (self.client_api_version)() } as u64;
    let major = version >> 16;
    if major == EXPECTED_API_MAJOR {
      Ok(())
    } else {
      Err(format!(
        "libmpv client API {major}.{} is not supported (expected {EXPECTED_API_MAJOR}.x)",
        version & 0xffff
      ))
    }
  }

  /// Turns an mpv error code into a `Result`.
  pub fn check(&self, code: c_int, what: &str) -> Result<(), String> {
    if code >= 0 {
      return Ok(());
    }
    // SAFETY: mpv_error_string returns a static NUL-terminated string for any code.
    let msg = unsafe { CStr::from_ptr((self.error_string)(code)) };
    Err(format!("{what}: {}", msg.to_string_lossy()))
  }
}

/// Owned mpv handle plus its software render context.
pub struct Mpv {
  pub api: Api,
  pub handle: *mut Handle,
  pub render: *mut RenderContext,
}

// SAFETY: the mpv client API is thread-safe. The render context is only used from the render
// thread while the update callback runs on mpv threads, as render.h allows.
unsafe impl Send for Mpv {}
// SAFETY: see above.
unsafe impl Sync for Mpv {}

impl Mpv {
  /// Creates mpv with `options` (set before init) and a render context for `backend`.
  pub fn new(api: Api, options: &[(&str, &str)], backend: RenderBackend) -> Result<Self, String> {
    // SAFETY: plain constructor, returns null on failure.
    let handle = unsafe { (api.create)() };
    if handle.is_null() {
      return Err("mpv_create failed".into());
    }
    let mut mpv = Self {
      api,
      handle,
      render: ptr::null_mut(),
    };
    for (name, value) in options {
      let (n, v) = (cstr(name), cstr(value));
      // SAFETY: valid handle and NUL-terminated strings that outlive the call.
      let code = unsafe { (mpv.api.set_option_string)(mpv.handle, n.as_ptr(), v.as_ptr()) };
      // Values are never echoed: they can hold credentials (HTTP headers).
      mpv.api.check(code, &format!("option {name}"))?;
    }
    // SAFETY: valid, not yet initialized handle.
    mpv.api.check(
      unsafe { (mpv.api.initialize)(mpv.handle) },
      "mpv_initialize",
    )?;

    let (api_type, mut d3d11_init) = match backend {
      RenderBackend::None => return Ok(mpv),
      RenderBackend::Software => (
        cstr("sw"),
        D3d11InitParams {
          device: ptr::null_mut(),
        },
      ),
      RenderBackend::D3d11 { device } => (cstr("d3d11"), D3d11InitParams { device }),
    };
    let mut params = vec![RenderParam {
      ty: RENDER_PARAM_API_TYPE,
      data: api_type.as_ptr() as *mut c_void,
    }];
    if !d3d11_init.device.is_null() {
      params.push(RenderParam {
        ty: RENDER_PARAM_D3D11_INIT_PARAMS,
        data: (&mut d3d11_init as *mut D3d11InitParams).cast(),
      });
    }
    params.push(RenderParam {
      ty: RENDER_PARAM_INVALID,
      data: ptr::null_mut(),
    });
    let mut render = ptr::null_mut();
    // SAFETY: params is a type==0 terminated array whose data outlives the call.
    let code =
      unsafe { (mpv.api.render_context_create)(&mut render, mpv.handle, params.as_mut_ptr()) };
    mpv.api.check(code, "mpv_render_context_create")?;
    mpv.render = render;
    Ok(mpv)
  }

  /// Runs an mpv command such as `["loadfile", url]`.
  pub fn command(&self, args: &[&str]) -> Result<(), String> {
    let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
    let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(ptr::null());
    // SAFETY: null-terminated array of valid C strings that outlive the call.
    let code = unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) };
    // Only the command name: arguments can hold credentials or private URLs.
    self.api.check(
      code,
      &format!("command {}", args.first().copied().unwrap_or_default()),
    )
  }

  /// Reads a property as a string, `None` when unavailable.
  pub fn property(&self, name: &str) -> Option<String> {
    let n = cstr(name);
    // SAFETY: valid handle; the result is owned by us and released with mpv_free.
    let raw = unsafe { (self.api.get_property_string)(self.handle, n.as_ptr()) };
    if raw.is_null() {
      return None;
    }
    // SAFETY: mpv returns a NUL-terminated UTF-8 string.
    let value = unsafe { CStr::from_ptr(raw) }
      .to_string_lossy()
      .into_owned();
    // SAFETY: raw was allocated by mpv and is freed exactly once.
    unsafe { (self.api.free)(raw.cast()) };
    Some(value)
  }

  /// Installs the render update callback. `ctx` must stay valid until the render context is freed.
  pub fn set_update_callback(&self, callback: UpdateCallback, ctx: *mut c_void) {
    // SAFETY: valid render context; the caller guarantees ctx outlives it.
    unsafe { (self.api.render_context_set_update_callback)(self.render, Some(callback), ctx) };
  }

  /// Returns the `mpv_render_update_flag` bits.
  pub fn render_update(&self) -> u64 {
    // SAFETY: valid render context, called from the render thread only.
    unsafe { (self.api.render_context_update)(self.render) }
  }

  /// Renders the current frame into `buf` (`bgr0`, `width * 4` bytes per row).
  /// With `block` false, returns immediately instead of waiting for the frame's display time.
  pub fn render_sw(
    &self,
    width: u32,
    height: u32,
    buf: &mut [u8],
    block: bool,
  ) -> Result<(), String> {
    let mut size = [width as c_int, height as c_int];
    let format = cstr("bgr0");
    let mut stride = width as usize * 4;
    let mut block_flag = c_int::from(block);
    debug_assert_eq!(buf.len(), stride * height as usize);
    let mut params = [
      RenderParam {
        ty: RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
        data: (&mut block_flag as *mut c_int).cast(),
      },
      RenderParam {
        ty: RENDER_PARAM_SW_SIZE,
        data: size.as_mut_ptr().cast(),
      },
      RenderParam {
        ty: RENDER_PARAM_SW_FORMAT,
        data: format.as_ptr() as *mut c_void,
      },
      RenderParam {
        ty: RENDER_PARAM_SW_STRIDE,
        data: (&mut stride as *mut usize).cast(),
      },
      RenderParam {
        ty: RENDER_PARAM_SW_POINTER,
        data: buf.as_mut_ptr().cast(),
      },
      RenderParam {
        ty: RENDER_PARAM_INVALID,
        data: ptr::null_mut(),
      },
    ];
    // SAFETY: params point to live locals and a buffer of exactly stride * height bytes.
    let code = unsafe { (self.api.render_context_render)(self.render, params.as_mut_ptr()) };
    self.api.check(code, "mpv_render_context_render")
  }

  /// Renders the current frame into `texture` (`ID3D11Texture2D*` on the render context's device).
  pub fn render_d3d11(&self, texture: *mut c_void) -> Result<(), String> {
    let mut fbo = D3d11Fbo { tex: texture };
    let mut params = [
      RenderParam {
        ty: RENDER_PARAM_D3D11_FBO,
        data: (&mut fbo as *mut D3d11Fbo).cast(),
      },
      RenderParam {
        ty: RENDER_PARAM_INVALID,
        data: ptr::null_mut(),
      },
    ];
    // SAFETY: params point to live locals; the caller guarantees `texture` is a valid render target.
    let code = unsafe { (self.api.render_context_render)(self.render, params.as_mut_ptr()) };
    self.api.check(code, "mpv_render_context_render (d3d11)")
  }

  /// Waits up to `timeout` seconds (negative: forever) for the next event.
  pub fn next_event(&self, timeout: f64) -> MpvEvent {
    // SAFETY: valid handle; the event and its data are owned by mpv and stay valid until the
    // next wait_event call on this handle, which only this thread makes.
    unsafe {
      let event = &*(self.api.wait_event)(self.handle, timeout);
      match event.event_id {
        EVENT_NONE => MpvEvent::None,
        EVENT_SHUTDOWN => MpvEvent::Shutdown,
        EVENT_FILE_LOADED => MpvEvent::FileLoaded,
        EVENT_END_FILE if !event.data.is_null() => {
          let end = &*(event.data as *const EventEndFile);
          MpvEvent::EndFile {
            reason: end.reason,
            error: end.error,
          }
        }
        EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
          let prop = &*(event.data as *const EventProperty);
          let name = CStr::from_ptr(prop.name).to_string_lossy().into_owned();
          match prop.format {
            FORMAT_DOUBLE if !prop.data.is_null() => {
              MpvEvent::Double(name, Some(*(prop.data as *const f64)))
            }
            FORMAT_FLAG if !prop.data.is_null() => {
              MpvEvent::Flag(name, Some(*(prop.data as *const c_int) != 0))
            }
            FORMAT_DOUBLE => MpvEvent::Double(name, None),
            FORMAT_FLAG => MpvEvent::Flag(name, None),
            _ => MpvEvent::Changed(name),
          }
        }
        id => MpvEvent::Other(id),
      }
    }
  }

  /// Sets a property from its string form.
  pub fn set_property(&self, name: &str, value: &str) -> Result<(), String> {
    let (n, v) = (cstr(name), cstr(value));
    // SAFETY: valid handle and NUL-terminated strings that outlive the call.
    let code = unsafe { (self.api.set_property_string)(self.handle, n.as_ptr(), v.as_ptr()) };
    self.api.check(code, &format!("property {name}"))
  }

  /// Subscribes to changes of a property (`format`: one of the `FORMAT_*` constants).
  pub fn observe(&self, name: &str, format: c_int) -> Result<(), String> {
    let n = cstr(name);
    // SAFETY: valid handle; mpv copies the name.
    let code = unsafe { (self.api.observe_property)(self.handle, 0, n.as_ptr(), format) };
    self.api.check(code, &format!("observe {name}"))
  }

  /// Wakes a thread blocked in `wait_event`.
  pub fn wakeup(&self) {
    // SAFETY: valid handle.
    unsafe { (self.api.wakeup)(self.handle) };
  }
}

impl Drop for Mpv {
  fn drop(&mut self) {
    // SAFETY: the render context is freed before the handle, as render.h requires.
    unsafe {
      if !self.render.is_null() {
        (self.api.render_context_free)(self.render);
      }
      (self.api.terminate_destroy)(self.handle);
    }
  }
}

fn cstr(s: &str) -> CString {
  // Interior NULs cannot occur in the option names and URLs the spike passes; replace them anyway.
  CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// `PlaySoundW` flags (mmsystem.h): the sound is a WAV in memory, and nothing plays if it cannot be.
const SND_NODEFAULT: u32 = 0x0002;
const SND_MEMORY: u32 = 0x0004;

/// Plays a WAV held in memory on Windows' default output with `PlaySoundW`, from winmm.dll
/// loaded at runtime like libmpv, and returns once it has played.
pub fn play_wav(wav: &[u8]) -> Result<(), String> {
  use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};
  type PlaySoundW = unsafe extern "system" fn(*const c_void, *mut c_void, u32) -> i32;
  // SAFETY: winmm is a system DLL, taken from System32 only; its initializers have no
  // preconditions.
  let lib = unsafe { Library::load_with_flags("winmm.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
    .map_err(|e| format!("winmm.dll: {e}"))?;
  // SAFETY: the type matches `PlaySoundW` in mmsystem.h.
  let play =
    unsafe { lib.get::<PlaySoundW>(b"PlaySoundW ") }.map_err(|e| format!("PlaySoundW: {e}"))?;
  // SAFETY: with `SND_MEMORY` the first argument points to the WAV's bytes, which outlive the call:
  // without `SND_ASYNC` it returns once the sound has played. No module handle is needed.
  let played = unsafe {
    play(
      wav.as_ptr().cast(),
      ptr::null_mut(),
      SND_MEMORY | SND_NODEFAULT,
    )
  };
  if played == 0 {
    return Err("PlaySoundW failed".into());
  }
  Ok(())
}
