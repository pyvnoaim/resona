//! Input: the keyboard, the macOS menu bar and media keys / Now Playing. All of it ends in App::command.
//! Without the menu bar (Windows, Linux) the keyboard handles the Ctrl shortcuts itself.

use std::time::Duration;

use eframe::egui;

#[cfg(target_os = "macos")]
use crate::skin;
use crate::{App, settings::*, state::*, ui::side_lists};

impl App {
    /// The macOS menu bar: every action with its shortcut, so they're discoverable. ⌘ shortcuts are real
    /// key equivalents; plain keys (Space, S, R…) only appear in the item's name, since a menu key
    /// equivalent without ⌘ would also fire while typing in the search box.
    #[cfg(target_os = "macos")]
    pub fn attach_menu(&mut self, ctx: &egui::Context) {
        use muda::{Menu, MenuItem, PredefinedMenuItem as Pre, Submenu};
        let item = |id: &str, text: &str, keys: Option<&str>| MenuItem::with_id(id, text, true, keys.and_then(|k| k.parse().ok()));
        let sep = Pre::separator;
        let built = (|| -> muda::Result<Menu> {
            let app = Submenu::with_items("resona", true, &[&Pre::about(None, None), &sep(), &Pre::services(None), &sep(), &Pre::hide(None), &Pre::hide_others(None), &Pre::show_all(None), &sep(), &Pre::quit(None)])?;
            let playback = Submenu::with_items("Playback", true, &[
                &item("toggle", "Play / Pause (Space)", None),
                &item("next", "Next Song", Some("CmdOrCtrl+Right")),
                &item("prev", "Previous Song", Some("CmdOrCtrl+Left")),
                &sep(),
                &item("fwd", "Forward 10 s (→)", None),
                &item("back", "Back 10 s (←)", None),
                &sep(),
                &item("shuffle", "Shuffle (S)", None),
                &item("repeat", "Repeat: Off / All / One (R)", None),
                &sep(),
                &item("louder", "Louder (+)", None),
                &item("quieter", "Quieter (-)", None),
                &item("mute", "Mute / Unmute (M)", None),
            ])?;
            let skins = Submenu::with_items("Skin", true, &[&item("skin:Wired", "Wired", None), &item("skin:Swiss", "Swiss", None), &item("skin:Ascii", "ASCII", None)])?;
            let themes = Submenu::with_items("Theme", true, &[&item("theme:System", "Follow macOS", None), &item("theme:Dark", "Dark", None), &item("theme:Light", "Light", None)])?;
            let accent_items: Vec<_> = skin::Accent::ALL.iter().map(|a| item(&format!("accent:{}", a.key()), &a.key(), None)).collect();
            let accent_refs: Vec<&dyn muda::IsMenuItem> = accent_items.iter().map(|i| i as &dyn muda::IsMenuItem).collect();
            let accents = Submenu::with_items("Accent", true, &accent_refs)?;
            let view = Submenu::with_items("View", true, &[
                &item("search", "Search Spotify", Some("CmdOrCtrl+F")),
                &item("jump", "Jump to the Playing Song", Some("CmdOrCtrl+J")),
                &item("focus", "Cover and Lyrics", Some("CmdOrCtrl+E")),
                &item("sort", "Sort: List Order / Newest / Oldest Added", Some("CmdOrCtrl+O")),
                &item("goback", "Back to the Last List (Esc)", Some("CmdOrCtrl+BracketLeft")),
                &sep(),
                &skins,
                &themes,
                &accents,
                &item("settings", "Skin and Theme Menu…", Some("CmdOrCtrl+Comma")),
            ])?;
            let song = Submenu::with_items("Song", true, &[
                &item("like", "Like / Unlike", Some("CmdOrCtrl+L")),
                &item("copy", "Copy Song Link", Some("CmdOrCtrl+Shift+C")),
                &item("queue", "Add Highlighted Song to Queue (Q)", None),
            ])?;
            let window = Submenu::with_items("Window", true, &[&Pre::minimize(None), &Pre::fullscreen(None)])?;
            let help = Submenu::with_items("Help", true, &[&item("help", "Keyboard Shortcuts (?)", None)])?;
            Menu::with_items(&[&app, &playback, &view, &song, &window, &help])
        })();
        let Ok(menu) = built else { return };
        menu.init_for_nsapp();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |e| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
        self.menu = Some(menu);
        self.menu_rx = rx;
    }

