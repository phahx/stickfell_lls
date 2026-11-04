//! Installing Stickfell properly, per user (no admin prompt): the launcher copies itself into the install folder
//! (default %LOCALAPPDATA%\Programs\Stickfell, changeable), the game goes in Game\ inside it, the Start menu (and,
//! if wanted, the desktop) get shortcuts, and Windows' Apps list gets an entry whose Uninstall runs
//! `Stickfell.exe --uninstall`. The tester key and other data stay in %LOCALAPPDATA%\Stickfell, apart from the
//! install. A copy started from anywhere else (Downloads, say) just opens the installed one.

use std::path::{Path, PathBuf};

/// The launcher's file name in the install folder.
pub const EXE: &str = "Stickfell.exe";
/// The game's folder inside the install folder.
pub const GAME: &str = "Game";
const NAME: &str = "Stickfell";
const DISPLAY_NAME: &str = "Stickfell: Last Limb Standing";
const PUBLISHER: &str = "khovanet";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Stickfell";

/// Where Stickfell installs unless the player picks somewhere else.
pub fn default_root() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Programs").join(NAME)
}

/// A folder the player picked: used as it is if it's already called Stickfell, otherwise Stickfell goes inside it.
pub fn root_in(picked: &Path) -> PathBuf {
    if picked.file_name().is_some_and(|n| n.eq_ignore_ascii_case(NAME)) { picked.to_path_buf() } else { picked.join(NAME) }
}

/// Asks the player for a folder (the system's own picker). None if they cancel.
pub fn choose_folder(start: &Path) -> Option<PathBuf> {
    let from = start.parent().filter(|p| p.exists()).unwrap_or(start);
    rfd::FileDialog::new().set_title("Choose where to install Stickfell").set_directory(from).pick_folder().map(|p| root_in(&p))
}

/// Whether two paths are the same folder (Windows paths ignore case).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    norm(a) == norm(b)
}

/// An old-style install beside an exe: a `game` folder holding the game (moved into the real install).
pub fn legacy_game(dir: &Path) -> Option<PathBuf> {
    let g = dir.join("game");
    g.join("stickfell.exe").is_file().then_some(g)
}

/// The install folder Windows has on record (from the Apps entry), if Stickfell is installed.
#[cfg(windows)]
pub fn registered_root() -> Option<PathBuf> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?;
    let at: String = key.get_value("InstallLocation").ok()?;
    let root = PathBuf::from(at);
    root.join(EXE).is_file().then_some(root)
}

#[cfg(not(windows))]
pub fn registered_root() -> Option<PathBuf> {
    None
}

fn err(what: &str) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{what} ({e}).")
}

/// Installs the running launcher into `root`: copies itself, moves an old game folder in if there is one, makes the
/// shortcuts and the Apps entry. Returns the installed launcher, to start in place of this one.
pub fn install(root: &Path, desktop: bool, legacy: Option<&Path>) -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(err("Can't tell where this launcher is"))?;
    std::fs::create_dir_all(root).map_err(err("Couldn't make the install folder"))?;
    let exe = root.join(EXE);
    if !same_dir(&me, &exe) {
        std::fs::copy(&me, &exe).map_err(err("Couldn't copy the launcher into the install folder"))?;
    }
    if let Some(old) = legacy {
        let game = root.join(GAME);
        if !game.exists() {
            // a rename on the same drive, else a copy (then the old folder goes)
            if std::fs::rename(old, &game).is_err() {
                copy_dir(old, &game).map_err(err("Couldn't move the installed game"))?;
                let _ = std::fs::remove_dir_all(old);
            }
        }
    }
    shortcuts(root, desktop)?;
    register(root);
    Ok(exe)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The Start menu's Programs folder and the desktop (wherever this account's are, e.g. moved into OneDrive).
#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: the API fills `p` with a string it allocates; it's read up to its NUL and then freed with
    // CoTaskMemFree, as the API requires (also when it fails).
    unsafe {
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p);
        let out = (hr >= 0 && !p.is_null()).then(|| {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(p, len)))
        });
        CoTaskMemFree(p as *const _);
        out
    }
}

#[cfg(windows)]
fn shortcut_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs};
    let lnk = format!("{NAME}.lnk");
    (known_folder(&FOLDERID_Programs).map(|p| p.join(&lnk)), known_folder(&FOLDERID_Desktop).map(|p| p.join(&lnk)))
}

#[cfg(not(windows))]
fn shortcut_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    (None, None)
}

