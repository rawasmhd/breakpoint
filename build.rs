//! Embeds the exe icons, version info and application manifest.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets");

    let mut res = winresource::WindowsResource::new();
    // Icon 1 is the exe icon and the tray's normal state; icon 2 is the greyed (quiet) state.
    res.set_icon_with_id("assets/breakpoint.ico", "1");
    res.set_icon_with_id("assets/breakpoint-grey.ico", "2");
    res.set_manifest_file("assets/breakpoint.exe.manifest");
    res.compile().expect("failed to embed Windows resources");
}
