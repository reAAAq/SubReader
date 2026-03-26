//! Build script for ffi_c: auto-generates `reader_core.h` via cbindgen.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent() // crates/
        .unwrap()
        .parent() // project root
        .unwrap()
        .join("target");

    // Ensure the target directory exists
    std::fs::create_dir_all(&out_dir).expect("Failed to create target directory");

    let config = cbindgen::Config::from_file(PathBuf::from(&crate_dir).join("cbindgen.toml"))
        .expect("Failed to read cbindgen.toml");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("Failed to generate C bindings")
        .write_to_file(out_dir.join("reader_core.h"));

    // Re-run if lib.rs or cbindgen.toml changes
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
}
