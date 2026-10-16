//! What survives a restart: ~/.resona/settings (key=value) and ~/.resona/unplayable.

use std::collections::HashSet;

use eframe::egui;

use crate::skin::{self, Skin};

/// Where Resona keeps everything: `~/.resona`.
pub fn resona_dir() -> std::path::PathBuf {
    std::env::home_dir().unwrap().join(".resona")
}

pub fn settings_path() -> std::path::PathBuf {
    resona_dir().join("settings")
}

/// One "key=value" setting from ~/.resona/settings.
pub fn setting(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(settings_path()).ok()?;
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().to_string())
}

/// Sets one setting, keeping the others.
pub fn set_setting(key: &str, value: &str) {
    let path = settings_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| l.split_once('=').is_none_or(|(k, _)| k != key)).map(String::from).collect();
    lines.push(format!("{key}={value}"));
    let _ = std::fs::create_dir_all(path.parent().unwrap()); // ponytail: a failed save just means defaults next launch
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

/// Skin and theme survive restarts.
pub fn save_settings(ctx: &egui::Context) {
    set_setting("skin", &format!("{:?}", skin::skin(ctx)));
    set_setting("theme", &format!("{:?}", ctx.options(|o| o.theme_preference)));
}

pub fn load_settings(ctx: &egui::Context) {
    let saved = setting("skin").unwrap_or_default();
    skin::apply(ctx, Skin::ALL.into_iter().find(|s| format!("{s:?}") == saved).unwrap_or_default());
    match setting("theme").as_deref() {
        Some("Dark") => ctx.set_theme(egui::ThemePreference::Dark),
        Some("Light") => ctx.set_theme(egui::ThemePreference::Light),
        _ => {}
    }
}

fn ids_path(name: &str) -> std::path::PathBuf {
    resona_dir().join(name)
}

/// Track ids, one per line in ~/.resona/<name>; None when there is no such file.
fn read_ids(name: &str) -> Option<HashSet<String>> {
    let text = std::fs::read_to_string(ids_path(name)).ok()?;
    Some(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
}

fn write_ids(name: &str, set: &HashSet<String>) {
    let _ = std::fs::write(ids_path(name), set.iter().map(|i| format!("{i}\n")).collect::<String>());
}

/// Songs Spotify refused, in ~/.resona/unplayable, so they stay skipped next launch.
pub fn remember_unplayable(id: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(ids_path("unplayable")) {
        let _ = writeln!(f, "{id}");
    }
}

/// Clicking a faded song gives it another chance: it leaves the list (and the file).
pub fn forget_unplayable(set: &mut HashSet<String>, id: &str) {
    if set.remove(id) { write_ids("unplayable", set) }
}

pub fn load_unplayable() -> HashSet<String> {
    read_ids("unplayable").unwrap_or_default()
}

/// Liked song ids in ~/.resona/liked: the hearts at startup without reading all of Liked Songs.
/// None on a first run. Only a complete set is ever saved (`State::liked_complete`).
pub fn load_liked_ids() -> Option<HashSet<String>> {
    read_ids("liked")
}

pub fn save_liked_ids(set: &HashSet<String>) {
    write_ids("liked", set)
}
