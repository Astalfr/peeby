//! Sign-in through a WebView (ADR 0017). Peeby starts itself again as `peeby --login <service>`:
//! a small window shows the service's own sign-in page in WebView2 (InPrivate), waits for the
//! session cookie, writes the session on stdout and closes. The space checks it as a manual
//! import (YouTube: Home, Twitch: `currentUser`) before it replaces anything.
//!
//! A separate process because tao's event loop and GPUI's cannot share the main thread. The
//! child's stdin is a pipe Peeby keeps open: when it closes (the page that opened the window is
//! gone, Peeby quit), the window closes too.
//!
//! The same way, `peeby --twitch-chat <login>` shows a channel's popout chat on twitch.tv, signed
//! in with the `auth-token` (read from stdin, never an argument), clicks its waiting bonus chest
//! after 3 s, as 7TV's auto-claim does, and closes (ADR 0012).

use std::io::{PipeReader, PipeWriter, Read as _, Write as _};
use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn as _;
use tao::platform::windows::{IconExtWindows as _, WindowBuilderExtWindows as _};
use tao::window::{Icon, Window, WindowBuilder};
use wry::cookie::{Cookie, Expiration};
use wry::{PageLoadEvent, WebContext, WebViewBuilder};

use ui::i18n::t;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
  YouTube,
  Twitch,
}

impl Service {
  fn arg(self) -> &'static str {
    match self {
      Self::YouTube => "youtube",
      Self::Twitch => "twitch",
    }
  }

  fn from_arg(arg: &str) -> Option<Self> {
    [Self::YouTube, Self::Twitch]
      .into_iter()
      .find(|s| s.arg() == arg)
  }

  fn start_url(self) -> &'static str {
    match self {
      Self::YouTube => {
        "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F"
      }
      Self::Twitch => "https://www.twitch.tv/login",
    }
  }

  fn title(self) -> &'static str {
    match self {
      Self::YouTube => t("Sign in to YouTube", "Connexion à YouTube"),
      Self::Twitch => t("Sign in to Twitch", "Connexion à Twitch"),
    }
  }

  /// The session, once the profile holds it: YouTube's youtube.com cookies as a Netscape file,
  /// Twitch's `auth-token`.
  fn session(self, cookies: &[Cookie]) -> Option<String> {
    match self {
      Self::YouTube => cookies
        .iter()
        .any(|c| c.name() == "SAPISID" && on(c, "youtube.com"))
        .then(|| netscape(cookies.iter().filter(|c| on(c, "youtube.com")))),
      Self::Twitch => cookies
        .iter()
        .find(|c| c.name() == "auth-token" && on(c, "twitch.tv") && !c.value().is_empty())
        .map(|c| c.value().to_owned()),
    }
  }
}

/// Exit codes of `peeby --login`.
const SIGNED_IN: i32 = 0;
const CLOSED: i32 = 2;
const UNAVAILABLE: i32 = 3;

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(520.0, 760.0);
/// How often the profile's cookies are read.
const POLL: Duration = Duration::from_secs(1);
/// The process is a console app in debug builds: it must not open a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The window's process, when the arguments are `--login <service>`: returns its exit code.
pub fn child_from_args() -> Option<i32> {
  let mut args = std::env::args().skip(1);
  if args.next().as_deref() != Some("--login") {
    return None;
  }
  let Some(service) = args.next().as_deref().and_then(Service::from_arg) else {
    tracing::error!("--login needs youtube or twitch");
    return Some(UNAVAILABLE);
  };
  Some(child(service))
}

fn child(service: Service) -> i32 {
  let profile =
    std::env::temp_dir().join(format!("peeby-sign-in-{}", uuid::Uuid::new_v4().simple()));
  let result = window(service, &profile);
  remove_profile(&profile);
  match result {
    Ok(Some(session)) => {
      let mut stdout = std::io::stdout();
      match stdout
        .write_all(session.as_bytes())
        .and_then(|()| stdout.flush())
      {
        Ok(()) => SIGNED_IN,
        Err(e) => {
          tracing::error!("cannot hand the session to Peeby: {e}");
          UNAVAILABLE
        }
      }
    }
    Ok(None) => CLOSED,
    Err(e) => {
      tracing::error!("sign-in window: {e}");
      UNAVAILABLE
    }
  }
}

