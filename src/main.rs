//! resona: a small native Spotify player. `App` holds the UI state and wires the pieces together;
//! see CLAUDE.md for the map of modules.

mod api;
mod cli;
mod controls;
mod engine;
mod settings;
mod skin;
mod state;
mod ui;

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use eframe::egui;
use librespot::{
    core::spotify_id::SpotifyId,
    playback::mixer::Mixer,
};
use engine::*;
use settings::*;
use state::*;
use ui::*;

struct App {
    rt: tokio::runtime::Runtime,
    state: Shared,
    engine: Arc<OnceLock<Engine>>,
    /// Seek position (ms) while dragging the progress blocks.
    scrub: Option<f32>,
    volume: f32,
    query: String,
    /// The big now-playing view (cover + lyrics) instead of the list.
    big: bool,
    /// Set while drawing a frame if the bouncing now-playing bars were on screen.
    bars_shown: std::cell::Cell<bool>,
    /// "Jump to the playing song": scroll the list to this track id once it's there.
    jump: Option<String>,
    /// The window title we last set, so it's only sent when the song changes.
    title: String,
    /// Saved volume and last list applied once the engine is up.
    restored: bool,
    /// Volume before muting (M).
    muted: Option<f32>,
    /// The "?" keyboard help overlay.
    help: bool,
    /// The row under the pointer, for Q (add to queue).
    hovered: Option<api::Track>,
    /// macOS Now Playing / media keys, and what we last told it.
    media: Option<souvlaki::MediaControls>,
    media_rx: std::sync::mpsc::Receiver<souvlaki::MediaControlEvent>,
    media_sent: (Option<String>, bool, u32),
    /// The macOS menu bar (kept alive here) and its clicks.
    menu: Option<muda::Menu>,
    menu_rx: std::sync::mpsc::Receiver<muda::MenuEvent>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        load_settings(&cc.egui_ctx);
        let mut app = Self {
            rt: tokio::runtime::Runtime::new().unwrap(),
            state: Default::default(),
            engine: Default::default(),
            scrub: None,
            volume: 0.7,
            query: String::new(),
            big: false,
            bars_shown: Default::default(),
            jump: None,
            title: String::new(),
            restored: false,
            muted: None,
            help: false,
            hovered: None,
            media: None,
            media_rx: std::sync::mpsc::channel().1,
            media_sent: (None, false, 0),
            menu: None,
            menu_rx: std::sync::mpsc::channel().1,
        };
        app.attach_menu(&cc.egui_ctx);
        app.volume = setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7);
        app.state.lock().unwrap().unplayable = load_unplayable();
        app.state.lock().unwrap().sort = Sort::from_key(&setting("sort").unwrap_or_default());
        app.attach_media(&cc.egui_ctx);
        let (state, slot, ctx) = (app.state.clone(), app.engine.clone(), cc.egui_ctx.clone());
        app.rt.spawn(async move {
            if let Err(e) = connect(state.clone(), slot, ctx.clone()).await {
                state.lock().unwrap().error = Some(e);
                ctx.request_repaint();
            }
        });
        app
    }

    fn open(&self, src: Source, ctx: &egui::Context) {
        if let Some(e) = self.engine.get() { open_list(self.rt.handle(), e, &self.state, src, ctx) }
    }

    fn command(&mut self, cmd: Cmd) {
        if let Some(e) = self.engine.get() { command(e, &mut self.state.lock().unwrap(), cmd) }
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(e) = self.engine.get() { e.mixer.set_volume((self.volume * u16::MAX as f32) as u16) }
    }

    /// Starts fetching lyrics for the playing track, once per track. Spotify's lyrics service goes
    /// through the librespot session (the Web API has none).
    fn fetch_lyrics(&self, s: &mut State) {
        let (Some(e), Some(id)) = (self.engine.get(), s.now.as_ref().and_then(|t| t.id.clone())) else { return };
        if s.lyrics.as_ref().is_some_and(|(have, _)| *have == id) { return }
        s.lyrics = Some((id.clone(), Lyrics::Loading));
        let (state, session) = (self.state.clone(), e.session.clone());
        self.rt.spawn(async move {
            let got = match SpotifyId::from_base62(track_id(&id)) {
                Ok(sid) => librespot::metadata::Lyrics::get(&session, &sid).await.ok(),
                Err(_) => None,
            };
            let lyrics = match got {
                Some(l) => {
                    let synced = !matches!(l.lyrics.sync_type, librespot::metadata::lyrics::SyncType::Unsynced);
                    let lines = l.lyrics.lines.into_iter().map(|line| (line.start_time_ms.parse().unwrap_or(0), line.words)).collect();
                    Lyrics::Lines { synced, lines }
                }
                None => Lyrics::Missing, // most often: Spotify has no lyrics for this song
            };
            let mut s = state.lock().unwrap();
            if s.lyrics.as_ref().is_some_and(|(have, _)| *have == id) {
                s.lyrics = Some((id, lyrics));
            }
        });
    }

    /// Appends a track to one of your playlists, then says so in the header.
    fn add_to(&self, playlist: String, name: String, track: String) {
        let Some(e) = self.engine.get() else { return };
        let (state, web, http) = (self.state.clone(), e.web.clone(), e.http.clone());
        self.rt.spawn(async move {
            let done = api::add_to_playlist(&web, &http, &playlist, &track).await;
            let mut s = state.lock().unwrap();
            match done {
                Ok(()) => {
                    s.members.entry(playlist).or_default().insert(track);
                    s.notice = Some(format!("Added to {name}"));
                }
                Err(err) => s.error = Some(err),
            }
        });
    }

    /// Starts reading what's in your own playlists, once, for the ticks in "Add to playlist".
    fn load_members(&self) {
        let Some(e) = self.engine.get() else { return };
        if std::mem::replace(&mut self.state.lock().unwrap().members_asked, true) { return }
        self.rt.spawn(load_members(self.state.clone(), e.web.clone(), e.http.clone(), e.session.username()));
    }

    fn toggle_like(&self, id: String) {
        if let Some(e) = self.engine.get() { toggle_like(self.rt.handle(), e, &self.state, id) }
    }

    /// Once the engine is up: the saved volume, and the list that was open last time.
    fn restore(&mut self, ctx: &egui::Context) {
        if self.restored || self.engine.get().is_none() { return }
        self.restored = true;
        self.set_volume(self.volume);
        self.open(setting("last").and_then(|k| Source::from_key(&k)).unwrap_or(Source::Liked), ctx);
    }

    /// "Song · Artist" in the window title (Dock, Mission Control, ⌘-Tab), sent only when it changes.
    fn window_title(&mut self, ctx: &egui::Context) {
        let title = self.state.lock().unwrap().now.as_ref().map_or("resona".to_string(), |t| format!("{} · {}", t.name, t.artist_line()));
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn toggle_shuffle(&mut self) {
        self.state.lock().unwrap().toggle_shuffle()
    }

    fn cycle_repeat(&mut self) {
        self.state.lock().unwrap().cycle_repeat()
    }

    /// Opens the list the playing song came from (if it isn't open) and scrolls to it.
    fn jump_to_playing(&mut self, ctx: &egui::Context) {
        let (src, id) = {
            let s = self.state.lock().unwrap();
            (s.queue_src.clone(), s.now.as_ref().and_then(|t| t.id.clone()))
        };
        self.big = false;
        if let Some(src) = src { self.open(src, ctx) }
        self.jump = id;
    }
}

