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
mod update;

use std::{
    collections::HashSet,
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
    /// The lists opened before this one, latest last: Esc, ⌘[ and the mouse's back button return to them.
    back: Vec<Source>,
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
    media_sent: controls::MediaSent,
    /// First launch without a client ID: what's typed into the setup screen. Nothing connects until it's saved.
    setup: Option<String>,
    /// The accent the Dock icon was last drawn in.
    icon_accent: Option<skin::Accent>,
    /// The macOS menu bar (kept alive here) and its clicks.
    #[cfg(target_os = "macos")]
    menu: Option<muda::Menu>,
    #[cfg(target_os = "macos")]
    menu_rx: std::sync::mpsc::Receiver<muda::MenuEvent>,
    /// The update downloading and installing after Update was clicked (`update::install`).
    updating: Option<(std::thread::JoinHandle<Result<(), String>>, update::Progress)>,
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
            back: Vec::new(),
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
            updating: None,
        };
        #[cfg(target_os = "macos")]
        app.attach_menu(&cc.egui_ctx);
        app.volume = setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7);
        app.state.lock().unwrap().unplayable = load_unplayable();
        app.state.lock().unwrap().load_modes();
        app.state.lock().unwrap().added_to = setting("added_to").map(|v| v.split(',').map(String::from).collect()).unwrap_or_default();
        app.attach_media(cc);
        match api::client_id(&resona_dir()) {
            Some(_) => app.start(&cc.egui_ctx),
            None => app.setup = Some(String::new()),
        }
        let (state, ctx) = (app.state.clone(), cc.egui_ctx.clone());
        app.rt.spawn(async move {
            let Some(tag) = update::check(&reqwest::Client::new()).await else { return };
            state.lock().unwrap().update = Some(tag);
            ctx.request_repaint();
        });
        app
    }

    /// The update dialog while a newer release is on offer, and install.sh once Update was clicked.
    fn offer_update(&mut self, ctx: &egui::Context) {
        let Some(tag) = self.state.lock().unwrap().update.clone() else { return };
        if self.updating.as_ref().is_some_and(|u| u.0.is_finished()) {
            let done = self.updating.take().unwrap().0.join().unwrap_or_else(|_| Err("the update stopped".into()));
            match done {
                Ok(()) => {
                    update::reopen();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(why) => {
                    let mut s = self.state.lock().unwrap();
                    s.update = None;
                    s.say(format!("Couldn't update: {why}"), true);
                }
            }
            return;
        }
        if self.updating.is_some() { ctx.request_repaint_after(Duration::from_millis(100)) }
        let got = self.updating.as_ref().map(|u| update::progress(&u.1));
        let in_place = update::in_place();
        match ui::update_dialog(ctx, &tag, got, in_place) {
            Some(true) if in_place => self.updating = Some(update::install(&tag, self.rt.handle().clone())),
            Some(true) => {
                ctx.open_url(egui::OpenUrl::new_tab(update::page(&tag)));
                self.state.lock().unwrap().update = None;
            }
            Some(false) => self.state.lock().unwrap().update = None,
            None => {}
        }
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
        let Some(e) = self.engine.get() else { return };
        let view = self.state.lock().unwrap().view.clone();
        if view.as_ref() == Some(&src) { return }
        self.back.extend(view);
        self.row = 0; // a new list: cursor to its top
        open_list(self.rt.handle(), e, &self.state, src, ctx);
    }

    /// Back to the list open before this one, if there was one.
    fn go_back(&mut self, ctx: &egui::Context) {
        let (Some(e), Some(src)) = (self.engine.get(), self.back.pop()) else { return };
        (self.big, self.row) = (false, 0);
        open_list(self.rt.handle(), e, &self.state, src, ctx);
    }

    fn command(&mut self, cmd: Cmd) {
        if let Some(e) = self.engine.get() { command(e, &mut self.state.lock().unwrap(), cmd) }
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(e) = self.engine.get() { e.mixer.set_volume((self.volume * u16::MAX as f32) as u16) }
    }

    fn fetch_lyrics(&self, s: &mut State) {
        if let Some(e) = self.engine.get() { fetch_lyrics(self.rt.handle(), e, &self.state, s) }
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

    fn add_to(&self, playlist: String, name: String, track: String, add: bool, ctx: &egui::Context) {
        if let Some(e) = self.engine.get() { add_to(self.rt.handle(), e, &self.state, playlist, name, track, add, ctx) }
    }

    pub(crate) fn new_playlist(&self, name: String, track: Option<String>, ctx: &egui::Context) {
        if let Some(e) = self.engine.get() { new_playlist(self.rt.handle(), e, &self.state, name, track, ctx) }
    }

    fn load_members(&self) {
        if let Some(e) = self.engine.get() { ask_members(self.rt.handle(), e, &self.state) }
    }

    fn toggle_like(&self, id: String) {
        if let Some(e) = self.engine.get() { toggle_like(self.rt.handle(), e, &self.state, id) }
    }

    /// Once the engine is up: the saved volume, and the list that was open last time.
    fn restore(&mut self, ctx: &egui::Context) {
        if self.restored || self.engine.get().is_none() { return }
        self.restored = true;
        self.set_volume(self.volume);
        if let Some(e) = self.engine.get() { resume(e, &mut self.state.lock().unwrap()) }
        self.open(Source::last(), ctx);
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

/// Starts fetching lyrics for the playing track, once per track. Spotify's lyrics service goes
/// through the librespot session (the Web API has none).
pub fn fetch_lyrics(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, s: &mut State) {
    let Some(id) = s.now.as_ref().and_then(|t| t.id.clone()) else { return };
    if s.lyrics.as_ref().is_some_and(|(have, _)| *have == id) { return }
    s.lyrics = Some((id.clone(), Lyrics::Loading));
    let (state, session) = (state.clone(), e.session());
    rt.spawn(async move {
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

/// Appends a track to one of your playlists (or takes it out, `add` false), then says so in a toast.
#[allow(clippy::too_many_arguments)] // one toggle more than add needed
pub fn add_to(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, playlist: String, name: String, track: String, add: bool, ctx: &egui::Context) {
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    rt.spawn(async move {
        if !add {
            let done = api::remove_from_playlist(&web, &http, &playlist, &track).await;
            let mut s = state.lock().unwrap();
            match done {
                Ok(()) => {
                    if s.view == Some(Source::Playlist(playlist.clone())) { s.tracks.retain(|t| t.id.as_ref() != Some(&track)) }
                    s.members.entry(playlist).or_default().remove(&track);
                    s.say(format!("Removed from {name}"), false);
                }
                Err(err) => s.say(format!("Not removed from {name}: {err}"), true),
            }
            drop(s);
            ctx.request_repaint();
            return;
        }
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
pub fn new_playlist(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, name: String, track: Option<String>, ctx: &egui::Context) {
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    rt.spawn(async move {
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
pub fn ask_members(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared) {
    if std::mem::replace(&mut state.lock().unwrap().members_asked, true) { return }
    rt.spawn(load_members(state.clone(), e.web.clone(), e.http.clone(), e.session().username()));
}

/// Shows the copy of this list kept from last time, if it's still at `version`: no song requests at all.
fn show_kept(state: &Shared, key: &str, version: &str, me: u64, ctx: &egui::Context) -> bool {
    let Some((_, tracks)) = load_list(key).filter(|(v, _)| v == version) else { return false };
    let mut s = state.lock().unwrap();
    if s.load_id == me {
        s.add_tracks(tracks);
        s.loading = false;
    }
    ctx.request_repaint();
    true
}

/// Liked Songs from the copy kept last time, when it's out of date: unliked songs dropped, the new ones read
/// from the newest page on until every liked id is there (usually one page, instead of all ~90). False when
/// there's no copy or Spotify can't say what's liked: the caller reads the whole list.
async fn liked_delta(session: &librespot::core::Session, web: &api::WebAuth, http: &reqwest::Client, state: &Shared, key: &str, me: u64, edits: u32) -> bool {
    let Ok(ids) = api::session_liked(session).await else { return false };
    let Some((v, kept)) = load_list(key) else { return false };
    let version = api::liked_version(&ids);
    let tracks = if v == version { kept } else {
        let mut missing: HashSet<&String> = ids.iter().collect();
        for t in &kept { if let Some(id) = &t.id { missing.remove(id); } }
        let mut read = Vec::new();
        let mut url = Some("me/tracks?limit=50".to_string());
        while let Some(u) = url.take().filter(|_| !missing.is_empty()) {
            let Ok(p) = api::get::<api::Page<api::Entry>>(web, http, &u).await else { return false };
            let offset = p.offset;
            for (i, t) in p.items.into_iter().filter_map(api::Entry::playable).enumerate() {
                if let Some(id) = &t.id { missing.remove(id); }
                read.push(api::Track { pos: offset + i, ..t });
            }
            url = p.next;
        }
        merge_liked(kept, read, &ids)
    };
    let mut s = state.lock().unwrap();
    if s.load_id != me { return true } // user moved on
    s.add_tracks(tracks);
    s.loading = false;
    if v != version { save_list(key, &version, &s.tracks) }
    if s.liked_edits == edits {
        s.liked = s.tracks.iter().filter_map(|t| t.id.clone()).collect();
        s.liked_complete = true;
        save_liked_ids(&s.liked);
    }
    true
}

/// The newest pages just read, then the kept copy without them and without what's no longer liked, numbered on.
fn merge_liked(mut kept: Vec<api::Track>, read: Vec<api::Track>, ids: &HashSet<String>) -> Vec<api::Track> {
    let fresh: HashSet<&String> = read.iter().filter_map(|t| t.id.as_ref()).collect();
    kept.retain(|t| t.id.as_ref().is_some_and(|id| ids.contains(id) && !fresh.contains(id)));
    kept.sort_by_key(|t| t.pos);
    let from = read.len();
    read.into_iter().chain(kept.into_iter().enumerate().map(|(i, t)| api::Track { pos: from + i, ..t })).collect()
}

#[test]
fn merge_liked_puts_new_songs_first_and_drops_unliked() {
    let t = |id: &str, pos| api::Track { id: Some(id.into()), pos, ..Default::default() };
    let ids: HashSet<String> = ["n", "a", "c"].map(String::from).into();
    let got = merge_liked(vec![t("a", 0), t("b", 1), t("c", 2)], vec![t("n", 0), t("a", 1)], &ids);
    let got: Vec<_> = got.iter().map(|t| (t.id.as_deref().unwrap(), t.pos)).collect();
    assert_eq!(got, [("n", 0), ("a", 1), ("c", 2)]);
}

/// Switches the main view and streams that list in, page by page.
pub fn open_list(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, src: Source, ctx: &egui::Context) {
    {
        let mut s = state.lock().unwrap();
        if s.view.as_ref() == Some(&src) { return }
        (s.view, s.loading, s.error, s.expected, s.artist, s.found) = (Some(src.clone()), true, None, None, None, None);
        s.load_id += 1;
        s.tracks.clear();
        // Before any song arrives: pages are sorted as they land. A radio keeps Spotify's mix order.
        s.sort = if matches!(src, Source::Radio { .. }) { Sort::List } else { Sort::saved(&src) };
        if src == Source::Queue {
            s.loading = false; // drawn straight from the play queue
            return;
        }
    }
    // Reopened at launch, before the sidebar is in: only a list that will have its name then.
    let visiting = matches!(&src, Source::Playlist(id) if !state.lock().unwrap().playlists.iter().any(|p| &p.id == id));
    // A search isn't a list to come back to: launch reopens the last list instead.
    let search = matches!(src, Source::Search(_));
    if let Some(key) = src.key().filter(|_| !visiting && !search) { set_setting("last", &key) }
    // Back to the last search: its results again, not a new request.
    if let Source::Search(q) = &src {
        let mut s = state.lock().unwrap();
        if let Some((.., tracks, found)) = s.searched.clone().filter(|(k, at, ..)| k == q && at.elapsed() < Duration::from_secs(600)) {
            s.add_tracks(tracks);
            (s.found, s.loading) = (Some(found), false);
            return;
        }
    }
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    let me = e.session().username();
    let foreign = match &src {
        Source::Playlist(id) => state.lock().unwrap().playlist(id).is_some_and(|p| !p.web_readable(&me)),
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
            let key = src.key().unwrap_or_default();
            let version = api::list_version(&session, &src).await;
            if version.as_deref().is_some_and(|v| show_kept(&state, &key, v, me, &ctx)) { return }
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
                    if let (Ok(()), Some(v)) = (&done, &version) { save_list(&key, v, &s.tracks) }
                    (s.loading, s.error) = (false, done.err());
                }
            }
            ctx.request_repaint();
        });
        return;
    }
    if src == Source::NewReleases {
        // Who you follow (session, else Web API), then each one's last two weeks (web player, else session), streamed in release by release.
        let (session, me) = (e.session(), state.lock().unwrap().load_id);
        rt.spawn(async move {
            let since = (std::time::SystemTime::now() - Duration::from_secs(14 * 86_400)).duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            // The session's list (good through a Web API ban), else the Web API's, else the last list we got.
            let artists = match api::session_following(&session).await {
                Some(a) => Ok(a),
                None => api::followed_artists(&web, &http).await,
            };
            let artists = match artists {
                Ok(a) => { settings::save_following(&a); Ok(a) }
                Err(e) => settings::load_following().map(|a| a.into_iter().collect()).ok_or(e),
            };
            let done = match artists {
                Ok(artists) => {
                    api::session_new_releases(&session, &http, &artists, since, |tracks| {
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
                Source::Artist { id, .. } => match tokio::join!(api::session_artist(&session, &http, id), api::artist_overview(&session, &http, id),
                    // Bounded: a rate-limit wait in `api::get` would hold the page; the file answers instead.
                    tokio::time::timeout(Duration::from_secs(3), api::is_followed(&web, &http, id))) {
                    (Ok((portrait, popularity, top, releases, latest)), overview, followed) => {
                        // Follow: what Spotify says now (and the file learns it), else the file from last time.
                        let followed = match followed {
                            Ok(Ok(on)) => { settings::set_following(id, on); on }
                            _ => settings::load_following().is_some_and(|f| f.contains(id.as_str())),
                        };
                        // No photo in the session's artist record: ask the Web API (middle-sized image).
                        let portrait = match portrait {
                            Some(p) => Some(p),
                            None => api::get::<api::ArtistImages>(&web, &http, &format!("artists/{id}")).await.ok()
                                .and_then(|a| a.images.get(1).or(a.images.first()).map(|i| i.url.clone())),
                        };
                        let n = top.len();
                        Ok((top.into_iter().chain(latest).collect(), Some(ArtistPage { portrait, popularity, overview, releases, top: n, followed })))
                    }
                    (Err(e), ..) => Err(e),
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
    let session = e.session();
    rt.spawn(async move {
        let (edits, sort, me) = { let s = state.lock().unwrap(); (s.liked_edits, s.sort, s.load_id) };
        // Search: the web player's, with artists, albums and playlists; the Web API's (songs only) if it doesn't answer.
        if let Source::Search(q) = &src && let Some((tracks, found)) = api::web_search(&session, &http, q).await {
            let mut s = state.lock().unwrap();
            if s.load_id != me { return } // user moved on
            let tracks: Vec<api::Track> = tracks.into_iter().enumerate().map(|(pos, t)| api::Track { pos, ..t }).collect();
            s.searched = Some((q.clone(), Instant::now(), tracks.clone(), found.clone()));
            s.add_tracks(tracks);
            (s.found, s.loading) = (Some(found), false);
            ctx.request_repaint();
            return;
        }
        let key = src.key().unwrap_or_default();
        if src == Source::Liked && liked_delta(&session, &web, &http, &state, &key, me, edits).await {
            ctx.request_repaint();
            return;
        }
        let version = api::list_version(&session, &src).await;
        if version.as_deref().is_some_and(|v| show_kept(&state, &key, v, me, &ctx)) { return }
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
        if let (None, Some(v), true) = (&s.error, &version, s.load_id == me) { save_list(&key, v, &s.tracks) }
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

/// Saves the playlist opened from search to your library; it joins the sidebar once Spotify says yes.
pub fn save_playlist(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, ctx: &egui::Context) {
    let Some(p) = state.lock().unwrap().visiting.clone() else { return };
    let (state, web, http, ctx) = (state.clone(), e.web.clone(), e.http.clone(), ctx.clone());
    rt.spawn(async move {
        let done = api::save_playlist(&web, &http, &p.id).await;
        let mut s = state.lock().unwrap();
        match done {
            Ok(()) => {
                s.say(format!("Saved {} to your library", p.name), false);
                if !s.playlists.iter().any(|l| l.id == p.id) { s.playlists.insert(0, p) }
            }
            Err(err) => s.say(format!("Not saved: {err}"), true),
        }
        drop(s);
        ctx.request_repaint();
    });
}

/// Follows or unfollows an artist: the button flips at once, and flips back if Spotify says no.
pub fn toggle_follow(rt: &tokio::runtime::Handle, e: &Engine, state: &Shared, id: String) {
    let artist = id.clone();
    let flip = move |state: &Shared| {
        let mut s = state.lock().unwrap();
        let a = s.artist.as_mut()?;
        a.followed = !a.followed;
        settings::set_following(&id, a.followed); // the list New releases reads during a ban: keep it in step
        Some(a.followed)
    };
    let Some(on) = flip(state) else { return };
    let (state, web, http) = (state.clone(), e.web.clone(), e.http.clone());
    rt.spawn(async move {
        if let Err(err) = api::set_followed(&web, &http, &artist, on).await {
            // ponytail: flips back whatever artist page is open now; fine unless you switch pages within the request.
            flip(&state);
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

/// At launch: the queue and song from last time (`save_resume`), loaded paused where it was left.
pub fn resume(e: &Engine, s: &mut State) {
    let Some(r) = load_resume() else { return };
    let Some(t) = r.queue.get(r.qi).cloned() else { return };
    let Ok(id) = SpotifyId::from_base62(track_id(t.id.as_deref().unwrap_or_default())) else { return };
    s.base = r.base();
    (s.queue, s.qi, s.queued, s.shuffle) = (r.queue, r.qi, r.queued, r.shuffle);
    s.queue_src = r.src.as_deref().and_then(Source::from_key);
    (s.now, s.pos_ms, s.since) = (Some(t), r.ms, None);
    // The shuffle setting wins: the queue may predate a later toggle (unset: the queue's own, as before).
    if let Some(on) = setting("shuffle").map(|v| v == "on").filter(|&on| on != s.shuffle) { s.set_shuffle(on) }
    e.player().load(librespot::core::SpotifyUri::Track { id }, false, r.ms);
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

    /// Where playback is now, for the next launch: while playing, the last save is a song start or a pause behind.
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        let saving = save_resume(&self.state.lock().unwrap());
        if let Some(t) = saving { let _ = t.join(); }
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.restore(ctx);
        self.sync_media();
        #[cfg(target_os = "macos")]
        self.menu_events(ctx);
        self.keys(ctx);
        self.window_title(ctx);
        if self.icon_accent != Some(skin::kept_accent(ctx)) { // not on a preview: a 1024 px redraw each time
            self.icon_accent = Some(skin::kept_accent(ctx));
            set_app_icon(ctx, icon_ink(skin::kept_accent(ctx)));
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
        self.offer_update(ctx);
        self.toast(ctx);
        self.reveal = false;
        show_cursor(ctx);
        screenshot(ctx);

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
    let log = env_logger::Builder::from_default_env().build();
    log::set_max_level(log.filter());
    let cli = std::env::args().nth(1).as_deref() == Some("cli");
    // The CLI draws on the terminal stderr goes to: a log line there scrolls the screen under its line-diff repaint.
    let quiet = cli && std::io::IsTerminal::is_terminal(&std::io::stderr());
    let _ = log::set_boxed_logger(Box::new(Log(log, quiet)));
    if cli {
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

/// env_logger, plus one catch: librespot ends the whole process (`exit(1)`) when a free account signs in, and
/// says why only in its log. Its error comes just before the exit, so this says it to the user first.
struct Log(env_logger::Logger, bool);

impl log::Log for Log {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.0.enabled(m)
    }
    fn log(&self, r: &log::Record) {
        if !self.1 { self.0.log(r) }
        if free_account(r.target(), &r.args().to_string()) { premium_needed() }
    }
    fn flush(&self) {
        self.0.flush()
    }
}

/// librespot's last words before it quits over the account: `librespot does not support "free" accounts.`
fn free_account(target: &str, msg: &str) -> bool {
    target.starts_with("librespot_core::session") && msg.contains("does not support") && msg.contains("accounts")
}

#[test]
fn free_account_is_told_by_librespots_error() {
    assert!(free_account("librespot_core::session", r#"librespot does not support "free" accounts."#));
    assert!(!free_account("librespot_core::session", "Connecting to AP"));
    assert!(!free_account("resona", r#"librespot does not support "free" accounts."#));
}

/// Shown on the way out: the window gets a dialog (macOS) and the terminal a line.
// ponytail: Linux and Windows windows only print to stderr; a native dialog there needs a new dependency.
fn premium_needed() {
    const WHY: &str = "Resona plays through librespot, which Spotify allows for Premium accounts only, and this \
        account is on Spotify Free. To sign in with another account, delete credentials.json in ~/.resona.";
    let cli = std::env::args().nth(1).as_deref() == Some("cli");
    if cli { cli::leave_terminal() }
    #[cfg(target_os = "macos")]
    if !cli {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("display alert \"Resona needs Spotify Premium\" message \"{WHY}\" as critical")])
            .status();
    }
    eprintln!("Resona needs Spotify Premium. {WHY}");
}

/// `RESONA_SHOT=<file.png>`: the window as it looks 15 s after launch, saved as a PNG, then closed. For the
/// README's picture; egui reads its own frame, so no screen-recording permission is needed.
fn screenshot(ctx: &egui::Context) {
    static START: OnceLock<Instant> = OnceLock::new();
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(path) = std::env::var_os("RESONA_SHOT") else { return };
    let start = *START.get_or_init(Instant::now);
    let shot = ctx.input(|i| i.raw.events.iter().find_map(|e| match e {
        egui::Event::Screenshot { image, .. } => Some(image.clone()),
        _ => None,
    }));
    if let Some(img) = shot {
        let px = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let saved = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, px).map(|i| i.save(&path));
        if !matches!(saved, Some(Ok(()))) { eprintln!("couldn't save the screenshot to {}", path.to_string_lossy()) }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    } else if start.elapsed() > Duration::from_secs(15) && !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    ctx.request_repaint_after(Duration::from_millis(500)); // keep frames coming while it waits
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