/// Runs the window until the session shows up (`Some`) or the window closes (`None`).
fn window(service: Service, profile: &Path) -> Result<Option<String>, Box<dyn std::error::Error>> {
  let mut event_loop = EventLoopBuilder::<()>::with_user_event().build();
  let proxy = event_loop.create_proxy();
  std::thread::spawn(move || {
    // Returns when Peeby closes the pipe (or quits).
    let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
    let _ = proxy.send_event(());
  });

  let window = centered(service.title(), WINDOW_SIZE, &event_loop)?;
  let mut context = WebContext::new(Some(profile.to_path_buf()));
  let webview = WebViewBuilder::new_with_web_context(&mut context)
    .with_url(service.start_url())
    .with_incognito(true)
    .with_on_page_load_handler(|event, url| {
      if matches!(event, PageLoadEvent::Finished) {
        // Sign-in URLs carry state in their query: the host and path only.
        let page = url.split(['?', '#']).next().unwrap_or_default();
        tracing::debug!("sign-in page {page}");
      }
    })
    .build(&window)?;

  let mut session = None;
  event_loop.run_return(|event, _, control_flow| match event {
    Event::NewEvents(StartCause::Init | StartCause::ResumeTimeReached { .. }) => {
      *control_flow = ControlFlow::WaitUntil(Instant::now() + POLL);
      if let Ok(cookies) = webview.cookies()
        && let Some(found) = service.session(&cookies)
      {
        session = Some(found);
        *control_flow = ControlFlow::Exit;
      }
    }
    Event::UserEvent(())
    | Event::WindowEvent {
      event: WindowEvent::CloseRequested,
      ..
    } => *control_flow = ControlFlow::Exit,
    _ => {}
  });
  drop(webview);
  drop(window);
  Ok(session)
}

/// A window centered on the screen under the cursor: the one where its button was clicked.
fn centered<T>(
  title: &str,
  size: LogicalSize<f64>,
  event_loop: &EventLoop<T>,
) -> Result<Window, tao::error::OsError> {
  let mut builder = WindowBuilder::new()
    .with_title(title)
    .with_inner_size(size)
    .with_window_icon(Icon::from_resource(1, None).ok());
  let monitor = event_loop
    .cursor_position()
    .ok()
    .and_then(|p| event_loop.monitor_from_point(p.x, p.y))
    .or_else(|| event_loop.primary_monitor());
  if let Some(monitor) = monitor {
    let window: PhysicalSize<f64> = size.to_physical(monitor.scale_factor());
    let (origin, area) = (monitor.position(), monitor.size());
    builder = builder.with_position(PhysicalPosition::new(
      f64::from(origin.x) + (f64::from(area.width) - window.width).max(0.) / 2.,
      f64::from(origin.y) + (f64::from(area.height) - window.height).max(0.) / 2.,
    ));
  }
  builder.build(event_loop)
}

/// Deletes a WebView2 profile folder; its processes may hold it for a moment after the window
/// closes.
fn remove_profile(profile: &Path) {
  for _ in 0..10 {
    if !profile.exists() || std::fs::remove_dir_all(profile).is_ok() {
      break;
    }
    std::thread::sleep(Duration::from_millis(300));
  }
}

/// Whether `cookie` belongs to `site` or one of its subdomains (`cookie` strips the domain's
/// leading dot).
fn on(cookie: &Cookie, site: &str) -> bool {
  cookie
    .domain()
    .is_some_and(|d| d == site || d.strip_suffix(site).is_some_and(|d| d.ends_with('.')))
}

/// A Netscape `cookies.txt`. Every cookie is written as a domain cookie (`.site`, subdomains
/// included): `cookie` does not keep whether WebView2 gave the domain with its leading dot.
fn netscape<'a>(cookies: impl Iterator<Item = &'a Cookie<'a>>) -> String {
  let mut out = String::from("# Netscape HTTP Cookie File\n");
  for c in cookies {
    let expires = match c.expires() {
      Some(Expiration::DateTime(t)) => t.unix_timestamp().max(0),
      _ => 0,
    };
    out.push_str(&format!(
      "{}.{}\tTRUE\t{}\t{}\t{expires}\t{}\t{}\n",
      if c.http_only().unwrap_or(false) {
        "#HttpOnly_"
      } else {
        ""
      },
      c.domain().unwrap_or_default(),
      c.path().unwrap_or("/"),
      if c.secure().unwrap_or(false) {
        "TRUE"
      } else {
        "FALSE"
      },
      c.name(),
      c.value()
    ));
  }
  out
}

