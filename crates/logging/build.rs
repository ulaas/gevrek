//! `GEVREK_DEBUG_LOG` at build time (`1`/`on` or `0`/`off`) decides what
//! the programs do when their setting says `auto`; a change of it builds
//! this crate again.

fn main() {
    println!("cargo:rerun-if-env-changed=GEVREK_DEBUG_LOG");
}
