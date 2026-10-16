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
    time::{Duration, Instant},
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
    /// The name being typed for a new playlist (the field under the sidebar's lists).
    new_list: String,
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
    /// The cursor: the pane it's in and its row in the sidebar and in the open list. Arrows move it, and so
    /// does the mouse, so there is one highlighted row for Enter, Q and L to act on.
    pane: Pane,
    side: usize,
    row: usize,
    /// A key moved the cursor this frame: scroll it into view.
    reveal: bool,
    /// Tab was pressed (`raw_input_hook` keeps it from egui).
    tab: bool,
    /// Enter on a song: the row to play, taken by the track list as if it were clicked.
    enter: Option<usize>,
    /// macOS Now Playing / media keys, and what we last told it.
    media: Option<souvlaki::MediaControls>,
    media_rx: std::sync::mpsc::Receiver<souvlaki::MediaControlEvent>,
    media_sent: (Option<String>, bool, u32),
    /// First launch without a client ID: what's typed into the setup screen. Nothing connects until it's saved.
    setup: Option<String>,
    /// The accent the Dock icon was last drawn in.
    icon_accent: Option<skin::Accent>,
    /// The macOS menu bar (kept alive here) and its clicks.
    #[cfg(target_os = "macos")]
    menu: Option<muda::Menu>,
    #[cfg(target_os = "macos")]
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
            new_list: String::new(),
            big: false,
            bars_shown: Default::default(),
            jump: None,
            title: String::new(),
            restored: false,
            muted: None,
            help: false,
            pane: Pane::Songs,
            side: 0,
            row: 0,
            reveal: false,
            tab: false,
            enter: None,
            media: None,
            media_rx: std::sync::mpsc::channel().1,
            media_sent: (None, false, 0),
            #[cfg(target_os = "macos")]
            menu: None,
            #[cfg(target_os = "macos")]
            menu_rx: std::sync::mpsc::channel().1,
            setup: None,
            icon_accent: None,
        };
        #[cfg(target_os = "macos")]
        app.attach_menu(&cc.egui_ctx);
        app.volume = setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7);
        app.state.lock().unwrap().unplayable = load_unplayable();
        app.state.lock().unwrap().added_to = setting("added_to").map(|v| v.split(',').map(String::from).collect()).unwrap_or_default();
        #[cfg(not(target_os = "windows"))] // souvlaki panics on Windows without the window's HWND
        app.attach_media(&cc.egui_ctx);
        match api::client_id(&resona_dir()) {
            Some(_) => app.start(&cc.egui_ctx),
            None => app.setup = Some(String::new()),
        }
        app
    }

    /// Signs in and connects, in the background.
    fn start(&self, ctx: &egui::Context) {
        let (state, slot, ctx) = (self.state.clone(), self.engine.clone(), ctx.clone());
        self.rt.spawn(async move {
            if let Err(e) = connect(state.clone(), slot, ctx.clone()).await {
                state.lock().unwrap().error = Some(e);
                ctx.request_repaint();
            }
        });
    }

    fn open(&mut self, src: Source, ctx: &egui::Context) {
        if self.state.lock().unwrap().view.as_ref() != Some(&src) { self.row = 0 } // a new list: cursor to its top
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
        let (state, session) = (self.state.clone(), e.session());
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

    /// Draws the toast until it runs out: hovering holds it, a click closes it.
    fn toast(&self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        if s.notice.as_ref().is_some_and(|n| n.until <= now) { s.notice = None }
        let Some(n) = &mut s.notice else { return };
        let mut r = skin::toast(ctx, &ui::short_err(&n.text), n.error, (n.until - now).as_secs_f32() / n.life.as_secs_f32());
        if n.error { r = r.on_hover_text(&n.text) } // the full error
        if r.hovered() { n.until = n.until.max(now + Duration::from_secs(1)) }
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { s.notice = None }
        ctx.request_repaint_after(Duration::from_millis(250)); // drains the bar, then clears it
    }

    /// Appends a track to one of your playlists, then says so in a toast.
    fn add_to(&self, playlist: String, name: String, track: String, ctx: &egui::Context) {
        let Some(e) = self.engine.get() else { return };
        let (state, web, http, ctx) = (self.state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
        self.rt.spawn(async move {
            let done = api::add_to_playlist(&web, &http, &playlist, &track).await;
            let mut s = state.lock().unwrap();
            let order = match done {
                Ok(()) => {
                    s.remember_added_to(&playlist);
                    s.members.entry(playlist).or_default().insert(track);
                    s.say(format!("Added to {name}"), false);
                    Some(s.added_to.join(","))
                }
                Err(err) => { s.say(format!("Not added to {name}: {err}"), true); None }
            };
            drop(s); // no disk writes under the lock the frame needs
            if let Some(order) = order { set_setting("added_to", &order) }
            ctx.request_repaint(); // nothing else redraws an idle window
        });
    }

    /// Makes a playlist, puts it on top of the sidebar, and adds `track` to it if one came along.
    pub(crate) fn new_playlist(&self, name: String, track: Option<String>, ctx: &egui::Context) {
        let Some(e) = self.engine.get() else { return };
        let (state, web, http, ctx) = (self.state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
        self.rt.spawn(async move {
            let made = api::create_playlist(&web, &http, &name).await;
            let added = match (&made, &track) {
                (Ok(pl), Some(t)) => api::add_to_playlist(&web, &http, &pl.id, t).await,
                _ => Ok(()),
            };
            let mut s = state.lock().unwrap();
            let order = match made {
                Ok(pl) => {
                    s.members.entry(pl.id.clone()).or_default().extend(track.filter(|_| added.is_ok()));
                    s.remember_added_to(&pl.id);
                    s.playlists.insert(0, pl);
                    match added {
                        Ok(()) => s.say(format!("Created {name}"), false),
                        Err(err) => s.say(format!("Created {name}, but the song wasn't added: {err}"), true),
                    }
                    Some(s.added_to.join(","))
                }
                Err(err) => { s.say(format!("Couldn't create {name}: {err}"), true); None }
            };
            drop(s); // no disk writes under the lock the frame needs
            if let Some(order) = order { set_setting("added_to", &order) }
            ctx.request_repaint(); // nothing else redraws an idle window
        });
    }

    /// Starts reading what's in your own playlists, once, for the ticks in "Add to playlist".
    fn load_members(&self) {
        let Some(e) = self.engine.get() else { return };
        if std::mem::replace(&mut self.state.lock().unwrap().members_asked, true) { return }
        self.rt.spawn(load_members(self.state.clone(), e.web.clone(), e.http.clone(), e.session().username()));
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
        match self.engine.get() {
            Some(_) => self.command(Cmd::Shuffle),
            None => self.state.lock().unwrap().toggle_shuffle(), // not signed in yet: nothing to start
        }
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
        (s.view, s.loading, s.error, s.expected, s.artist) = (Some(src.clone()), true, None, None, None);
        s.load_id += 1;
        s.tracks.clear();
        // Before any song arrives: pages are sorted as they land. A radio keeps Spotify's mix order.
        s.sort = if matches!(src, Source::Radio { .. }) { Sort::List } else { Sort::saved(&src) };
        if src == Source::Queue {
            s.loading = false; // drawn straight from the play queue
            return;
        }
    }
    if let Some(key) = src.key() { set_setting("last", &key) }
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    let me = e.session().username();
    let foreign = match &src {
        Source::Playlist(id) => state.lock().unwrap().playlists.iter().any(|p| &p.id == id && !p.web_readable(&me)),
        Source::Radio { .. } => true,
        _ => false,
    };
    if foreign {
        let (session, sort, me) = { let s = state.lock().unwrap(); (e.session(), s.sort, s.load_id) };
        rt.spawn(async move {
            let id = match &src {
                Source::Radio { track, .. } => api::radio_playlist(&session, track).await,
                Source::Playlist(id) => Ok(id.clone()),
                _ => unreachable!(),
            };
            let done = async { api::session_playlist(&session, &id?, sort, |tracks, total| {
                let mut s = state.lock().unwrap();
                if s.load_id != me { return false } // user moved on, or opened it again
                s.expected = Some(total);
                s.add_tracks(tracks);
                ctx.request_repaint();
                true
            }).await }.await;
            {
                let mut s = state.lock().unwrap();
                if s.load_id == me {
                    (s.loading, s.error) = (false, done.err());
                }
            }
            ctx.request_repaint();
        });
        return;
    }
    if src == Source::NewReleases {
        // Who you follow (Web API), then each one's last two weeks (session), streamed in release by release.
        let (session, me) = (e.session(), state.lock().unwrap().load_id);
        rt.spawn(async move {
            let since = (std::time::SystemTime::now() - Duration::from_secs(14 * 86_400)).duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            let done = match api::followed_artists(&web, &http).await {
                Ok(artists) => {
                    api::session_new_releases(&session, &artists, since, |tracks| {
                        let mut s = state.lock().unwrap();
                        if s.load_id != me { return false } // user moved on, or opened it again
                        s.add_tracks(tracks);
                        ctx.request_repaint();
                        true
                    }).await;
                    Ok(())
                }
                Err(e) => Err(e),
            };
            let mut s = state.lock().unwrap();
            if s.load_id == me { (s.loading, s.error) = (false, done.err()) }
            drop(s);
            ctx.request_repaint();
        });
        return;
    }
    if matches!(src, Source::Artist { .. } | Source::Album { .. }) {
        // Artist and album pages come through the session, like the official client.
        let session = e.session();
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
                Ok((tracks, page)) => {
                    s.artist = page;
                    s.add_tracks(tracks.into_iter().enumerate().map(|(pos, t)| api::Track { pos, ..t }).collect());
                }
                Err(e) => s.error = Some(e),
            }
            s.loading = false;
            ctx.request_repaint();
        });
        return;
    }
    rt.spawn(async move {
        let (edits, sort) = { let s = state.lock().unwrap(); (s.liked_edits, s.sort) };
        // Pages come in list order, except when the sort puts the list's end on top: Liked Songs is newest
        // first already, playlists oldest first. Then a one-song page asks for the length, and pages are read
        // from the end, so the top of the list fills in first.
        let (page_url, size) = match &src {
            Source::Liked => ("me/tracks?limit=".to_string(), 50),
            Source::Playlist(id) => (format!("playlists/{id}/items?limit="), 100),
            _ => (String::new(), 0),
        };
        let backwards = matches!((&src, sort), (Source::Liked, Sort::Oldest) | (Source::Playlist(_), Sort::Newest));
        let mut back: Vec<usize> = Vec::new(); // page offsets still to read backwards; the last one first
        let mut url = Some(match &src {
            Source::Search(q) => api::search_url(q),
            Source::Liked | Source::Playlist(_) => format!("{page_url}{}", if backwards { 1 } else { size }),
            Source::Artist { .. } | Source::Album { .. } | Source::Radio { .. } | Source::NewReleases | Source::Queue => return,
        });
        let mut pages = 0;
        while let Some(u) = url {
            let page = match &src {
                Source::Search(_) => api::get::<api::Search>(&web, &http, &u).await
                    .map(|r| (r.tracks.items, r.tracks.next, r.tracks.total.map(|t| t.min(50)), r.tracks.offset)),
                _ => api::get::<api::Page<api::Entry>>(&web, &http, &u).await
                    .map(|p| (p.items.into_iter().filter_map(api::Entry::playable).collect(), p.next, p.total, p.offset)),
            };
            let mut s = state.lock().unwrap();
            if s.view.as_ref() != Some(&src) { return } // user moved on
            pages += 1;
            match page {
                Ok((tracks, next, total, offset)) => {
                    s.expected = s.expected.or(total);
                    if backwards && pages == 1 {
                        back = (0..total.unwrap_or(0)).step_by(size).collect(); // only asked for the length
                    } else {
                        // ponytail: a song Spotify can't play here drops out of the page, so later positions
                        // shift by one; only ties between same-date songs could notice.
                        s.add_tracks(tracks.into_iter().enumerate().map(|(i, t)| api::Track { pos: offset + i, ..t }).collect());
                    }
                    url = if backwards {
                        back.pop().map(|o| format!("{page_url}{size}&offset={o}"))
                    } else {
                        // Search pages are small; 5 of them is plenty, and keeps us off the rate limit.
                        next.filter(|_| !matches!(src, Source::Search(_)) || pages < 5)
                    };
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
        if !list.is_empty() { s.start(&e.player(), list, None, src) }
    };
    match cmd {
        Cmd::Toggle if s.now.is_none() => start_open_list(s),
        Cmd::Toggle => if s.since.is_some() { e.player().pause() } else { e.player().play() },
        Cmd::Play if s.now.is_none() => start_open_list(s),
        Cmd::Play => e.player().play(),
        Cmd::Pause => e.player().pause(),
        Cmd::Next => { let from = s.qi; s.step(&e.player(), from, 1); }
        Cmd::Prev => {
            // Like every player: restart the song unless we're at its very start.
            let from = s.qi;
            if pos > 3000 || !s.step(&e.player(), from, -1) { e.player().seek(0) }
        }
        Cmd::SeekTo(ms) => e.player().seek(ms),
        Cmd::Shuffle => {
            s.toggle_shuffle();
            if s.shuffle && s.now.is_none() { start_open_list(s) }
        }
        Cmd::SeekBy(d) => {
            let dur = s.now.as_ref().map_or(0, |t| t.duration_ms) as i64;
            e.player().seek((pos as i64 + d).clamp(0, dur.max(1) - 1) as u32);
        }
    }
}

/// Elsewhere winit shows the cursor egui asks for.
#[cfg(not(target_os = "macos"))]
fn show_cursor(_: &egui::Context) {}

/// Shows the cursor egui asked for this frame. egui-winit asks too, but winit 0.30 changes the cursor through
/// AppKit cursor rects, and on macOS 27 the hand over buttons never showed that way.
#[cfg(target_os = "macos")]
fn show_cursor(ctx: &egui::Context) {
    use objc2_app_kit::NSCursor;
    if !ctx.input(|i| i.focused && i.pointer.has_pointer()) { return } // not ours to set: another app is in front, or the pointer is elsewhere
    // The arrow is set once, to take our hand back: set every frame it would fight the cursors AppKit
    // shows by itself (the window's resize edges).
    static OURS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let cursor = match ctx.output(|o| o.cursor_icon) {
        egui::CursorIcon::PointingHand => NSCursor::pointingHandCursor(),
        egui::CursorIcon::Text => NSCursor::IBeamCursor(),
        egui::CursorIcon::Default if OURS.swap(false, std::sync::atomic::Ordering::Relaxed) => NSCursor::arrowCursor(),
        _ => return, // the rest (the sidebar's resize arrows) stays with winit
    };
    if cursor != NSCursor::arrowCursor() { OURS.store(true, std::sync::atomic::Ordering::Relaxed) }
    cursor.set();
}

impl eframe::App for App {
    /// Tab switches panes. Left to egui it would hand focus to a button, and a focused widget takes the
    /// plain-key shortcuts (and answers Space and Enter itself).
    fn raw_input_hook(&mut self, _: &egui::Context, raw: &mut egui::RawInput) {
        raw.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::Tab, pressed, repeat, .. } => {
                self.tab |= *pressed && !*repeat;
                false
            }
            _ => true,
        });
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.restore(ctx);
        self.sync_media();
        #[cfg(target_os = "macos")]
        self.menu_events(ctx);
        self.keys(ctx);
        self.window_title(ctx);
        if self.icon_accent != Some(skin::accent(ctx)) {
            self.icon_accent = Some(skin::accent(ctx));
            set_app_icon(ctx, icon_ink(skin::accent(ctx)));
        }
        self.bars_shown.set(false);
        if let Some(typed) = &mut self.setup {
            let ground = egui::Frame::new().fill(skin::pal(ctx).ground);
            let error = self.state.lock().unwrap().error.clone();
            let save = egui::CentralPanel::default().frame(ground).show(ctx, |ui| ui::setup(ui, typed, engine::REDIRECT, error.as_deref())).inner;
            if save {
                match api::save_client_id(&resona_dir(), typed) {
                    Ok(()) => { self.setup = None; self.start(ctx) }
                    Err(e) => self.state.lock().unwrap().error = Some(format!("Couldn't save the client ID: {e}")),
                }
            }
            show_cursor(ctx);
            return;
        }
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
        self.toast(ctx);
        self.reveal = false;
        show_cursor(ctx);

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
    // librespot says why a song failed only in its log: `RUST_LOG=warn ./resona 2>resona.log`.
    env_logger::init();
    if std::env::args().nth(1).as_deref() == Some("cli") {
        cli::run();
        return Ok(());
    }
    // `resona icon`: the 1024px icon in the saved accent, raw RGBA on stdout, for bundle.sh to wrap as a PNG.
    if std::env::args().nth(1).as_deref() == Some("icon") {
        use std::io::Write as _;
        let _ = std::io::stdout().write_all(&skin::app_icon(1024, icon_ink(saved_accent())));
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

/// The icon's ink: the accent's dark-theme shade (the icon is dark).
fn icon_ink(a: skin::Accent) -> egui::Color32 {
    skin::pal_of(skin::Skin::Wired, egui::Theme::Dark, a).accent
}

/// Windows and Linux: the window's icon (title bar, taskbar), which winit can change at any time.
#[cfg(not(target_os = "macos"))]
fn set_app_icon(ctx: &egui::Context, ink: egui::Color32) {
    const N: u32 = 128;
    let icon = egui::IconData { rgba: skin::app_icon(N as usize, ink), width: N, height: N };
    ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon))));
}

/// The Dock icon in `ink`. eframe sets an icon only once, at startup, so a new accent repaints it here.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn set_app_icon(_: &egui::Context, ink: egui::Color32) {
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage};
    use objc2_foundation::NSSize;
    const N: usize = 1024; // as sharp as the bundle's own icon in a big Retina Dock
    let Some(mtm) = MainThreadMarker::new() else { return };
    let rgba = skin::app_icon(N, ink); // alpha is 0 or 255, so already premultiplied as AppKit expects
    // SAFETY: null planes make the rep allocate and own its pixel buffer (N*N*4 bytes, as described); ours is
    // copied into it, so nothing it keeps points at memory we free.
    unsafe {
        let Some(rep) = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), N as isize, N as isize, 8, 4, true, false, NSDeviceRGBColorSpace, (N * 4) as isize, 32,
        ) else { return };
        let data = rep.bitmapData();
        // The buffer must be exactly what we asked for: unpadded rows of N RGBA pixels.
        if data.is_null() || rep.bytesPerRow() != (N * 4) as isize || rgba.len() != N * N * 4 { return }
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len());
        let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(N as f64, N as f64));
        image.addRepresentation(&rep);
        NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
    }
}
