// Puts the icon, the product details and a manifest into Stickfell.exe on Windows (Explorer, the taskbar pin and Task
// Manager read them from there; complete version info and a manifest also make antivirus less suspicious of it).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/stickfell.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-env-changed=STICKFELL_BUILD");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let build: u64 = std::env::var("STICKFELL_BUILD").ok().and_then(|b| b.trim().parse().ok()).unwrap_or(0);
    let version = format!("{}.{build}", env!("CARGO_PKG_VERSION"));
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/stickfell.ico")
        .set("ProductName", "Stickfell: Last Limb Standing")
        .set("FileDescription", "Stickfell: Last Limb Standing")
        .set("CompanyName", "khovanet")
        .set("LegalCopyright", "Copyright © 2026 khovanet")
        .set("InternalName", "stickfell")
        .set("OriginalFilename", "Stickfell.exe")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed(build))
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed(build))
        .set_manifest_file("assets/app.manifest");
    res.compile().expect("couldn't embed the icon (needs the Windows SDK's rc.exe)");
}

/// major.minor.patch.build packed the way a Windows version resource wants it.
fn packed(build: u64) -> u64 {
    let part = |s: &str| s.parse::<u64>().unwrap_or(0) & 0xffff;
    let (ma, mi, pa) = (part(env!("CARGO_PKG_VERSION_MAJOR")), part(env!("CARGO_PKG_VERSION_MINOR")), part(env!("CARGO_PKG_VERSION_PATCH")));
    ma << 48 | mi << 32 | pa << 16 | (build & 0xffff)
}
