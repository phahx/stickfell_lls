//! Updates from the Stickfell update service (update_server/), on two channels with their own signing keys: the
//! launcher updates itself first, then installs or updates the game. GitHub Actions signs each build's manifest
//! (build, commit, notes, SHA-256, size) with a key only that channel's release workflow holds; the launcher checks the
//! signature against the public key built into it (keys/), so neither the server nor anyone in between can hand it a
//! build of their own, then checks the download against the manifest's hash before it touches anything. Downloading
//! needs a tester key. (Adapted from khovdesk's desktop updater.)

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The update service (set when the release workflow builds the launcher); `STICKFELL_UPDATE_URL` overrides it.
const SERVICE: &str = match option_env!("STICKFELL_UPDATE_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:3000",
};
/// No build is bigger than this (the server refuses them too).
const MAX_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Launcher,
    Game,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Launcher => "launcher",
            Channel::Game => "game",
        }
    }
    fn app(self) -> &'static str {
        match self {
            Channel::Launcher => "stickfell-launcher",
            Channel::Game => "stickfell-game",
        }
    }
    /// The channel's public key: only builds signed with its private half install.
    fn key(self) -> &'static str {
        match self {
            Channel::Launcher => include_str!("../../keys/launcher-signing-key.pub"),
            Channel::Game => include_str!("../../keys/game-signing-key.pub"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    app: String,
    pub build: u32,
    pub commit: String,
    #[serde(default)]
    pub message: Option<String>,
    pub sha256: String,
    pub size: u64,
    /// When it was built (RFC 3339), for patch notes.
    #[serde(default)]
    pub built_at: Option<String>,
}

impl Manifest {
    /// The day it was built, written out ("4 November 2026"), if known.
    pub fn date(&self) -> Option<String> {
        const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        let d = self.built_at.as_deref()?.get(..10)?;
        let mut parts = d.split('-').map(|p| p.parse::<u32>().ok());
        let (y, m, day) = (parts.next()??, parts.next()??, parts.next()??);
        Some(format!("{day} {} {y}", MONTHS.get(m.checked_sub(1)? as usize)?))
    }
}

/// What went wrong, as the launcher says it.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// The service couldn't be reached (offline, or it's down).
    Offline(String),
    /// The tester key was refused (wrong, revoked, or none).
    Key(String),
    /// Anything else (a bad signature, a bad download, the disk).
    Other(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Problem::Offline(s) | Problem::Key(s) | Problem::Other(s) => f.write_str(s),
        }
    }
}

type Result<T> = std::result::Result<T, Problem>;

/// This launcher's build number from GitHub Actions (0: built on a PC, which never updates itself).
pub fn launcher_build() -> u32 {
    option_env!("STICKFELL_LAUNCHER_BUILD").and_then(|b| b.parse().ok()).unwrap_or(0)
}

/// Checks a manifest's signature (ECDSA P-256, SHA-256, DER) against the channel's built-in key, and that it's for
/// that channel and sane. Anything else is refused.
pub fn verify(manifest_b64: &str, signature_b64: &str, channel: Channel) -> Result<Manifest> {
    verify_with(manifest_b64, signature_b64, channel, channel.key())
}

fn verify_with(manifest_b64: &str, signature_b64: &str, channel: Channel, key_pem: &str) -> Result<Manifest> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let bad = || Problem::Other("The update's signature doesn't match. Nothing was installed.".into());
    let (bytes, sig) = (b64.decode(manifest_b64).map_err(|_| bad())?, b64.decode(signature_b64).map_err(|_| bad())?);
    let key = VerifyingKey::from_public_key_pem(key_pem).map_err(|_| Problem::Other("This launcher has no valid update key.".into()))?;
    key.verify(&bytes, &Signature::from_der(&sig).map_err(|_| bad())?).map_err(|_| bad())?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let hex = m.sha256.len() == 64 && m.sha256.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if m.app != channel.app() || m.build == 0 || !hex || m.size == 0 || m.size > MAX_BYTES {
        return Err(bad());
    }
    Ok(m)
}

