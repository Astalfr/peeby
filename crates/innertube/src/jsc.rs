//! The `n` challenge of YouTube's stream URLs, solved by yt-dlp's EJS scripts in the bundled
//! Deno, which stays running: started at the first challenge, it keeps each player's solving
//! function, so a challenge costs a few milliseconds instead of a Deno start and a parse of the
//! player (about 150 ms). `n` differs in every `player` answer: results are never cached.
//!
//! The scripts (`yt.solver.lib.min.js`, `yt.solver.core.min.js`, from `github.com/yt-dlp/ejs`)
//! follow YouTube's player: they are pinned in scripts/fetch-tools.ps1 and updated with yt-dlp.

use std::collections::HashSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Error, Result};

/// Peeby's line protocol around EJS's `jsc`.
const SERVER: &str = include_str!("jsc_server.js");
/// The first challenge of a player parses it (about 0.4 s); then a few milliseconds.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);
/// `CREATE_NO_WINDOW`: no console window flashes when Deno starts.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A player JS, as the solver needs it: its path (the key of its solving function) and code.
#[derive(Clone)]
pub struct PlayerJs {
  /// `/s/player/<id>/…/base.js`.
  pub path: String,
  /// The whole script.
  pub code: Arc<str>,
}

/// The challenge solver: one Deno process, started when first needed, started again if it died.
pub struct Solver {
  deno: PathBuf,
  script: PathBuf,
  worker: Mutex<Option<Worker>>,
}

struct Worker {
  child: Child,
  stdin: ChildStdin,
  lines: Receiver<String>,
  /// Players whose code this process has received.
  players: HashSet<String>,
}

impl Drop for Worker {
  fn drop(&mut self) {
    // Deno also ends when its stdin closes; killing it does not wait for that.
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

impl Solver {
  /// A solver running `deno` on the EJS scripts of `ejs_dir`, its script written to `work_dir`.
  /// Blocking (reads and writes files): call it off the UI thread. Fails when the scripts are
  /// missing.
  pub fn new(deno: &Path, ejs_dir: &Path, work_dir: &Path) -> Result<Self> {
    let read = |name: &str| {
      std::fs::read_to_string(ejs_dir.join(name))
        .map_err(|e| Error::Solver(format!("{name}: {}", e.kind())))
    };
    let lib = read("yt.solver.lib.min.js")?;
    let core = read("yt.solver.core.min.js")?;
    // As yt-dlp's `_construct_stdin`: the lib's exports made global before the core runs.
    let script = format!("{lib}\nObject.assign(globalThis, lib);\n{core}\n{SERVER}");
    let path = work_dir.join("peeby-jsc.js");
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(script.as_str()) {
      std::fs::write(&path, script).map_err(|e| Error::Solver(format!("script: {}", e.kind())))?;
    }
    Ok(Self {
      deno: deno.to_owned(),
      script: path,
      worker: Mutex::new(None),
    })
  }

  /// `challenge` transformed by `player`'s `n` function.
  pub async fn solve_n(self: &Arc<Self>, player: &PlayerJs, challenge: &str) -> Result<String> {
    let solver = Arc::clone(self);
    let (player, challenge) = (player.clone(), challenge.to_owned());
    tokio::task::spawn_blocking(move || solver.solve_blocking(&player, &challenge))
      .await
      .map_err(|_| Error::Solver("the solver's thread ended".into()))?
  }

  /// Starts Deno and loads the scripts, ahead of the first challenge. Blocking.
  pub fn warm_up(&self) {
    let mut worker = crate::client::lock(&self.worker);
    if worker.is_none() {
      match self.start() {
        Ok(started) => *worker = Some(started),
        Err(e) => tracing::warn!("n solver: {e}"),
      }
    }
  }

  fn solve_blocking(&self, player: &PlayerJs, challenge: &str) -> Result<String> {
    let mut worker = crate::client::lock(&self.worker);
    // A dead or wedged process is dropped and started again, once.
    for attempt in 0..2 {
      if worker.is_none() {
        *worker = Some(self.start()?);
      }
      let Some(running) = worker.as_mut() else {
        continue;
      };
      match running.solve(player, challenge) {
        Ok(solved) => return Ok(solved),
        Err(Failure::Process(e)) if attempt == 0 => {
          tracing::warn!("n solver: {e}, starting it again");
          *worker = None;
        }
        Err(Failure::Process(e) | Failure::Answer(e)) => {
          if worker.as_mut().is_some_and(|w| !w.alive()) {
            *worker = None;
          }
          return Err(Error::Solver(e));
        }
      }
    }
    Err(Error::Solver("Deno does not answer".into()))
  }

  fn start(&self) -> Result<Worker> {
    let mut child = Command::new(&self.deno)
      .args([
        "run",
        "--quiet",
        "--no-prompt",
        "--no-remote",
        "--no-lock",
        "--node-modules-dir=none",
        "--no-config",
        "--no-npm",
        "--cached-only",
      ])
      .arg(&self.script)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .creation_flags(CREATE_NO_WINDOW)
      .spawn()
      .map_err(|e| Error::Solver(format!("cannot start Deno: {}", e.kind())))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
      let _ = child.kill();
      return Err(Error::Solver("Deno's pipes are missing".into()));
    };
    // A reader thread, so that an answer can be waited for with a timeout.
    let (send, lines) = channel();
    std::thread::Builder::new()
      .name("n-solver".into())
      .spawn(move || {
        for line in BufReader::new(stdout).lines() {
          let Ok(line) = line else { break };
          if send.send(line).is_err() {
            break;
          }
        }
      })
      .map_err(|e| Error::Solver(format!("reader thread: {}", e.kind())))?;
    Ok(Worker {
      child,
      stdin,
      lines,
      players: HashSet::new(),
    })
  }
}

enum Failure {
  /// The process is gone or does not answer: worth a new one.
  Process(String),
  /// It answered with an error: the scripts cannot solve this player.
  Answer(String),
}

impl Worker {
  fn alive(&mut self) -> bool {
    matches!(self.child.try_wait(), Ok(None))
  }

