//! "Start with Windows": a value of the user's `Run` key that starts Peeby in the notification
//! area (ADR 0020).

use windows_registry::CURRENT_USER;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Peeby";
/// The argument of the `Run` key's command: Peeby starts without showing its window.
pub const BACKGROUND: &str = "--background";

/// Adds or removes the `Run` value, which starts this executable. Blocking (the registry):
/// called in the background, at startup (the executable may have moved) and on a change.
pub fn set(on: bool) {
  let command = if on {
    match std::env::current_exe() {
      Ok(exe) => Some(format!("\"{}\" {BACKGROUND}", exe.display())),
      Err(e) => {
        tracing::warn!("cannot start with Windows: {e}");
        return;
      }
    }
  } else {
    None
  };
  let result = CURRENT_USER.create(RUN).and_then(|key| match &command {
    Some(command) => key.set_string(VALUE, command),
    None if key.get_type(VALUE).is_ok() => key.remove_value(VALUE),
    // Nothing to remove.
    None => Ok(()),
  });
  if let Err(e) = result {
    tracing::warn!("cannot change the start with Windows: {e}");
  }
}
