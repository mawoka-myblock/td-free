use anyhow::Result;
use std::env;

use vergen_gix::{BuildBuilder, Emitter, GixBuilder, RustcBuilder};
fn main() -> Result<()> {
    let build = BuildBuilder::all_build()?;
    let gitcl = GixBuilder::all_git()?;
    let rustc = RustcBuilder::all_rustc()?;
    let _version = match env::var("VERSION") {
        Ok(val) => val,
        Err(_) => "UNSET".to_string(),
    };
    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&gitcl)?
        .add_instructions(&rustc)?
        .emit()?;
    embuild::espidf::sysenv::output();
    Ok(())
}
