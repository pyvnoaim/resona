//! Input: the keyboard, the macOS menu bar and media keys / Now Playing. All of it ends in App::command.

use std::time::Duration;

use eframe::egui;

use crate::skin::{self, Skin};
use crate::{App, settings::*, state::*};

impl App {
    /// The macOS menu bar: every action with its shortcut, so they're discoverable. ⌘ shortcuts are real
    /// key equivalents; plain keys (Space, S, R…) only appear in the item's name, since a menu key
    /// equivalent without ⌘ would also fire while typing in the search box.
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
                &item("louder", "Louder (↑)", None),
                &item("quieter", "Quieter (↓)", None),
                &item("mute", "Mute / Unmute (M)", None),
            ])?;
            let skins = Submenu::with_items("Skin", true, &[&item("skin:Wired", "Wired", None), &item("skin:Swiss", "Swiss", None)])?;
            let themes = Submenu::with_items("Theme", true, &[&item("theme:System", "Follow macOS", None), &item("theme:Dark", "Dark", None), &item("theme:Light", "Light", None)])?;
            let view = Submenu::with_items("View", true, &[
                &item("search", "Search Spotify", Some("CmdOrCtrl+F")),
                &item("jump", "Jump to the Playing Song", Some("CmdOrCtrl+J")),
                &item("focus", "Cover and Lyrics", Some("CmdOrCtrl+E")),
                &sep(),
                &skins,
                &themes,
                &item("settings", "Skin and Theme Menu…", Some("CmdOrCtrl+Comma")),
            ])?;
            let song = Submenu::with_items("Song", true, &[
                &item("like", "Like / Unlike", Some("CmdOrCtrl+L")),
                &item("copy", "Copy Song Link", Some("CmdOrCtrl+Shift+C")),
                &item("queue", "Add Song Under Pointer to Queue (Q)", None),
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
                "louder" => self.set_volume((self.volume + 0.05).min(1.0)),
                "quieter" => self.set_volume((self.volume - 0.05).max(0.0)),
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
                "settings" => ctx.data_mut(|d| d.insert_temp(egui::Id::new("open-menu"), true)),
                "like" => if let Some(id) = now_id { self.toggle_like(id) },
                "copy" => if let Some(id) = now_id { ctx.copy_text(format!("https://open.spotify.com/track/{id}")) },
                "queue" => if let (Some(t), Some(e)) = (self.hovered.clone(), self.engine.get()) {
                    self.state.lock().unwrap().enqueue(&e.player, t, false);
                },
                "help" => self.help = !self.help,
                other => {
                    if let Some(name) = other.strip_prefix("skin:") && let Some(sk) = Skin::ALL.into_iter().find(|s| format!("{s:?}") == name) {
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

    /// Hooks into macOS Now Playing: media keys, AirPods and Control Center send us commands.
    pub fn attach_media(&mut self, ctx: &egui::Context) {
        let config = souvlaki::PlatformConfig { display_name: "resona", dbus_name: "resona", hwnd: None };
        let Ok(mut controls) = souvlaki::MediaControls::new(config) else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let attached = controls.attach(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint();
        });
        if attached.is_ok() {
            self.media = Some(controls);
            self.media_rx = rx;
        }
    }

    /// Media-key commands in; the playing song and its state out (only when something changed).
    pub fn sync_media(&mut self) {
        use souvlaki::{MediaControlEvent as Ev, MediaMetadata, MediaPlayback, MediaPosition, SeekDirection};
        let events: Vec<_> = self.media_rx.try_iter().collect();
        for ev in events {
            match ev {
                Ev::Toggle => self.command(Cmd::Toggle),
                Ev::Play => self.command(Cmd::Play),
                Ev::Pause | Ev::Stop => self.command(Cmd::Pause),
                Ev::Next => self.command(Cmd::Next),
                Ev::Previous => self.command(Cmd::Prev),
                Ev::SetPosition(MediaPosition(at)) => self.command(Cmd::SeekTo(at.as_millis() as u32)),
                Ev::Seek(dir) | Ev::SeekBy(dir, _) => self.command(Cmd::SeekBy(if dir == SeekDirection::Forward { 10_000 } else { -10_000 })),
                _ => {}
            }
        }
        let Some(controls) = self.media.as_mut() else { return };
        let s = self.state.lock().unwrap();
        let sent = (s.now.as_ref().and_then(|t| t.id.clone()), s.since.is_some(), s.pos_ms);
        if sent == self.media_sent { return }
        if sent.0 != self.media_sent.0 && let Some(t) = &s.now {
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
        self.media_sent = sent;
    }

    /// The keyboard (see the "?" overlay). Nothing fires while you're typing in the search box.
    pub fn keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        if ctx.wants_keyboard_input() { return }
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let hit = |k: Key| ctx.input(|i| i.key_pressed(k));
        if hit(Key::Escape) {
            if self.help { self.help = false } else { self.big = false }
        }
        if hit(Key::Questionmark) || (shift && hit(Key::Slash)) { self.help = !self.help }
        if hit(Key::Space) { self.command(Cmd::Toggle) }
        let now = self.state.lock().unwrap().now.clone();
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
            return;
        }
        if hit(Key::ArrowRight) { self.command(Cmd::SeekBy(10_000)) }
        if hit(Key::ArrowLeft) { self.command(Cmd::SeekBy(-10_000)) }
        let step = |v: f32| (v * 20.0).round() / 20.0; // whole 5% steps
        if hit(Key::ArrowUp) { self.set_volume(step(self.volume + 0.05)); set_setting("volume", &format!("{:.2}", self.volume)) }
        if hit(Key::ArrowDown) { self.set_volume(step(self.volume - 0.05)); set_setting("volume", &format!("{:.2}", self.volume)) }
        if hit(Key::M) {
            match self.muted.take() {
                Some(v) => self.set_volume(v),
                None => { self.muted = Some(self.volume); self.set_volume(0.0) }
            }
        }
        if hit(Key::S) { self.toggle_shuffle() }
        if hit(Key::R) { self.cycle_repeat() }
        if hit(Key::Q) && let (Some(t), Some(e)) = (self.hovered.clone(), self.engine.get()) {
            let mut s = self.state.lock().unwrap();
            s.notice = Some(format!("Queued: \u{201c}{}\u{201d}", t.name));
            s.enqueue(&e.player, t, false);
        }
    }
}
