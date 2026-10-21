//! New releases on GitHub: one check at launch, and the update in place: the Mac app reruns install.sh, Linux
//! and Windows swap their own binary for the release's, and Resona reopens.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::JoinHandle;

const REPO: &str = "pyvnoaim/resona";

/// The latest release's tag (`v0.2.0`) when it's newer than this build; None when it isn't, or when offline.
pub async fn check(http: &reqwest::Client) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    // Windows can't delete a running binary, so the last update left the old one beside this one.
    if let Ok(exe) = std::env::current_exe() { let _ = std::fs::remove_file(exe.with_extension("old")); }
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let r = http.get(url).header("user-agent", "resona").send().await.ok()?.error_for_status().ok()?;
    let tag = r.json::<Release>().await.ok()?.tag_name;
    newer(&tag, env!("CARGO_PKG_VERSION")).then_some(tag)
}

fn newer(tag: &str, current: &str) -> bool {
    let parts = |s: &str| s.trim_start_matches('v').split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parts(tag) > parts(current)
}

#[test]
fn newer_compares_numbers_not_text() {
    assert!(newer("v0.2.0", "0.1.0"));
    assert!(newer("v0.10.0", "0.9.3"));
    assert!(!newer("v0.1.0", "0.1.0"));
    assert!(!newer("v0.1.0", "0.2.0"));
}

/// The release's page: "What's new", and where builds the update can't replace go instead.
pub fn page(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// Whether Update can install it here: the Mac app in /Applications (not a build run from target/), or the
/// Linux x86_64 / Windows binary, which the release has a build of.
pub fn in_place() -> bool {
    let exe = std::env::current_exe().unwrap_or_default();
    if cfg!(target_os = "macos") { exe.starts_with("/Applications/Resona.app") } else { cfg!(target_arch = "x86_64") && !exe.starts_with(env!("CARGO_MANIFEST_DIR")) }
}

/// Downloads and installs the latest release on its own thread; the result says why it failed.
pub fn install() -> JoinHandle<Result<(), String>> {
    std::thread::spawn(|| {
        if cfg!(target_os = "macos") {
            let script = format!("f=$(mktemp) && curl -fsSL https://raw.githubusercontent.com/{REPO}/main/install.sh -o \"$f\" && sh \"$f\"; s=$?; rm -f \"$f\"; exit $s");
            return run(Command::new("sh").args(["-c", &script]));
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let asset = if cfg!(windows) { "resona-windows-x86_64.zip" } else { "resona-linux-x86_64.tar.gz" };
        // Unpacked beside the binary, so the swap is a rename on one disk.
        let tmp = exe.with_file_name(".resona-update");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let swapped = swap(&exe, &tmp, asset);
        let _ = std::fs::remove_dir_all(&tmp);
        swapped
    })
}

/// curl and tar come with Linux and with Windows 10 and later; tar reads both the .tar.gz and the .zip.
fn swap(exe: &Path, tmp: &Path, asset: &str) -> Result<(), String> {
    let archive = tmp.join(asset);
    run(Command::new("curl").arg("-fsSL").arg(format!("https://github.com/{REPO}/releases/latest/download/{asset}")).arg("-o").arg(&archive))?;
    run(Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(tmp))?;
    let new: PathBuf = tmp.join(exe.file_name().unwrap_or_default());
    // A running binary can be renamed, not overwritten (Windows), so the old one steps aside first.
    if cfg!(windows) { std::fs::rename(exe, exe.with_extension("old")).map_err(|e| e.to_string())?; }
    std::fs::rename(&new, exe).map_err(|e| format!("{}: {e}", exe.display()))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Starts the new Resona once this one has quit (on macOS `open` would only bring this one forward).
pub fn reopen() {
    let pid = std::process::id();
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = if cfg!(target_os = "macos") {
        Command::new("sh").args(["-c", &format!("while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; open /Applications/Resona.app")]).spawn()
    } else if cfg!(windows) {
        Command::new(&exe).spawn() // ponytail: starts while this one closes; a wait would need a helper
    } else {
        Command::new("sh").args(["-c", &format!("while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; exec \"$0\"")]).arg(&exe).spawn()
    };
}