/// What the sign-in window gave.
pub enum Outcome {
  /// The session (see `Service::session`): a secret.
  SignedIn(String),
  /// The user closed the window.
  Closed,
  /// The window could not open or failed (WebView2 missing…): the manual way remains.
  Failed,
}

/// The open sign-in window: dropping it closes the window.
pub struct Pending {
  _stdin: PipeWriter,
}

/// Prepares the window: the `Pending` to keep while it is open, and the call that starts it and
/// blocks until it closes (to run in the background).
pub fn open(service: Service) -> std::io::Result<(Pending, impl FnOnce() -> Outcome + Send)> {
  let (reader, writer) = std::io::pipe()?;
  Ok((Pending { _stdin: writer }, move || run(service, reader)))
}

fn run(service: Service, stdin: PipeReader) -> Outcome {
  let child = std::env::current_exe().and_then(|exe| {
    Command::new(exe)
      .args(["--login", service.arg()])
      .stdin(stdin)
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .creation_flags(CREATE_NO_WINDOW)
      .spawn()
  });
  let mut child = match child {
    Ok(child) => child,
    Err(e) => {
      tracing::warn!("cannot start the sign-in window: {e}");
      return Outcome::Failed;
    }
  };
  let mut session = String::new();
  if let Some(mut stdout) = child.stdout.take()
    && let Err(e) = stdout.read_to_string(&mut session)
  {
    tracing::warn!("cannot read the sign-in window's answer: {e}");
  }
  match child.wait().map(|status| status.code()) {
    Ok(Some(SIGNED_IN)) if !session.is_empty() => Outcome::SignedIn(session),
    Ok(Some(CLOSED)) => Outcome::Closed,
    Ok(code) => {
      tracing::warn!("the {service:?} sign-in window failed (exit code {code:?})");
      Outcome::Failed
    }
    Err(e) => {
      tracing::warn!("sign-in window: {e}");
      Outcome::Failed
    }
  }
}

// The Twitch chat window.

const CHAT_SIZE: LogicalSize<f64> = LogicalSize::new(340.0, 520.0);
/// When the bonus is first looked for: long enough for twitch.tv's chat to load and show it.
const CLAIM_AFTER: Duration = Duration::from_secs(3);
/// How often it is looked for again, when it was not there yet.
const CLAIM_RETRY: Duration = Duration::from_millis(500);
/// When the window gives up without having found it, from its start.
const CLAIM_GIVE_UP: Duration = Duration::from_secs(12);
/// How long the window stays after the click, for the claim's request to leave.
const CLOSE_AFTER_CLAIM: Duration = Duration::from_millis(1500);
/// Clicks the chat's "Claim Bonus" button, as 7TV's auto-claim does in the browser; says whether
/// there was one. Found by its icon's class, then by its label (English and French pages).
const CLAIM_SCRIPT: &str = r#"(() => {
  const icon = document.querySelector('.claimable-bonus__icon');
  const button = (icon && icon.closest('button'))
    || document.querySelector('button[aria-label="Claim Bonus"], button[aria-label="Récupérer le bonus"]');
  if (!button) return false;
  button.click();
  return true;
})()"#;
/// Shows the window, to see what twitch.tv's page does (the window is hidden otherwise).
const SHOW_ENV: &str = "PEEBY_CLAIM_VISIBLE";

/// Exit code of `peeby --twitch-chat` when the page had no bonus button in time (0: clicked).
const NO_BONUS: i32 = 4;

/// What the chat window's event loop hears.
enum ChatEvent {
  /// Peeby closed the pipe, or quit.
  Closed,
  /// The claim script ran: whether it clicked.
  Clicked(bool),
}

/// The chat window's process, when the arguments are `--twitch-chat <login>`: returns its exit
/// code. The auth-token is stdin's first line; the rest of stdin is the pipe whose end closes the
/// window.
pub fn chat_from_args() -> Option<i32> {
  let mut args = std::env::args().skip(1);
  if args.next().as_deref() != Some("--twitch-chat") {
    return None;
  }
  let Some(login) = args.next().filter(|l| twitch_login(l)) else {
    tracing::error!("--twitch-chat needs a channel's login");
    return Some(UNAVAILABLE);
  };
  let mut token = String::new();
  if std::io::stdin().read_line(&mut token).is_err() || token.trim().is_empty() {
    tracing::error!("--twitch-chat needs the auth-token on stdin");
    return Some(UNAVAILABLE);
  }
  let profile = std::env::temp_dir().join(format!(
    "peeby-twitch-chat-{}",
    uuid::Uuid::new_v4().simple()
  ));
  let result = chat_window(&login, token.trim(), &profile);
  remove_profile(&profile);
  Some(match result {
    Ok(true) => 0,
    Ok(false) => NO_BONUS,
    Err(e) => {
      tracing::error!("Twitch chat window: {e}");
      UNAVAILABLE
    }
  })
}

