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

/// Marks one artist followed or not in `following`, so the file keeps up with the Follow button. Only once
/// there is a file: one started here would hold just this artist, and New releases would take it for the whole list.
pub fn set_following(id: &str, on: bool) {
    let Some(mut kept) = load_following() else { return };
    let changed = if on { kept.insert(id.to_string()) } else { kept.remove(id) };
    if changed { write_ids("following", &kept) }
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

/// The copy kept from last time and the version it was saved at.
pub fn load_list(key: &str) -> Option<(String, Vec<Track>)> {
    let kept: Kept = serde_json::from_slice(&std::fs::read(list_path(key)).ok()?).ok()?;
    Some((kept.version, kept.tracks.into_iter().map(|(t, added, pos)| Track { added, pos, ..t }).collect()))
}

pub fn save_list(key: &str, version: &str, tracks: &[Track]) {
    let kept = Kept { version: version.into(), tracks: tracks.iter().map(|t| (t.clone(), t.added, t.pos)).collect() };
    let path = list_path(key);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    // A torn write just fails to parse next time, and the list loads from Spotify.
    if let Ok(json) = serde_json::to_vec(&kept) { let _ = std::fs::write(path, json); }
}

#[test]
fn a_kept_list_comes_back_with_its_version() {
    let key = format!("test:{}", std::process::id());
    let t = Track { id: Some("a".into()), name: "Song".into(), added: 42, pos: 7, ..Default::default() };
    save_list(&key, "v1", &[t]);
    let (v, back) = load_list(&key).unwrap();
    assert_eq!((v.as_str(), back[0].id.as_deref(), back[0].added, back[0].pos, back[0].name.as_str()), ("v1", Some("a"), 42, 7, "Song"));
    let _ = std::fs::remove_file(list_path(&key));
}

/// Where playback was, in ~/.resona/queue: the queue, the song in it and how far in. The next launch picks it
/// up paused (`resume`). Written on every play and pause (engine.rs) and on quit, so a kill loses only the last stretch.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Resume {
    pub queue: Vec<Track>,
    pub base: Vec<Track>,
    pub qi: usize,
    pub queued: usize,
    pub shuffle: bool,
    pub src: Option<String>,
    pub ms: u32,
}

/// `base` is kept only while shuffled: otherwise it's the queue without its hand-queued songs. The copy is made
/// here, under the state lock; the JSON (megabytes for a big Liked Songs) is written on a thread, then renamed in
/// whole, so a kill mid-write leaves the last good one.
/// ponytail: two saves a moment apart may land out of order; a writer thread with a channel if that ever shows.
/// Returns the writing thread, for a quit to wait on.
pub fn save_resume(s: &crate::state::State) -> Option<std::thread::JoinHandle<()>> {
    s.now.as_ref()?; // nothing playing: keep what was
    let r = Resume {
        queue: s.queue.clone(), base: if s.shuffle { s.base.clone() } else { Vec::new() }, qi: s.qi, queued: s.queued,
        shuffle: s.shuffle, src: s.queue_src.as_ref().and_then(|src| src.key()), ms: s.position(),
    };
    Some(std::thread::spawn(move || {
        let Ok(json) = serde_json::to_vec(&r) else { return };
        let tmp = resona_dir().join(format!("queue.{:?}", std::thread::current().id()));
        if std::fs::write(&tmp, json).is_ok() { let _ = std::fs::rename(&tmp, resona_dir().join("queue")); }
    }))
}

impl Resume {
    /// The queue's own order: `base` as saved, or the queue without the hand-queued songs after `qi`.
    pub fn base(&self) -> Vec<Track> {
        if self.shuffle { return self.base.clone() }
        let mut base = self.queue.clone();
        let from = (self.qi + 1).min(base.len());
        base.drain(from..(from + self.queued).min(base.len()));
        base
    }
}

#[test]
fn an_unshuffled_resume_rebuilds_base_without_the_hand_queued() {
    let t = |id: &str| Track { id: Some(id.into()), ..Default::default() };
    let r = Resume { queue: vec![t("a"), t("b"), t("q"), t("c")], base: vec![], qi: 1, queued: 1, shuffle: false, src: None, ms: 0 };
    let ids: Vec<_> = r.base().into_iter().map(|t| t.id.unwrap()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

pub fn load_resume() -> Option<Resume> {
    serde_json::from_slice(&std::fs::read(resona_dir().join("queue")).ok()?).ok()
}
