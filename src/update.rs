//! New releases on GitHub: one check at launch, and the one-click update of the app install.sh put in
//! /Applications (it runs install.sh again, then reopens Resona).

use std::process::{Child, Command, Stdio};

const REPO: &str = "pyvnoaim/resona";

/// The latest release's tag (`v0.2.0`) when it's newer than this build; None when it isn't, or when offline.
pub async fn check(http: &reqwest::Client) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
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

/// The release's page, for the systems (and builds) the one click doesn't cover.
pub fn page(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// Whether the one click can do it: this is the app install.sh (or bundle.sh) put in /Applications.
// ponytail: macOS only; Linux and Windows get the release page. Swapping their binary in place is the upgrade.
pub fn one_click() -> bool {
    cfg!(target_os = "macos") && std::env::current_exe().is_ok_and(|p| p.starts_with("/Applications/Resona.app"))
}

/// install.sh, fetched and run in the background; it replaces /Applications/Resona.app while this one runs.
pub fn install() -> std::io::Result<Child> {
    let script = format!(
        "f=$(mktemp) && curl -fsSL https://raw.githubusercontent.com/{REPO}/main/install.sh -o \"$f\" && sh \"$f\"; s=$?; rm -f \"$f\"; exit $s"
    );
    Command::new("sh").args(["-c", &script]).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()
}

/// Opens the new app once this process has quit (macOS would only bring a running Resona forward).
pub fn reopen() {
    let wait = format!("while kill -0 {} 2>/dev/null; do sleep 0.2; done; open /Applications/Resona.app", std::process::id());
    let _ = Command::new("sh").args(["-c", &wait]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