    /// Menu bar clicks (and ⌘ shortcuts, which the menu receives first).
    #[cfg(target_os = "macos")]
    pub fn menu_events(&mut self, ctx: &egui::Context) {
        let ids: Vec<String> = self.menu_rx.try_iter().map(|e| e.id().0.clone()).collect();
        for id in ids {
            let now = self.state.lock().unwrap().now.clone();
            let now_id = now.as_ref().and_then(|t| t.id.clone());
            match id.as_str() {
                "toggle" => self.command(Cmd::Toggle),
                "next" => self.command(Cmd::Next),
                "prev" => self.command(Cmd::Prev),
                "fwd" => self.command(Cmd::SeekBy(10_000)),
                "back" => self.command(Cmd::SeekBy(-10_000)),
                "shuffle" => self.toggle_shuffle(),
                "repeat" => self.cycle_repeat(),
                "louder" => self.step_volume(0.05),
                "quieter" => self.step_volume(-0.05),
                "mute" => match self.muted.take() {
                    Some(v) => self.set_volume(v),
                    None => { self.muted = Some(self.volume); self.set_volume(0.0) }
                },
                "search" => {
                    self.big = false;
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("search")));
                }
                "jump" => self.jump_to_playing(ctx),
                "focus" => self.big = !self.big,
                "sort" => self.state.lock().unwrap().cycle_sort(),
                "goback" => self.go_back(ctx),
                "settings" => ctx.data_mut(|d| d.insert_temp(egui::Id::new("open-menu"), true)),
                "like" => if let Some(id) = now_id { self.toggle_like(id) },
                "copy" => if let Some(id) = now_id { ctx.copy_text(format!("https://open.spotify.com/track/{id}")) },
                "queue" => self.queue_cursor(),
                "help" => self.help = !self.help,
                other => {
                    if let Some(name) = other.strip_prefix("accent:") && let Some(a) = skin::Accent::parse(name) {
                        skin::set_accent(ctx, a);
                        save_settings(ctx);
                    }
                    if let Some(name) = other.strip_prefix("skin:") && let Some(sk) = skin::Skin::ALL.into_iter().find(|s| format!("{s:?}") == name) {
                        skin::apply(ctx, sk);
                        save_settings(ctx);
                    }
                    let theme = match other {
                        "theme:System" => Some(egui::ThemePreference::System),
                        "theme:Dark" => Some(egui::ThemePreference::Dark),
                        "theme:Light" => Some(egui::ThemePreference::Light),
                        _ => None,
                    };
                    if let Some(t) = theme {
                        ctx.set_theme(t);
                        save_settings(ctx);
                    }
                }
            }
        }
    }

    /// Hooks into the system's media controls (see `media_controls`); a key press wakes the window.
    pub fn attach_media(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        if let Some((controls, rx)) = media_controls(move || ctx.request_repaint()) {
            self.media = Some(controls);
            self.media_rx = rx;
        }
    }

    /// Media-key commands in; the playing song and its state out (only when something changed).
    pub fn sync_media(&mut self) {
        let cmds: Vec<_> = self.media_rx.try_iter().filter_map(media_cmd).collect();
        for c in cmds { self.command(c) }
        let Some(controls) = self.media.as_mut() else { return };
        media_show(controls, &self.state.lock().unwrap(), &mut self.media_sent);
    }

    /// + and -: whole 5% steps, saved. While muted they start from the volume before the mute, and unmute.
    fn step_volume(&mut self, by: f32) {
        let from = self.muted.take().unwrap_or(self.volume);
        self.set_volume(((from + by) * 20.0).round() / 20.0);
        set_setting("volume", &format!("{:.2}", self.volume));
    }

    /// Q: the song under the cursor goes to the end of the queue.
    fn queue_cursor(&mut self) {
        let Some(e) = self.engine.get() else { return };
        if self.big { return } // no list on screen, so no highlighted song
        let mut s = self.state.lock().unwrap();
        if let (Pane::Songs, Some(t)) = (self.pane, s.tracks.get(self.row).cloned()) {
            s.enqueue(&e.player(), t, false);
        }
    }

    /// The keyboard (see the "?" overlay). Nothing fires while you're typing in the search box.
    pub fn keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let tab = std::mem::take(&mut self.tab);
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) { self.go_back(ctx) } // the mouse's back button
        if ctx.wants_keyboard_input() { return }
        if !tab && !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { .. }))) { return } // no key, nothing to do
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let hit = |k: Key| ctx.input(|i| i.key_pressed(k));
        // Not while the key is held: a repeated L would be a burst of likes, a repeated Enter of restarts.
        let once = |k: Key| ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, .. } if *key == k)));
        if hit(Key::Escape) {
            if self.help { self.help = false } else if self.big { self.big = false } else if !egui::Popup::is_any_open(ctx) { self.go_back(ctx) }
        }
        if hit(Key::Questionmark) || (shift && hit(Key::Slash)) { self.help = !self.help }
        if hit(Key::Space) { self.command(Cmd::Toggle) }
        let now = self.state.lock().unwrap().now.clone();
        #[cfg(target_os = "macos")]
        if cmd && self.menu.is_some() { return } // the menu bar owns the ⌘ shortcuts
        if cmd {
            if hit(Key::ArrowRight) { self.command(Cmd::Next) }
            if hit(Key::ArrowLeft) { self.command(Cmd::Prev) }
            if hit(Key::F) {
                self.big = false;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("search")));
            }
            if hit(Key::J) { self.jump_to_playing(ctx) }
            if hit(Key::E) { self.big = !self.big }
            if hit(Key::L) && let Some(id) = now.as_ref().and_then(|t| t.id.clone()) { self.toggle_like(id) }
            if shift && hit(Key::C) && let Some(id) = now.as_ref().and_then(|t| t.id.clone()) {
                ctx.copy_text(format!("https://open.spotify.com/track/{id}"));
            }
            if hit(Key::Comma) { ctx.data_mut(|d| d.insert_temp(egui::Id::new("open-menu"), true)) }
            if hit(Key::O) { self.state.lock().unwrap().cycle_sort() }
            if hit(Key::OpenBracket) { self.go_back(ctx) }
            return;
        }
        if hit(Key::ArrowRight) { self.command(Cmd::SeekBy(10_000)) }
        if hit(Key::ArrowLeft) { self.command(Cmd::SeekBy(-10_000)) }
        if hit(Key::Plus) || hit(Key::Equals) { self.step_volume(0.05) }
        if hit(Key::Minus) { self.step_volume(-0.05) }
        if hit(Key::M) {
            match self.muted.take() {
                Some(v) => self.set_volume(v),
                None => { self.muted = Some(self.volume); self.set_volume(0.0) }
            }
        }
        if hit(Key::S) { self.toggle_shuffle() }
        if hit(Key::R) { self.cycle_repeat() }
        if once(Key::Q) { self.queue_cursor() }
        // No list to move through: the cover view is up, or the help or a menu is over it.
        if self.big || self.help || egui::Popup::is_any_open(ctx) { return }
        // The highlighted song, or the playing one while the cursor is in the sidebar.
        let song = if self.pane == Pane::Songs { self.state.lock().unwrap().tracks.get(self.row).cloned() } else { now };
        if once(Key::L) && let Some(id) = song.and_then(|t| t.id) { self.toggle_like(id) }
        if tab {
            self.pane = if self.pane == Pane::Lists { Pane::Songs } else { Pane::Lists };
            self.reveal = true;
        }
        let lists = side_lists(&self.state.lock().unwrap());
        let (cur, len) = match self.pane { Pane::Lists => (&mut self.side, lists.len()), Pane::Songs => (&mut self.row, self.state.lock().unwrap().tracks.len()) };
        let last = len.saturating_sub(1);
        *cur = (*cur).min(last); // the list may have got shorter under the cursor
        let was = *cur;
        let page = 10; // ponytail: a fixed stride, the visible row count if it should match the window
        if hit(Key::ArrowUp) || hit(Key::K) { *cur = cur.saturating_sub(1) }
        if hit(Key::ArrowDown) || hit(Key::J) { *cur = (*cur + 1).min(last) }
        if hit(Key::PageUp) { *cur = cur.saturating_sub(page) }
        if hit(Key::PageDown) { *cur = (*cur + page).min(last) }
        if hit(Key::Home) { *cur = 0 }
        if hit(Key::End) { *cur = last }
        if *cur != was { self.reveal = true }
        if once(Key::Enter) {
            match self.pane {
                Pane::Songs => self.enter = Some(self.row),
                Pane::Lists => if let Some(src) = lists.into_iter().nth(self.side) {
                    self.pane = Pane::Songs;
                    self.open(src, ctx);
                },
            }
        }
    }
}

