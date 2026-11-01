//! Embeds the Windows resources (app icon, see peebtube.rc) into peebtube.exe.

fn main() -> Result<(), Box<dyn std::error::Error>> {
  println!("cargo:rerun-if-changed=peebtube.rc");
  println!("cargo:rerun-if-changed=assets/peebtube.ico");
  // Required, not optional: PeebTube is Windows-only, so a missing resource compiler is an error.
  embed_resource::compile("peebtube.rc", embed_resource::NONE).manifest_required()?;
  Ok(())
}
