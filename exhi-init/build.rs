use std::path::PathBuf;

fn main() {
    let linker_script =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("linker.ld");
    println!("cargo:rustc-link-arg-bin=exhi-init=-T{}", linker_script.display());
    println!("cargo:rerun-if-changed=linker.ld");
}