/// What the system was last told: the song, playing or not, and the position it was given.
pub type MediaSent = (Option<String>, bool, u32);

/// The system's media controls: macOS Now Playing (media keys, AirPods, Control Center), Linux MPRIS. `wake`
/// runs on every event, which arrives on the returned channel.
/// ponytail: not on Windows, where souvlaki needs the window's HWND (and panics without it); pass it from
/// eframe's CreationContext to get media keys there (the CLI has no window at all).
pub fn media_controls(wake: impl Fn() + Send + 'static) -> Option<(souvlaki::MediaControls, std::sync::mpsc::Receiver<souvlaki::MediaControlEvent>)> {
    if cfg!(target_os = "windows") { return None } // souvlaki panics there without an HWND
    let config = souvlaki::PlatformConfig { display_name: "resona", dbus_name: "resona", hwnd: None };
    let mut controls = souvlaki::MediaControls::new(config).ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    controls.attach(move |event| {
        let _ = tx.send(event);
        wake();
    }).ok()?;
    Some((controls, rx))
}

/// A media key as a playback command.
pub fn media_cmd(ev: souvlaki::MediaControlEvent) -> Option<Cmd> {
    use souvlaki::{MediaControlEvent as Ev, MediaPosition, SeekDirection};
    Some(match ev {
        Ev::Toggle => Cmd::Toggle,
        Ev::Play => Cmd::Play,
        Ev::Pause | Ev::Stop => Cmd::Pause,
        Ev::Next => Cmd::Next,
        Ev::Previous => Cmd::Prev,
        Ev::SetPosition(MediaPosition(at)) => Cmd::SeekTo(at.as_millis() as u32),
        Ev::Seek(dir) | Ev::SeekBy(dir, _) => Cmd::SeekBy(if dir == SeekDirection::Forward { 10_000 } else { -10_000 }),
        _ => return None,
    })
}

