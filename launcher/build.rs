// Puts the icon and the product name into the launcher on Windows (Explorer, the taskbar and Task Manager read them
// from there). Players run it as Stickfell.exe.
fn main() {
    println!("cargo:rerun-if-changed=../game/assets/icon/stickfell.ico");
    println!("cargo:rerun-if-env-changed=STICKFELL_LAUNCHER_BUILD");
    println!("cargo:rerun-if-env-changed=STICKFELL_UPDATE_URL");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../game/assets/icon/stickfell.ico")
        .set("ProductName", "Stickfell: Last Limb Standing")
        .set("FileDescription", "Stickfell launcher")
        .set("OriginalFilename", "Stickfell.exe");
    res.compile().expect("couldn't embed the icon (needs the Windows SDK's rc.exe)");
}