/// The Start menu shortcut, and the desktop one if wanted.
#[cfg(windows)]
fn shortcuts(root: &Path, desktop: bool) -> Result<(), String> {
    let exe = root.join(EXE);
    let make = |at: &Path| -> Result<(), String> {
        let mut link = mslnk::ShellLink::new(&exe).map_err(|e| format!("Couldn't make a shortcut ({e})."))?;
        link.set_working_dir(Some(root.to_string_lossy().into_owned()));
        link.set_icon_location(Some(exe.to_string_lossy().into_owned()));
        link.set_name(Some(DISPLAY_NAME.into()));
        link.create_lnk(at).map_err(|e| format!("Couldn't make a shortcut ({e})."))
    };
    let (menu, desk) = shortcut_paths();
    make(&menu.ok_or("Couldn't find the Start menu.")?)?;
    if let (true, Some(d)) = (desktop, desk) {
        make(&d)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn shortcuts(_root: &Path, _desktop: bool) -> Result<(), String> {
    Ok(())
}

/// Adds (or refreshes) the entry in Windows' Apps list: name, version, publisher, size, and how to uninstall.
#[cfg(windows)]
pub fn register(root: &Path) {
    let Ok((key, _)) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY) else { return };
    let exe = root.join(EXE);
    let version = format!("{}.{}", env!("CARGO_PKG_VERSION"), crate::update::launcher_build());
    let size_kb = (folder_size(root) / 1024) as u32;
    let _ = key.set_value("DisplayName", &DISPLAY_NAME);
    let _ = key.set_value("DisplayIcon", &format!("{},0", exe.display()));
    let _ = key.set_value("DisplayVersion", &version);
    let _ = key.set_value("Publisher", &PUBLISHER);
    let _ = key.set_value("InstallLocation", &root.display().to_string());
    let _ = key.set_value("UninstallString", &format!("\"{}\" --uninstall", exe.display()));
    let _ = key.set_value("EstimatedSize", &size_kb);
    let _ = key.set_value("NoModify", &1u32);
    let _ = key.set_value("NoRepair", &1u32);
}

#[cfg(not(windows))]
pub fn register(_root: &Path) {}

fn folder_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries.flatten().map(|e| if e.path().is_dir() { folder_size(&e.path()) } else { e.metadata().map_or(0, |m| m.len()) }).sum()
    })
}

/// Removes Stickfell: the game, the shortcuts and the Apps entry now; the launcher (and the install folder, if
/// nothing else is in it) just after this launcher closes. The tester key stays, for a reinstall.
pub fn uninstall(root: &Path) -> Result<(), String> {
    let game = root.join(GAME);
    if game.exists() {
        std::fs::remove_dir_all(&game).map_err(|e| format!("Couldn't remove the game. Is it still running? ({e})"))?;
    }
    let (menu, desk) = shortcut_paths();
    for lnk in [menu, desk].into_iter().flatten() {
        let _ = std::fs::remove_file(lnk);
    }
    #[cfg(windows)]
    let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
    for leftover in ["Stickfell.old.exe", "Stickfell.update.exe"] {
        let _ = std::fs::remove_file(root.join(leftover));
    }
    remove_after_exit(root);
    Ok(())
}

/// A running exe can't delete itself: a hidden command keeps trying (for about a minute) until the launcher has
/// closed. The whole folder only if nothing but the launcher is left in it (someone may have installed into a
/// folder of their own).
#[cfg(windows)]
fn remove_after_exit(root: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let only_launcher = std::fs::read_dir(root).is_ok_and(|e| e.flatten().all(|e| e.file_name().eq_ignore_ascii_case(EXE)));
    let (remove, gone) = if only_launcher {
        (format!("rmdir /s /q \"{}\" 2>nul", root.display()), root.display().to_string())
    } else {
        let exe = root.join(EXE);
        (format!("del /f /q \"{}\" 2>nul", exe.display()), exe.display().to_string())
    };
    // (/s: cmd takes everything between the outer quotes as it is, inner quotes included)
    let line = format!("/s /c \"for /l %i in (1,1,40) do ( ping -n 2 127.0.0.1 >nul & {remove} & if not exist \"{gone}\" exit )\"");
    let _ = std::process::Command::new("cmd").raw_arg(line).creation_flags(CREATE_NO_WINDOW).spawn();
}

#[cfg(not(windows))]
fn remove_after_exit(_root: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_folder_gets_stickfell_inside_unless_it_is_one() {
        assert_eq!(root_in(Path::new(r"D:\Games")), PathBuf::from(r"D:\Games\Stickfell"));
        assert_eq!(root_in(Path::new(r"D:\Games\stickfell")), PathBuf::from(r"D:\Games\stickfell"));
    }

    #[test]
    fn the_default_is_per_user_programs() {
        assert!(default_root().ends_with(r"Programs\Stickfell"));
    }
}

#[cfg(all(test, windows))]
mod round_trip {
    use super::*;

    /// Really installs (into STICKFELL_TEST_ROOT, no desktop shortcut) and uninstalls, checking each step:
    /// `STICKFELL_TEST_ROOT=<dir> cargo test -p stickfell_launcher -- --ignored install_and_uninstall --nocapture`
    #[test]
    #[ignore]
    fn install_and_uninstall() {
        let base = PathBuf::from(std::env::var("STICKFELL_TEST_ROOT").expect("STICKFELL_TEST_ROOT"));
        let legacy = base.join("Downloads").join("game");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("stickfell.exe"), b"MZ old game").unwrap();
        std::fs::write(legacy.join("build.txt"), b"2").unwrap();
        let root = base.join("Programs").join(NAME);

        let exe = install(&root, false, legacy_game(&base.join("Downloads")).as_deref()).unwrap();
        assert!(exe.is_file(), "the launcher is copied in");
        assert_eq!(std::fs::read(root.join(GAME).join("stickfell.exe")).unwrap(), b"MZ old game", "the old game is moved in");
        assert!(!legacy.exists(), "and isn't left behind");
        let (menu, desk) = shortcut_paths();
        let menu = menu.unwrap();
        assert!(menu.is_file(), "Start menu shortcut at {}", menu.display());
        assert!(desk.is_none_or(|d| !d.exists()), "no desktop shortcut when unticked");
        assert!(same_dir(&registered_root().expect("an Apps entry"), &root));
        eprintln!("installed: {} ; shortcut {}", exe.display(), menu.display());

        uninstall(&root).unwrap();
        assert!(!root.join(GAME).exists() && !menu.exists() && registered_root().is_none(), "game, shortcut and Apps entry go at once");
        for _ in 0..20 {
            if !root.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        assert!(!root.exists(), "the launcher and its folder go a moment later");
        eprintln!("uninstalled cleanly");
    }
}