/// Switches the main view and streams that list in, page by page.
pub fn open_list(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, src: Source, ctx: &egui::Context) {
    {
        let mut s = state.lock().unwrap();
        if s.view.as_ref() == Some(&src) { return }
        (s.view, s.loading, s.error, s.expected, s.notice, s.artist) = (Some(src.clone()), true, None, None, None, None);
        s.tracks.clear();
        if src == Source::Queue {
            s.loading = false; // drawn straight from the play queue
            return;
        }
    }
    if let Some(key) = src.key() { set_setting("last", &key) }
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    let me = e.session.username();
    let foreign = match &src {
        Source::Playlist(id) => state.lock().unwrap().playlists.iter().any(|p| &p.id == id && !p.web_readable(&me)),
        _ => false,
    };
    if let (true, Source::Playlist(id)) = (foreign, src.clone()) {
        let session = e.session.clone();
        rt.spawn(async move {
            let done = api::session_playlist(&session, &id, |tracks, total| {
                let mut s = state.lock().unwrap();
                if s.view.as_ref() != Some(&src) { return false } // user moved on
                s.expected = Some(total);
                s.add_tracks(tracks);
                ctx.request_repaint();
                true
            }).await;
            {
                let mut s = state.lock().unwrap();
                if s.view.as_ref() == Some(&src) {
                    (s.loading, s.error) = (false, done.err());
                }
            }
            ctx.request_repaint();
        });
        return;
    }
    if matches!(src, Source::Artist { .. } | Source::Album { .. }) {
        // Artist and album pages come through the session, like the official client.
        let session = e.session.clone();
        rt.spawn(async move {
            let got = match &src {
                Source::Artist { id, .. } => match api::session_artist(&session, id).await {
                    Ok((portrait, top, releases)) => {
                        // No photo in the session's artist record: ask the Web API (middle-sized image).
                        let portrait = match portrait {
                            Some(p) => Some(p),
                            None => api::get::<api::ArtistImages>(&web, &http, &format!("artists/{id}")).await.ok()
                                .and_then(|a| a.images.get(1).or(a.images.first()).map(|i| i.url.clone())),
                        };
                        Ok((top, Some(ArtistPage { portrait, releases })))
                    }
                    Err(e) => Err(e),
                },
                Source::Album { id, .. } => api::session_album(&session, id).await.map(|t| (t, None)),
                _ => return,
            };
            let mut s = state.lock().unwrap();
            if s.view.as_ref() != Some(&src) { return } // user moved on
            match got {
                Ok((tracks, page)) => { s.artist = page; s.add_tracks(tracks) }
                Err(e) => s.error = Some(e),
            }
            s.loading = false;
            ctx.request_repaint();
        });
        return;
    }
    rt.spawn(async move {
        let edits = state.lock().unwrap().liked_edits;
        let mut url = Some(match &src {
            Source::Liked => "me/tracks?limit=50".to_string(),
            Source::Playlist(id) => format!("playlists/{id}/items?limit=100"),
            Source::Search(q) => api::search_url(q),
            Source::Artist { .. } | Source::Album { .. } | Source::Queue => return,
        });
        let mut pages = 0;
        while let Some(u) = url {
            let page = match &src {
                Source::Search(_) => api::get::<api::Search>(&web, &http, &u).await
                    .map(|r| (r.tracks.items, r.tracks.next, r.tracks.total.map(|t| t.min(50)))),
                _ => api::get::<api::Page<api::Entry>>(&web, &http, &u).await
                    .map(|p| (p.items.into_iter().filter_map(api::Entry::playable).collect(), p.next, p.total)),
            };
            let mut s = state.lock().unwrap();
            if s.view.as_ref() != Some(&src) { return } // user moved on
            pages += 1;
            match page {
                Ok((tracks, next, total)) => {
                    s.expected = s.expected.or(total);
                    s.add_tracks(tracks);
                    // Search pages are small; 5 of them is plenty, and keeps us off the rate limit.
                    url = next.filter(|_| !matches!(src, Source::Search(_)) || pages < 5);
                }
                Err(e) => { s.error = Some(e); url = None; }
            }
            ctx.request_repaint();
        }
        let mut s = state.lock().unwrap();
        s.loading = false;
        if src == Source::Liked && s.error.is_none() && s.liked_edits == edits {
            // The whole of Liked Songs just loaded anyway: that's the fresh set of hearts (unless a like or
            // unlike landed meanwhile; then the pages may predate it, and the set stays as it is).
            s.liked = s.tracks.iter().filter_map(|t| t.id.clone()).collect();
            s.liked_complete = true;
            save_liked_ids(&s.liked);
        }
        drop(s);
        ctx.request_repaint();
    });
}

