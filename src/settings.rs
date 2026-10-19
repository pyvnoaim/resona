//! What survives a restart: ~/.resona/settings (key=value) and ~/.resona/unplayable.

use std::collections::HashSet;

use eframe::egui;

use crate::{api::Track, skin::{self, Skin}};

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

/// The accent colour from ~/.resona/settings (the skin's red if unset).
pub fn saved_accent() -> skin::Accent {
    let saved = setting("accent").unwrap_or_default();
    skin::Accent::parse(&saved).unwrap_or_default()
}

/// Skin, theme and accent survive restarts.
pub fn save_settings(ctx: &egui::Context) {
    set_setting("skin", &format!("{:?}", skin::skin(ctx)));
    set_setting("accent", &skin::kept_accent(ctx).key()); // a name, or #rrggbb
    set_setting("theme", &format!("{:?}", ctx.options(|o| o.theme_preference)));
}

pub fn load_settings(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("accent"), saved_accent())); // before apply, which paints with it
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

/// Followed artist ids in ~/.resona/following: New releases still works while the Web API is banned.
pub fn load_following() -> Option<HashSet<String>> {
    read_ids("following")
}

pub fn save_following(ids: &[String]) {
    write_ids("following", &ids.iter().cloned().collect())
}

/// A list as last loaded, in ~/.resona/lists/: shown at once while Spotify says it's still `version`.
/// `added` and `pos` ride alongside each song: the Web API never sends them as fields.
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    version: String,
    tracks: Vec<(Track, i64, usize)>,
}

fn list_path(key: &str) -> std::path::PathBuf {
    resona_dir().join("lists").join(key.replace([':', '/'], "_"))
}

pub fn load_list(key: &str, version: &str) -> Option<Vec<Track>> {
    let kept: Kept = serde_json::from_slice(&std::fs::read(list_path(key)).ok()?).ok()?;
    (kept.version == version).then(|| kept.tracks.into_iter().map(|(t, added, pos)| Track { added, pos, ..t }).collect())
}

pub fn save_list(key: &str, version: &str, tracks: &[Track]) {
    let kept = Kept { version: version.into(), tracks: tracks.iter().map(|t| (t.clone(), t.added, t.pos)).collect() };
    let path = list_path(key);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    // A torn write just fails to parse next time, and the list loads from Spotify.
    if let Ok(json) = serde_json::to_vec(&kept) { let _ = std::fs::write(path, json); }
}

#[test]
fn a_kept_list_comes_back_only_at_its_version() {
    let key = format!("test:{}", std::process::id());
    let t = Track { id: Some("a".into()), name: "Song".into(), added: 42, pos: 7, ..Default::default() };
    save_list(&key, "v1", &[t]);
    let back = load_list(&key, "v1").unwrap();
    assert_eq!((back[0].id.as_deref(), back[0].added, back[0].pos, back[0].name.as_str()), (Some("a"), 42, 7, "Song"));
    assert!(load_list(&key, "v2").is_none());
    let _ = std::fs::remove_file(list_path(&key));
}
