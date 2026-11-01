//! Embeds the Windows resources (app icon, see peeby.rc) into peeby.exe.

fn main() -> Result<(), Box<dyn std::error::Error>> {
  println!("cargo:rerun-if-changed=peeby.rc");
  println!("cargo:rerun-if-changed=assets/peeby.ico");
  // Required, not optional: Peeby is Windows-only, so a missing resource compiler is an error.
  embed_resource::compile("peeby.rc", embed_resource::NONE).manifest_required()?;
  Ok(())
}