/// The service's address: https only (plain http just for a test service on this PC).
fn base() -> Result<String> {
    let url = std::env::var("STICKFELL_UPDATE_URL").ok().filter(|u| !u.trim().is_empty()).unwrap_or(SERVICE.into());
    let url = url.trim().trim_end_matches('/').to_owned();
    let local = ["http://127.0.0.1", "http://localhost"].iter().any(|p| url.starts_with(p));
    if url.starts_with("https://") || local {
        Ok(url)
    } else {
        Err(Problem::Other(format!("The update service's address isn't secure ({url}).")))
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(900)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get(path: &str, key: Option<&str>) -> Result<ureq::http::Response<ureq::Body>> {
    let mut req = agent().get(format!("{}{path}", base()?));
    if let Some(key) = key {
        req = req.header("Authorization", format!("Bearer {}", key.trim()));
    }
    req.call().map_err(|e| Problem::Offline(format!("The update service couldn't be reached ({e}).")))
}

/// The service's refusal as a problem: its own message, as a key problem when it's about the key.
fn refusal(mut res: ureq::http::Response<ureq::Body>) -> Problem {
    let status = res.status().as_u16();
    let text = res.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|j| j["error"].as_str().map(str::to_owned));
    let message = message.unwrap_or_else(|| format!("The update service answered {status}."));
    if matches!(status, 401 | 403 | 429) { Problem::Key(message) } else { Problem::Other(message) }
}

/// Whether the service takes this tester key.
pub fn check_key(key: &str) -> Result<()> {
    let res = get("/v1/license", Some(key))?;
    if res.status().is_success() { Ok(()) } else { Err(refusal(res)) }
}

/// The channel's newest signed build (None: nothing published yet).
pub fn latest(channel: Channel) -> Result<Option<Manifest>> {
    let mut res = get(&format!("/v1/{}/latest", channel.name()), None)?;
    if !res.status().is_success() {
        return Err(refusal(res));
    }
    let text = res.body_mut().with_config().limit(256 * 1024).read_to_string().map_err(|e| Problem::Offline(e.to_string()))?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|_| Problem::Other("The update service sent something odd.".into()))?;
    if j["available"] != true {
        return Ok(None);
    }
    verify(j["manifest"].as_str().unwrap_or_default(), j["signature"].as_str().unwrap_or_default(), channel).map(Some)
}

/// Every build the service kept for the channel, newest first (patch notes): only those whose signature checks out.
pub fn history(channel: Channel) -> Result<Vec<Manifest>> {
    let mut res = get(&format!("/v1/{}/history", channel.name()), None)?;
    if !res.status().is_success() {
        return Err(refusal(res));
    }
    let text = res.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().map_err(|e| Problem::Offline(e.to_string()))?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|_| Problem::Other("The update service sent something odd.".into()))?;
    let builds = j["builds"].as_array().cloned().unwrap_or_default();
    Ok(builds.iter().filter_map(|b| verify(b["manifest"].as_str()?, b["signature"].as_str()?, channel).ok()).collect())
}

/// Downloads a channel's build (it must match `m`, already verified), reporting progress (bytes so far, total).
pub fn download(channel: Channel, key: &str, m: &Manifest, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>> {
    let mut res = get(&format!("/v1/{}/download", channel.name()), Some(key))?;
    if !res.status().is_success() {
        return Err(refusal(res));
    }
    let mut reader = res.body_mut().with_config().limit(m.size + 1).reader();
    let mut bytes = Vec::with_capacity(m.size as usize);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| Problem::Offline(format!("The download stopped ({e}).")))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() as u64 > m.size {
            break;
        }
        progress(bytes.len() as u64, m.size);
    }
    if !matches_manifest(&bytes, m) {
        return Err(Problem::Other("The download doesn't match its signed manifest. Nothing was installed.".into()));
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn matches_manifest(bytes: &[u8], m: &Manifest) -> bool {
    bytes.len() as u64 == m.size && hex(&Sha256::digest(bytes)) == m.sha256
}

/// Where the launcher is, and so where everything is installed.
pub fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|_| Problem::Other("Can't tell where Stickfell is installed.".into()))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| Problem::Other("Can't tell where Stickfell is installed.".into()))
}

