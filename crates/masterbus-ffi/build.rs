//! Generate `include/masterbus.h` from the C ABI via cbindgen.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out = PathBuf::from(&crate_dir)
        .join("include")
        .join("masterbus.h");

    let config = cbindgen::Config::from_root_or_default(&crate_dir);
    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(bindings) => {
            std::fs::create_dir_all(out.parent().unwrap()).ok();
            bindings.write_to_file(&out);
        }
        // Don't fail the build (e.g. on minimal cross environments); the header
        // is committed to the repo and regenerated on a normal host build.
        Err(e) => println!("cargo:warning=cbindgen header generation skipped: {e}"),
    }
}
