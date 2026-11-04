// Puts the icon and the product name into Stickfell.exe on Windows
// (Explorer, the taskbar pin and Task Manager read them from there).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/stickfell.ico");
    println!("cargo:rerun-if-env-changed=STICKFELL_BUILD");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/stickfell.ico")
        .set("ProductName", "Stickfell: Last Limb Standing")
        .set("FileDescription", "Stickfell: Last Limb Standing")
        .set("OriginalFilename", "Stickfell.exe");
    res.compile().expect("couldn't embed the icon (needs the Windows SDK's rc.exe)");
}
