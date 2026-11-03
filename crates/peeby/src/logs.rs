//! Peeby's log: on stderr (a debug build's console) and in a file per launch,
//! `%LOCALAPPDATA%\peeby\logs\<profile>-<ms>.log`, kept as long as mpv's logs
//! ([`settings::prune_logs`]). A release build has no console: without the file its warnings
//! were lost. The file is created at the first line, so a launch that only hands its arguments
//! to the running Peeby leaves none; a thread writes it, off the UI thread. A panic is logged too.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

use crate::settings;

/// The default filter, unless `RUST_LOG` says otherwise. Stream URLs are logged at debug only:
/// a `RUST_LOG` at debug writes them to the file too.
const DEFAULT_FILTER: &str = "peeby=info,innertube=info,twitch=info";
/// How long a panic waits for the file to get its last lines.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

enum Message {
  Line(Vec<u8>),
  /// Answers once the lines sent before are written.
  Flush(mpsc::Sender<()>),
}

/// Sends each event, formatted, to the writing thread.
struct Sink(mpsc::Sender<Message>);

/// One event's text, sent when the formatter drops it.
struct Line<'a> {
  sink: &'a mpsc::Sender<Message>,
  text: Vec<u8>,
}

impl<'a> MakeWriter<'a> for Sink {
  type Writer = Line<'a>;

  fn make_writer(&'a self) -> Line<'a> {
    Line {
      sink: &self.0,
      text: Vec::new(),
    }
  }
}

impl Write for Line<'_> {
  fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    self.text.extend_from_slice(buf);
    Ok(buf.len())
  }

  fn flush(&mut self) -> io::Result<()> {
    Ok(())
  }
}

impl Drop for Line<'_> {
  fn drop(&mut self) {
    if !self.text.is_empty() {
      // The writing thread is gone only when the file could not be opened.
      self
        .sink
        .send(Message::Line(std::mem::take(&mut self.text)))
        .ok();
    }
  }
}

/// Starts the log, first thing in `main`.
pub fn init() {
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_FILTER.into());
  let file = new_log().map(|path| {
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || write(&path, &received));
    sender
  });
  let flush = file.clone();
  tracing_subscriber::registry()
    .with(filter)
    .with(fmt::layer().with_writer(io::stderr))
    .with(file.map(|sender| fmt::layer().with_ansi(false).with_writer(Sink(sender))))
    .init();
  let previous = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    tracing::error!("{info}");
    if let Some(sender) = &flush {
      let (done, written) = mpsc::channel();
      if sender.send(Message::Flush(done)).is_ok() {
        written.recv_timeout(FLUSH_WAIT).ok();
      }
    }
    previous(info);
  }));
}

/// This launch's file, named after the profile and the time it starts.
fn new_log() -> Option<PathBuf> {
  let millis = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .ok()?
    .as_millis();
  Some(settings::logs_dir()?.join(format!("{}-{millis}.log", settings::PROFILE)))
}

/// The writing thread: opens `path` at the first line, then appends each one.
fn write(path: &Path, received: &mpsc::Receiver<Message>) {
  let mut file: Option<File> = None;
  while let Ok(message) = received.recv() {
    match message {
      Message::Line(text) => {
        if file.is_none() {
          file = open(path);
          // No file: the lines go on stderr only.
          if file.is_none() {
            return;
          }
        }
        if let Some(file) = &mut file {
          file.write_all(&text).ok();
        }
      }
      Message::Flush(done) => {
        if let Some(file) = &mut file {
          file.flush().ok();
        }
        done.send(()).ok();
      }
    }
  }
}

fn open(path: &Path) -> Option<File> {
  std::fs::create_dir_all(path.parent()?).ok()?;
  OpenOptions::new().create(true).append(true).open(path).ok()
}