/// A Twitch login: what goes in the chat's URL.
fn twitch_login(login: &str) -> bool {
  !login.is_empty()
    && login.len() <= 25
    && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Runs the chat window until its bonus is clicked (`true`) or it gives up (`false`).
fn chat_window(
  login: &str,
  token: &str,
  profile: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
  let mut event_loop = EventLoopBuilder::<ChatEvent>::with_user_event().build();
  let proxy = event_loop.create_proxy();
  std::thread::spawn({
    let proxy = proxy.clone();
    move || {
      // Returns when Peeby closes the pipe (or quits).
      let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
      let _ = proxy.send_event(ChatEvent::Closed);
    }
  });

  // Hidden, out of the taskbar and never focused: the viewer does not see it work.
  let shown = std::env::var_os(SHOW_ENV).is_some();
  let window = WindowBuilder::new()
    .with_title(format!("{login} · {}", t("Twitch chat", "Chat Twitch")))
    .with_inner_size(CHAT_SIZE)
    .with_window_icon(Icon::from_resource(1, None).ok())
    .with_visible(shown)
    .with_focused(false)
    .with_skip_taskbar(!shown)
    .build(&event_loop)?;
  let mut context = WebContext::new(Some(profile.to_path_buf()));
  // Blank first: the cookie is set before twitch.tv's page loads.
  let webview = WebViewBuilder::new_with_web_context(&mut context)
    .with_url("about:blank")
    .with_incognito(true)
    .build(&window)?;
  webview.set_cookie(
    // The page's own host: `cookie` drops a domain's leading dot, and WebView2 makes a cookie
    // with the bare `twitch.tv` host-only, which www.twitch.tv never gets.
    &Cookie::build(("auth-token", token))
      .domain("www.twitch.tv")
      .path("/")
      .secure(true)
      .build(),
  )?;
  webview.load_url(&format!("https://www.twitch.tv/popout/{login}/chat"))?;

  let started = Instant::now();
  let mut clicked = false;
  event_loop.run_return(|event, _, control_flow| match event {
    Event::NewEvents(StartCause::Init) => {
      *control_flow = ControlFlow::WaitUntil(started + CLAIM_AFTER);
    }
    // Clicked, and the request had its time: done.
    Event::NewEvents(StartCause::ResumeTimeReached { .. }) if clicked => {
      *control_flow = ControlFlow::Exit;
    }
    Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
      if started.elapsed() >= CLAIM_GIVE_UP {
        tracing::debug!("no Twitch bonus button in the chat");
        *control_flow = ControlFlow::Exit;
        return;
      }
      let proxy = proxy.clone();
      let ran = webview.evaluate_script_with_callback(CLAIM_SCRIPT, move |found| {
        tracing::debug!("claim script answered {found}");
        let _ = proxy.send_event(ChatEvent::Clicked(found.trim() == "true"));
      });
      if let Err(e) = ran {
        tracing::warn!("cannot claim the Twitch bonus: {e}");
        *control_flow = ControlFlow::Exit;
        return;
      }
      // Its answer sets the next wake; this one keeps looking if it never comes.
      *control_flow = ControlFlow::WaitUntil(Instant::now() + CLAIM_RETRY);
    }
    Event::UserEvent(ChatEvent::Clicked(true)) => {
      tracing::debug!("Twitch bonus clicked");
      clicked = true;
      *control_flow = ControlFlow::WaitUntil(Instant::now() + CLOSE_AFTER_CLAIM);
    }
    Event::UserEvent(ChatEvent::Closed)
    | Event::WindowEvent {
      event: WindowEvent::CloseRequested,
      ..
    } => *control_flow = ControlFlow::Exit,
    _ => {}
  });
  drop(webview);
  drop(window);
  Ok(clicked)
}