/// Tells the system what's playing, only when that changed since `sent`.
pub fn media_show(controls: &mut souvlaki::MediaControls, s: &State, sent: &mut MediaSent) {
    use souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};
    let now = (s.now.as_ref().and_then(|t| t.id.clone()), s.since.is_some(), s.pos_ms);
    if now == *sent { return }
    // the same, as a file for other apps on this Mac (notchroom's record player)
    if let Ok(json) = serde_json::to_vec(&serde_json::json!({
        "title": s.now.as_ref().map(|t| t.name.as_str()),
        "artist": s.now.as_ref().map(|t| t.artist_line()),
        "cover": s.now.as_ref().and_then(|t| t.album.images.first()).map(|i| i.url.as_str()),
        "playing": s.since.is_some(),
    })) { let _ = std::fs::write(crate::settings::resona_dir().join("now-playing"), json); }
    if now.0 != sent.0 && let Some(t) = &s.now {
        let artist = t.artist_line();
        let _ = controls.set_metadata(MediaMetadata {
            title: Some(&t.name),
            artist: Some(&artist),
            album: Some(&t.album.name),
            cover_url: t.album.images.first().map(|i| i.url.as_str()),
            duration: Some(Duration::from_millis(t.duration_ms as u64)),
        });
    }
    let progress = Some(MediaPosition(Duration::from_millis(s.position() as u64)));
    let _ = controls.set_playback(match (&s.now, s.since.is_some()) {
        (None, _) => MediaPlayback::Stopped,
        (_, true) => MediaPlayback::Playing { progress },
        (_, false) => MediaPlayback::Paused { progress },
    });
    *sent = now;
}
