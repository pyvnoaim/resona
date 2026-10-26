//! New releases on GitHub: one check at launch, and the update in place: the release is downloaded (counted, for
//! the dialog's bar), then the Mac app reruns install.sh on it, Linux and Windows swap their own binary for the
//! release's, and Resona reopens.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

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

/// Bytes downloaded so far and the download's size (0 until it's known).
pub type Progress = Arc<(AtomicUsize, AtomicUsize)>;

/// `(downloaded, size)`, for the dialog.
pub fn progress(p: &Progress) -> (usize, usize) {
    (p.0.load(Relaxed), p.1.load(Relaxed))
}

/// Downloads (on `rt`) and installs release `tag` on its own thread; the result says why it failed. The Mac runs
/// install.sh as tagged with `tag`, not as it is on main, so a later edit there can't break this update.
pub fn install(tag: &str, rt: tokio::runtime::Handle) -> (JoinHandle<Result<(), String>>, Progress) {
    let (tag, got) = (tag.to_string(), Progress::default());
    let seen = got.clone();
    let thread = std::thread::spawn(move || {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let asset = if cfg!(target_os = "macos") { "Resona-macos.zip" } else if cfg!(windows) { "resona-windows-x86_64.zip" } else { "resona-linux-x86_64.tar.gz" };
        // Linux and Windows unpack beside the binary, so the swap is a rename on one disk. Named per process, so
        // two copies updating at once don't delete each other's download.
        let dir = format!(".resona-update-{}", std::process::id());
        let tmp = if cfg!(target_os = "macos") { std::env::temp_dir().join(dir) } else { exe.with_file_name(dir) };
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let archive = tmp.join(asset);
        let done = rt.block_on(download(&format!("https://github.com/{REPO}/releases/download/{tag}/{asset}"), &archive, &got)).and_then(|()| {
            if !cfg!(target_os = "macos") { return swap(&exe, &tmp, &archive) }
            // The URL goes in as $0, not into the script: a tag can hold characters the shell would read.
            // install.sh installs the zip in RESONA_ZIP instead of downloading its own.
            let url = format!("https://raw.githubusercontent.com/{REPO}/{tag}/install.sh");
            let script = "f=$(mktemp) && curl -fsSL \"$0\" -o \"$f\" && sh \"$f\"; s=$?; rm -f \"$f\"; exit $s";
            run(Command::new("sh").args(["-c", script, &url]).env("RESONA_ZIP", &archive))
        });
        let _ = std::fs::remove_dir_all(&tmp);
        done
    });
    (thread, seen)
}

/// `url` into the file `to`, counting into `got` as it comes.
async fn download(url: &str, to: &Path, got: &Progress) -> Result<(), String> {
    // No limit on the whole (a slow line takes a while), but 30 s without a byte fails it instead of hanging.
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let mut r = http.get(url).header("user-agent", "resona").send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    got.1.store(r.content_length().unwrap_or(0) as usize, Relaxed);
    let mut file = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    while let Some(bytes) = r.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        got.0.fetch_add(bytes.len(), Relaxed);
    }
    // Known now even when the server didn't say, so the dialog moves on to "Installing".
    got.1.store(got.0.load(Relaxed), Relaxed);
    Ok(())
}

/// tar comes with Linux and with Windows 10 and later, and reads both the .tar.gz and the .zip.
fn swap(exe: &Path, tmp: &Path, archive: &Path) -> Result<(), String> {
    run(Command::new(tool("tar")).arg("-xf").arg(archive).arg("-C").arg(tmp))?;
    let new: PathBuf = tmp.join(exe.file_name().unwrap_or_default());
    // A running binary can be renamed, not overwritten (Windows), so the old one steps aside first.
    if cfg!(windows) { std::fs::rename(exe, exe.with_extension("old")).map_err(|e| e.to_string())?; }
    std::fs::rename(&new, exe).map_err(|e| format!("{}: {e}", exe.display()))
}

/// Windows' own copy in System32, not one found by searching (which can start in Resona's folder).
fn tool(name: &str) -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) if cfg!(windows) => Path::new(&root).join("System32").join(format!("{name}.exe")),
        _ => PathBuf::from(name),
    }
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

#[test]
#[ignore = "downloads a release from GitHub"]
fn download_counts_every_byte() {
    let to = std::env::temp_dir().join("resona-download-test.zip");
    let got = Progress::default();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(download(&format!("https://github.com/{REPO}/releases/download/v0.3.1/Resona-macos.zip"), &to, &got)).unwrap();
    let size = std::fs::metadata(&to).unwrap().len() as usize;
    let _ = std::fs::remove_file(&to);
    assert!(size > 0 && got.0.load(Relaxed) == size && got.1.load(Relaxed) == size);
}
