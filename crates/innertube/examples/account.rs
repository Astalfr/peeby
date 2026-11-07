//! Prints the signed-in account's channel handle, read from `account/account_menu`.
//!
//! `cargo run -p innertube --example account -- <cookies.txt> [--save <file>]`
//! (an exported copy of the jar, as for `home`). `--save` writes the whole response, which holds
//! the account's name, handle and picture: trim and rename them before it becomes a fixture.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use innertube::{Client, Cookies};

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let mut args = std::env::args().skip(1);
  let Some(path) = args.next() else {
    fail("usage: account <cookies.txt> [--save <file>]");
  };
  let save = match (args.next().as_deref(), args.next()) {
    (Some("--save"), Some(file)) => Some(file),
    (None, _) => None,
    _ => fail("usage: account <cookies.txt> [--save <file>]"),
  };
  let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
  let cookies = Cookies::from_netscape(&text).unwrap_or_else(|e| fail(&e.to_string()));
  // Rotated cookies are written back, as the app does.
  let client = Client::new(
    Some(cookies),
    Some(innertube::save_to_file(path.into())),
    "fr",
    "FR",
  )
  .unwrap_or_else(|e| fail(&e.to_string()));
  if let Err(e) = client.refresh_client_version().await {
    eprintln!("warning: {e}");
  }
  let response = client
    .account_menu()
    .await
    .unwrap_or_else(|e| fail(&e.to_string()));
  if let Some(file) = save {
    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::write(&file, json).unwrap_or_else(|e| fail(&format!("{file}: {e}")));
    println!("saved to {file}");
  }
  println!("handle: {:?}", innertube::parse_account_handle(&response));
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