/// Likes or unlikes a track: the heart flips at once, and flips back if Spotify says no.
pub fn toggle_like(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, id: String) {
    let on = !state.lock().unwrap().liked.contains(&id);
    set_like(state, &id, on);
    let (state, web, http) = (state.clone(), e.web.clone(), e.http.clone());
    rt.spawn(async move {
        if let Err(err) = api::set_liked(&web, &http, &id, on).await {
            set_like(&state, &id, !on);
            state.lock().unwrap().error = Some(err);
        }
    });
}

/// Runs one playback command; buttons, media keys and the keyboard all come through here.
pub fn command(e: &Engine, s: &mut State, cmd: Cmd) {
    let pos = s.position();
    let start_open_list = |s: &mut State| {
        // Nothing loaded yet: play the open list from the top.
        let (list, src) = (s.tracks.clone(), s.view.clone());
        if !list.is_empty() { s.start(&e.player, list, None, src) }
    };
    match cmd {
        Cmd::Toggle if s.now.is_none() => start_open_list(s),
        Cmd::Toggle => if s.since.is_some() { e.player.pause() } else { e.player.play() },
        Cmd::Play if s.now.is_none() => start_open_list(s),
        Cmd::Play => e.player.play(),
        Cmd::Pause => e.player.pause(),
        Cmd::Next => { let from = s.qi; s.step(&e.player, from, 1); }
        Cmd::Prev => {
            // Like every player: restart the song unless we're at its very start.
            let from = s.qi;
            if pos > 3000 || !s.step(&e.player, from, -1) { e.player.seek(0) }
        }
        Cmd::SeekTo(ms) => e.player.seek(ms),
        Cmd::SeekBy(d) => {
            let dur = s.now.as_ref().map_or(0, |t| t.duration_ms) as i64;
            e.player.seek((pos as i64 + d).clamp(0, dur.max(1) - 1) as u32);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.restore(ctx);
        self.sync_media();
        self.menu_events(ctx);
        self.keys(ctx);
        self.window_title(ctx);
        self.bars_shown.set(false);
        // Windows sit on the bare ground: `gap` between them, `edge` to the window border so square
        // frames clear macOS's rounded corners. Swiss has neither: its areas run edge to edge.
        let l = skin::look(ctx);
        let (g, half, edge) = (l.gap, l.gap / 2, l.edge);
        let ground = |left, right, top, bottom| egui::Frame::new().fill(skin::pal(ctx).ground).inner_margin(egui::Margin { left, right, top, bottom });
        egui::TopBottomPanel::bottom("player")
            .exact_height(l.player_h)
            .show_separator_line(false)
            .frame(ground(edge, edge, half, edge))
            .show(ctx, |ui| self.player_bar(ui, ctx));
        if !self.big {
            egui::SidePanel::left("library")
                .resizable(true)
                .default_width(280.0)
                .width_range(200.0..=420.0)
                .show_separator_line(false)
                .frame(ground(edge, half, g, half))
                .show(ctx, |ui| self.sidebar(ui, ctx));
        }
        // Focus view runs edge to edge over its own backdrop.
        let center = if self.big { ground(0, 0, 0, 0) } else { ground(half, edge, g, half) };
        egui::CentralPanel::default().frame(center).show(ctx, |ui| self.tracklist(ui));
        if self.help { help_overlay(ctx) }

        // Tick the clock only while music plays; idle costs nothing.
        if self.state.lock().unwrap().loading {
            ctx.request_repaint_after(Duration::from_millis(60)); // animate the loader
        } else if self.state.lock().unwrap().since.is_some() {
            // Smooth bars only while they're on screen; otherwise 4 fps is plenty for the clock.
            ctx.request_repaint_after(Duration::from_millis(if self.bars_shown.get() { 50 } else { 250 }));
        }
    }
}

fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("cli") {
        cli::run();
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 760.0]).with_min_inner_size([760.0, 480.0]).with_title("resona")
            // Empty, or eframe swaps in its own default icon over the bundle's at runtime.
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        ..Default::default()
    };
    eframe::run_native("resona", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