/// How a bonus claim through the chat window went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Claim {
  /// The bonus button was clicked.
  Clicked,
  /// The page showed no bonus button in time (not signed in, already claimed, Twitch changed it).
  NoButton,
  /// The window could not open or failed (WebView2 missing…).
  Failed,
}

/// Opens `login`'s chat in its hidden window, signed in with `token`, to click its bonus. Blocks
/// until the window closes: run it in the background. The token goes through the child's stdin,
/// never its arguments.
pub fn twitch_chat(login: &str, token: &str) -> Claim {
  let child = std::env::current_exe().and_then(|exe| {
    Command::new(exe)
      .args(["--twitch-chat", login])
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .creation_flags(CREATE_NO_WINDOW)
      .spawn()
  });
  let mut child = match child {
    Ok(child) => child,
    Err(e) => {
      tracing::warn!("cannot start the Twitch chat window: {e}");
      return Claim::Failed;
    }
  };
  // Kept until the child ends: closing it closes the window.
  let mut stdin = child.stdin.take();
  if let Some(pipe) = stdin.as_mut()
    && let Err(e) = writeln!(pipe, "{token}").and_then(|()| pipe.flush())
  {
    tracing::warn!("cannot hand the auth-token to the Twitch chat window: {e}");
  }
  let claim = match child.wait().map(|status| status.code()) {
    Ok(Some(0)) => Claim::Clicked,
    Ok(Some(NO_BONUS)) => Claim::NoButton,
    Ok(code) => {
      tracing::warn!("the Twitch chat window failed (exit code {code:?})");
      Claim::Failed
    }
    Err(e) => {
      tracing::warn!("Twitch chat window: {e}");
      Claim::Failed
    }
  };
  drop(stdin);
  claim
}

#[cfg(test)]
mod tests {
  use super::*;
  use wry::cookie::time::OffsetDateTime;

  fn youtube_profile() -> Vec<Cookie<'static>> {
    let expires = OffsetDateTime::from_unix_timestamp(1_900_000_000).unwrap();
    vec![
      Cookie::build(("SAPISID", "abc"))
        .domain(".youtube.com")
        .path("/")
        .secure(true)
        .expires(expires)
        .build(),
      Cookie::build(("YSC", "ysc"))
        .domain("youtube.com")
        .path("/")
        .secure(true)
        .http_only(true)
        .expires(Expiration::Session)
        .build(),
      Cookie::build(("NID", "nid")).domain(".google.com").build(),
      Cookie::build(("X", "x")).domain("notyoutube.com").build(),
    ]
  }

  #[test]
  fn youtube_session_is_a_jar_innertube_reads() {
    let jar = Service::YouTube.session(&youtube_profile()).unwrap();
    assert!(jar.contains(".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc\n"));
    assert!(jar.contains("#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tYSC\tysc\n"));
    assert!(!jar.contains("NID") && !jar.contains("notyoutube"));
    let cookies = innertube::Cookies::from_netscape(&jar).unwrap();
    assert!(cookies.to_netscape().contains("\tYSC\tysc"));
  }

  #[test]
  fn youtube_waits_for_sapisid() {
    let mut profile = youtube_profile();
    profile.retain(|c| c.name() != "SAPISID");
    assert_eq!(Service::YouTube.session(&profile), None);
  }

  #[test]
  fn twitch_session_is_the_auth_token() {
    let mut profile = vec![
      Cookie::build(("auth-token", ""))
        .domain("twitch.tv")
        .build(),
    ];
    assert_eq!(Service::Twitch.session(&profile), None);
    profile.push(
      Cookie::build(("auth-token", "abcdefghijklmnopqrstuvwxyz0123"))
        .domain(".twitch.tv")
        .build(),
    );
    assert_eq!(
      Service::Twitch.session(&profile).as_deref(),
      Some("abcdefghijklmnopqrstuvwxyz0123")
    );
  }

  #[test]
  fn twitch_chat_takes_logins_only() {
    assert!(twitch_login("some_streamer42"));
    assert!(!twitch_login(""));
    assert!(!twitch_login("a/../b"));
    assert!(!twitch_login("x?y=1"));
  }

  #[test]
  fn login_argument_round_trips() {
    for service in [Service::YouTube, Service::Twitch] {
      assert_eq!(Service::from_arg(service.arg()), Some(service));
    }
    assert_eq!(Service::from_arg("jellyfin"), None);
  }
}