/// The game's exe, in game/ beside the launcher.
pub fn game_exe(dir: &Path) -> PathBuf {
    dir.join("game").join("stickfell.exe")
}

/// Whether the installed game is exactly this build.
pub fn game_is(dir: &Path, m: &Manifest) -> bool {
    std::fs::read(game_exe(dir)).is_ok_and(|b| matches_manifest(&b, m))
}

/// The installed game's build, as the launcher noted it (None: not installed, or not by the launcher).
pub fn installed_build(dir: &Path) -> Option<u32> {
    game_exe(dir).exists().then(|| std::fs::read_to_string(dir.join("game").join("build.txt")).ok()?.trim().parse().ok()).flatten()
}

/// Keeps the installed build's manifest beside it (its patch notes, shown offline too).
pub fn save_notes(dir: &Path, m: &Manifest) {
    if let Ok(text) = serde_json::to_string(m) {
        let _ = std::fs::write(dir.join("game").join("notes.json"), text);
    }
}

/// The installed build's manifest, as [`save_notes`] kept it.
pub fn load_notes(dir: &Path) -> Option<Manifest> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("game").join("notes.json")).ok()?).ok()
}

/// Puts a verified game build in place (written beside it first, then swapped in, so a failed write never leaves a
/// broken game), and notes its build number.
pub fn install_game(dir: &Path, bytes: &[u8], build: u32) -> Result<()> {
    let disk = |e: std::io::Error| Problem::Other(format!("Couldn't install the game ({e})."));
    let exe = game_exe(dir);
    std::fs::create_dir_all(exe.parent().unwrap()).map_err(disk)?;
    let next = exe.with_extension("update.exe");
    std::fs::write(&next, bytes).map_err(disk)?;
    if let Err(e) = std::fs::rename(&next, &exe) {
        let _ = std::fs::remove_file(&next);
        return Err(Problem::Other(format!("Couldn't replace the game. Is it still running? ({e})")));
    }
    std::fs::write(dir.join("game").join("build.txt"), build.to_string()).map_err(disk)
}

/// Puts the new launcher in place of `exe` (Windows lets a running exe be renamed, not deleted): the old one moves
/// to .old.exe, and comes back if the new one can't be put in place.
pub fn swap(exe: &Path, bytes: &[u8]) -> Result<()> {
    let disk = |e: std::io::Error| Problem::Other(format!("Couldn't update the launcher ({e})."));
    let (next, old) = (exe.with_extension("update.exe"), exe.with_extension("old.exe"));
    std::fs::write(&next, bytes).map_err(disk)?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(disk)?;
    if let Err(e) = std::fs::rename(&next, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(disk(e));
    }
    Ok(())
}

