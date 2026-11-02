// PROTOTYPE: embed a comctl32 v6 + PerMonitorV2 manifest with the MSVC linker,
// no build-dependency crate needed.
fn main() {
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("popup.manifest");
    println!("cargo:rerun-if-changed=popup.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
}