  fn solve(&mut self, player: &PlayerJs, challenge: &str) -> std::result::Result<String, Failure> {
    for _ in 0..2 {
      let code = (!self.players.contains(&player.path)).then(|| player.code.as_ref());
      let answer = self.ask(&json!({ "player": player.path, "code": code, "n": [challenge] }))?;
      if answer["need_player"] == true {
        self.players.remove(&player.path);
        continue;
      }
      self.players.insert(player.path.clone());
      if let Some(error) = answer["error"].as_str() {
        return Err(Failure::Answer(error.chars().take(200).collect()));
      }
      return match answer["ok"][challenge].as_str() {
        // The function returns its input when it fails on a newer player.
        Some(solved) if solved != challenge && !solved.is_empty() => Ok(solved.to_owned()),
        _ => Err(Failure::Answer("the challenge was not solved".into())),
      };
    }
    Err(Failure::Answer(
      "the solver kept asking for the player".into(),
    ))
  }

  fn ask(&mut self, request: &Value) -> std::result::Result<Value, Failure> {
    let mut line = request.to_string();
    line.push('\n');
    self
      .stdin
      .write_all(line.as_bytes())
      .and_then(|()| self.stdin.flush())
      .map_err(|e| Failure::Process(format!("cannot write to Deno: {}", e.kind())))?;
    let answer = match self.lines.recv_timeout(ANSWER_TIMEOUT) {
      Ok(answer) => answer,
      Err(RecvTimeoutError::Timeout) => return Err(Failure::Process("no answer in time".into())),
      Err(RecvTimeoutError::Disconnected) => return Err(Failure::Process("Deno ended".into())),
    };
    serde_json::from_str(&answer).map_err(|_| Failure::Process("unreadable answer".into()))
  }
}