/// The previous launcher, left beside the new one after it updated itself.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::{EncodePublicKey, LineEnding};

    fn signed(app: &str, build: u32, body: &[u8], key: &SigningKey) -> (String, String) {
        let b64 = base64::engine::general_purpose::STANDARD;
        let json = serde_json::json!({
            "app": app, "build": build, "commit": "abc1234", "message": "Faster knives",
            "sha256": hex(&Sha256::digest(body)), "size": body.len(), "builtAt": "2026-11-04T03:00:00Z",
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let sig: Signature = key.sign(&bytes);
        (b64.encode(&bytes), b64.encode(sig.to_der()))
    }

    #[test]
    fn only_manifests_signed_for_that_channel() {
        let game = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let launcher = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pem = |k: &SigningKey| k.verifying_key().to_public_key_pem(LineEnding::LF).unwrap();
        let (m, s) = signed("stickfell-game", 3, b"MZ game", &game);
        assert_eq!(verify_with(&m, &s, Channel::Game, &pem(&game)).unwrap().build, 3);
        // the launcher's key can't sign a game build, nor a game manifest pass as the launcher's
        let (m2, s2) = signed("stickfell-game", 4, b"MZ game", &launcher);
        assert!(verify_with(&m2, &s2, Channel::Game, &pem(&game)).is_err());
        assert!(verify_with(&m, &s, Channel::Launcher, &pem(&game)).is_err());
        // tampered manifests fail
        let b64 = base64::engine::general_purpose::STANDARD;
        let mut bytes = b64.decode(&m).unwrap();
        bytes[10] ^= 1;
        assert!(verify_with(&b64.encode(&bytes), &s, Channel::Game, &pem(&game)).is_err());
        // the real keys parse
        assert!(VerifyingKey::from_public_key_pem(Channel::Game.key()).is_ok());
        assert!(VerifyingKey::from_public_key_pem(Channel::Launcher.key()).is_ok());
        assert_ne!(Channel::Game.key(), Channel::Launcher.key());
    }

    #[test]
    fn swaps_the_exe_and_installs_the_game() {
        let dir = std::env::temp_dir().join(format!("stickfell-launcher-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("Stickfell.exe");
        std::fs::write(&exe, "old").unwrap();
        swap(&exe, b"new").unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        assert_eq!((read(&exe), read(&dir.join("Stickfell.old.exe"))), ("new".into(), "old".into()));
        install_game(&dir, b"MZ game", 12).unwrap();
        assert_eq!(read(&game_exe(&dir)), "MZ game");
        assert_eq!(installed_build(&dir), Some(12));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A round trip with a real update service (start one, publish builds to it, make a tester key):
    /// `STICKFELL_UPDATE_URL=http://127.0.0.1:<port> STICKFELL_TEST_KEY=STICK-… cargo test -p stickfell_launcher -- --ignored real_service`
    #[test]
    #[ignore]
    fn real_service_round_trip() {
        let key = std::env::var("STICKFELL_TEST_KEY").expect("set STICKFELL_TEST_KEY");
        assert_eq!(check_key(&key), Ok(()));
        assert!(matches!(check_key("STICK-AAAAA-AAAAA-AAAAA-AAAAA"), Err(Problem::Key(_))));
        let launcher = latest(Channel::Launcher).unwrap().expect("publish a launcher build first");
        let game = latest(Channel::Game).unwrap().expect("publish a game build first");
        eprintln!("launcher build {}, game build {} ({:?})", launcher.build, game.build, game.message);
        let dir = std::env::temp_dir().join(format!("stickfell-round-trip-{}", std::process::id()));
        let mut seen = 0;
        let bytes = download(Channel::Game, &key, &game, |a, _| seen = a).unwrap();
        assert_eq!(seen, game.size);
        install_game(&dir, &bytes, game.build).unwrap();
        assert!(game_is(&dir, &game));
        assert_eq!(installed_build(&dir), Some(game.build));
        assert!(matches!(download(Channel::Game, "STICK-AAAAA-AAAAA-AAAAA-AAAAA", &game, |_, _| {}), Err(Problem::Key(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_secure_addresses() {
        // (via the override, which the release build's address goes through too)
        for (url, ok) in [("https://updates.example.com/", true), ("http://127.0.0.1:3000", true), ("http://updates.example.com", false), ("ftp://x", false)] {
            unsafe { std::env::set_var("STICKFELL_UPDATE_URL", url) };
            assert_eq!(base().is_ok(), ok, "{url}");
        }
        unsafe { std::env::remove_var("STICKFELL_UPDATE_URL") };
    }
}
