use anyhow::Result;
use vergen_gix::{BuildBuilder, Emitter, GixBuilder, RustcBuilder};
fn main() -> Result<()> {
    let build = BuildBuilder::all_build()?;
    let gitcl = GixBuilder::all_git()?;
    let rustc = RustcBuilder::all_rustc()?;
    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&gitcl)?
        .add_instructions(&rustc)?
        .emit()?;
    embuild::espidf::sysenv::output();
    println!(
        "cargo:rustc-env=TD_FREE_VERSION={}",
        std::env::var("VERSION").unwrap_or("UNKNOWN".to_string())
    );
    Ok(())
}
