//! The tester key, kept in %LOCALAPPDATA%\Stickfell\tester-key.bin, encrypted for this Windows account (DPAPI), so
//! another account on the PC, or a copy of the file, can't read it.

use std::path::PathBuf;

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(PathBuf::from))?;
    Some(base.join("Stickfell").join("tester-key.bin"))
}

/// The saved tester key, if there is one (and it's this account's).
pub fn load() -> Option<String> {
    let bytes = std::fs::read(file()?).ok()?;
    String::from_utf8(unprotect(&bytes)?).ok().filter(|k| !k.trim().is_empty())
}

/// Saves the tester key (false if it couldn't be).
pub fn save(key: &str) -> bool {
    let (Some(path), Some(bytes)) = (file(), protect(key.trim().as_bytes())) else { return false };
    path.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok()) && std::fs::write(path, bytes).is_ok()
}

/// Forgets the tester key.
pub fn forget() {
    if let Some(path) = file() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
fn crypt(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: the input blob points at `data` for the call; on success the output is a LocalAlloc'd buffer of
    // cbData bytes, copied out and then freed exactly once.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Some(out)
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    crypt(data, true)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    crypt(data, false)
}

#[cfg(not(windows))]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn round_trips_and_isnt_stored_plain() {
        let key = "STICK-ABCDE-FGHJK-MNPQR-STVWX";
        let sealed = super::protect(key.as_bytes()).unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("ABCDE"));
        assert_eq!(super::unprotect(&sealed).unwrap(), key.as_bytes());
    }
}
