//! `resona cli`: the player full-screen in a terminal, dressed like flow (a centred card of titled boxes, Wired's red).
//! Shares the engine, `open_list` and `command` with the window, so it plays the same queue the same way.

use std::{
    io::Write,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use eframe::egui;
use librespot::playback::mixer::Mixer;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{add_to, api, ask_members, command, controls::*, engine::*, fetch_lyrics, new_playlist, open_list, settings::*, state::*, toggle_like, ui::{album_of, artist_of, day_label, own_lists, short_err}};

type C = (u8, u8, u8);
const BG: C = (0x1a, 0x1d, 0x23);
const RULE: C = (0x2c, 0x31, 0x3a);
const PANEL: C = (0x1e, 0x21, 0x28);
const FG: C = (0xc9, 0xd1, 0xd9);
/// The playing song's name and ▶ on the selected (red) row, where red on red would vanish.
const BRIGHT: C = (0xf4, 0xf6, 0xf8);
const DIM: C = (0x6b, 0x72, 0x80);
const SUB: C = (0x5f, 0x67, 0x73);
const FAINT: C = (0x4a, 0x51, 0x5b);
const GUTTER: C = (0x34, 0x3a, 0x44);

/// The accent (red unless another is picked in the window), read once at start.
fn red() -> C {
    static ACCENT: std::sync::OnceLock<C> = std::sync::OnceLock::new();
    *ACCENT.get_or_init(|| {
        let accent = if cfg!(test) { Default::default() } else { saved_accent() }; // tests don't read your settings
        let c = crate::skin::pal_of(crate::skin::Skin::Wired, egui::Theme::Dark, accent).accent;
        (c.r(), c.g(), c.b())
    })
}

/// The selected row: clearly lit in the focused pane (`cursor`), softer in the other (`dull`): the accent over the
/// background, a quarter and a tenth of the way.
fn tint(share: f32) -> C {
    let (a, b) = (red(), BG);
    let mix = |x: u8, y: u8| (y as f32 + (x as f32 - y as f32) * share).round() as u8;
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}
fn cursor() -> C { tint(0.25) }
fn dull() -> C { tint(0.1) }

/// How many songs up next lists when a song has no lyrics.
const UP_NEXT: usize = 5;
const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The hint lines: the common keys; `?` lists them all (`ALL_KEYS`).
const KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"), ("tab", "pane"), ("enter", "play"), ("/", "search"), (".", "playing"), ("q", "quit"),
    ("space", "pause"), ("n b", "next / back"), ("←→", "seek"), ("+-", "volume"), ("l", "like"), ("a", "queue"),
    ("p", "add to"), ("v", "lyrics"), ("?", "all keys"),
];

/// The board's hint line: the keys for the pane the cursor is in.
const LIST_KEYS: &[(&str, &str)] = &[("↑↓", "move"), ("enter", "open"), ("tab", "songs"), ("/", "search"), ("q", "quit")];
const SONG_KEYS: &[(&str, &str)] = &[("enter", "play"), ("a", "queue"), ("l", "like"), ("g", "artist"), ("i", "radio"), ("v", "lyrics")];

/// `?`: every key, in groups.
const ALL_KEYS: &[(&str, &[(&str, &str)])] = &[
    ("move", &[("↑↓ j k", "move"), ("pgup pgdn", "a page"), ("home end", "top, bottom"), ("tab", "library / songs"), ("enter", "open / play"), ("/", "search"), (".", "the playing song")]),
    ("play", &[("space", "pause / play"), ("n b", "next / back"), ("← →", "seek 10 s"), ("+ -", "volume"), ("m", "mute"), ("s", "shuffle"), ("r", "repeat")]),
    ("song", &[("l", "like"), ("a", "queue"), ("A", "play next"), ("i", "song radio"), ("g", "artist"), ("G", "album"), ("p", "add to playlist"), ("c", "copy link")]),
    ("view", &[("v", "lyrics"), ("z", "now playing"), ("f", "artists, fans, shows"), ("o", "sort"), ("t", "card / board"), ("?", "these keys"), ("^L", "redraw"), ("q", "quit")]),
];

/// `t` switches: a centred card of titled boxes on the terminal's own background (like flow), or a full-width
/// painted board (like devboard). Remembered as `cli_style` in ~/.resona/settings.
#[derive(Clone, Copy, PartialEq)]
enum Style { Card, Board }

struct Tui {
    pane: Pane,
    /// Cursor and first visible row, per pane.
    side: usize,
    side_top: usize,
    row: usize,
    row_top: usize,
    /// The search being typed, while `/` is open.
    search: Option<String>,
    volume: f32,
    /// The list we last drew, so a newly opened one starts at its top.
    seen: Option<Source>,
    body_h: usize,
    style: Style,
    /// `.`: the playing song's id, to put the cursor on once its list is open.
    jump: Option<String>,
    /// The song under the cursor, so it stays there when a sorted list moves songs around (pages arriving, `o`).
    sel: Option<String>,
    /// Ctrl+L: send the whole screen next time, not only what changed.
    repaint: bool,
    /// The clock the splash and spinners run on.
    born: Instant,
    /// `m`: the volume to come back to.
    muted: Option<f32>,
    /// The list under the sidebar cursor, so the cursor stays on it when Queue comes or goes above it.
    side_src: Option<Source>,
    /// Your account name, for which playlists are yours (`p`).
    me: String,
    /// `v`: the songs box shows the playing song's lyrics. `next_row` is the cursor in up next (no lyrics),
    /// `lyr_top` the first line shown of lyrics without timings.
    lyrics: bool,
    next_row: usize,
    lyr_top: usize,
    /// `p`: adding a song to one of your playlists.
    picker: Option<Picker>,
    /// `?`: the songs box lists every key.
    all_keys: bool,
    /// `f`: the songs box lists a search's artists, albums or playlists, or an artist's fans-also-like or shows.
    cards: Option<Cards>,
    /// `z`: the playing song full-screen: its cover, the lyrics and what's next.
    focus: bool,
    /// The playing song's cover (`want_cover`).
    cover: Arc<std::sync::Mutex<Cover>>,
}

/// The playing song's cover: its URL, the picture once downloaded (`busy` while that runs), and its half-block rows
/// as last drawn, with their size, so a frame doesn't make them again.
#[derive(Default)]
struct Cover { url: String, img: Option<Arc<image::RgbImage>>, busy: bool, drawn: Option<((usize, usize), Vec<String>)> }

/// `f`: which list shows instead of the songs, and the cursor in it.
struct Cards { kind: Kind, row: usize, top: usize }

#[derive(Clone, Copy, PartialEq)]
enum Kind { Artists, Albums, Playlists, Fans, Shows }

impl Kind {
    fn name(self) -> &'static str {
        match self { Kind::Artists => "artists", Kind::Albums => "albums", Kind::Playlists => "playlists", Kind::Fans => "fans also like", Kind::Shows => "shows" }
    }
}

/// What `f` steps through on this page, in order.
fn kinds(s: &State) -> Vec<Kind> {
    let mut out = vec![];
    if let Some(f) = &s.found {
        if !f.artists.is_empty() { out.push(Kind::Artists) }
        if !f.albums.is_empty() { out.push(Kind::Albums) }
        if !f.playlists.is_empty() { out.push(Kind::Playlists) }
    }
    if let Some(o) = s.artist.as_ref().and_then(|a| a.overview.as_ref()) {
        if !o.related.is_empty() { out.push(Kind::Fans) }
        if !o.shows.is_empty() { out.push(Kind::Shows) }
    }
    out
}

/// One list's rows: name, the dim line beside it, and what Enter opens (shows open nothing).
fn cards_of(s: &State, kind: Kind) -> Vec<(String, String, Option<Source>)> {
    let artist = |c: &api::Card| (c.name.clone(), c.note.clone(), Some(Source::Artist { id: c.id.clone(), name: c.name.clone() }));
    let overview = s.artist.as_ref().and_then(|a| a.overview.as_ref());
    match (kind, &s.found) {
        (Kind::Artists, Some(f)) => f.artists.iter().map(artist).collect(),
        (Kind::Albums, Some(f)) => f.albums.iter().map(|c| (c.name.clone(), c.note.clone(), Some(Source::Album { id: c.id.clone(), name: c.name.clone() }))).collect(),
        (Kind::Playlists, Some(f)) => f.playlists.iter().map(|p| {
            let by = p.owner.as_ref().and_then(|o| o.display_name.clone()).map_or(String::new(), |n| format!("by {n}"));
            (p.name.clone(), by, Some(Source::Playlist(p.id.clone())))
        }).collect(),
        (Kind::Fans, _) => overview.map_or(vec![], |o| o.related.iter().map(artist).collect()),
        (Kind::Shows, _) => overview.map_or(vec![], |o| o.shows.iter().map(|w| {
            (format!("{} {}  {}, {}", w.day, w.year, w.venue, w.city), format!("{}{}", if w.near { "near you · " } else { "" }, w.title), None)
        }).collect()),
        _ => vec![],
    }
}

/// `p`: the song, the cursor over your playlists (the last row makes a new one), and the new one's name.
struct Picker { track: api::Track, row: usize, top: usize, name: String }

pub fn run() {
    // First launch: ask for the client ID here, before the screen goes full-screen.
    let dir = resona_dir();
    if api::client_id(&dir).is_none() {
        println!("Resona needs your own Spotify app (free): create one at https://developer.spotify.com/dashboard,");
        println!("add the Redirect URI {}, and paste its Client ID here:", crate::engine::REDIRECT);
        let mut id = String::new();
        let _ = std::io::stdin().read_line(&mut id);
        if !api::valid_client_id(id.trim()) { return eprintln!("That doesn't look like a client ID: it's 32 letters and digits.") }
        if let Err(e) = api::save_client_id(&dir, &id) { return eprintln!("Couldn't save it: {e}") }
    }
    let _term = match Term::enter() {
        Ok(t) => t,
        Err(e) => return eprintln!("resona cli needs a terminal: {e}"),
    };
    // ponytail: a headless egui context, only so the engine's repaint calls have somewhere to go.
    let ctx = egui::Context::default();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let state: Shared = Default::default();
    let slot: Arc<OnceLock<Engine>> = Default::default();
    state.lock().unwrap().unplayable = load_unplayable();
    let (st, sl, cx) = (state.clone(), slot.clone(), ctx.clone());
    rt.spawn(async move {
        if let Err(e) = connect(st.clone(), sl, cx).await { st.lock().unwrap().error = Some(e) }
    });

    let mut ui = Tui {
        pane: Pane::Songs, side: 0, side_top: 0, row: 0, row_top: 0, search: None, seen: None, body_h: 1, jump: None, sel: None, repaint: false, born: Instant::now(), muted: None, side_src: None,
        me: String::new(), lyrics: false, next_row: 0, lyr_top: 0, picker: None, all_keys: false, cards: None, focus: false, cover: Default::default(),
        style: if setting("cli_style").as_deref() == Some("board") { Style::Board } else { Style::Card },
        volume: setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7),
    };
    #[cfg(unix)]
    quit_on_hangup();
    let mut restored = false;
    // Media keys and Now Playing / MPRIS, as in the window. The events are looked for in the wait below.
    let mut media = media_controls(None, || {});
    let mut media_sent = MediaSent::default();
    let mut out = std::io::stdout();
    // What's on screen now, so each tick only sends the lines that changed (the clock, mostly).
    let (mut shown, mut shown_size): (Vec<String>, (u16, u16)) = (Vec::new(), (0, 0));
    loop {
        if let (Some(e), false) = (slot.get(), restored) {
            restored = true;
            ui.me = e.session().username();
            e.mixer.set_volume((ui.volume * u16::MAX as f32) as u16);
            crate::resume(e, &mut state.lock().unwrap());
            open_list(rt.handle(), e, &state, Source::last(), &ctx);
        }
        let (w, h) = terminal::size().unwrap_or((80, 24));
        if (w, h) != shown_size || std::mem::take(&mut ui.repaint) { (shown, shown_size) = (Vec::new(), (w, h)) } // resize or Ctrl+L: everything
        {
            let mut s = state.lock().unwrap();
            // The queue view is drawn straight from the play queue: the playing song, then what's next.
            s.sync_queue_view();
            if let Some(e) = slot.get() {
                if ui.lyrics || ui.focus { fetch_lyrics(rt.handle(), e, &state, &mut s) }
                ui.want_cover(rt.handle(), e, &s);
            }
        }
        let lines = ui.draw(&state.lock().unwrap(), w as usize, h as usize, slot.get().is_none());
        let update = paint(&shown, &lines);
        // A failed write or read means the terminal is gone (its tab closed, with hangups ignored): quit, or the
        // music would play on with no way to stop it.
        if !update.is_empty() && out.write_all(update.as_bytes()).and_then(|_| out.flush()).is_err() { break }
        shown = lines;
        // Idle costs nothing: the clock only ticks while something moves.
        // The splash at ~12 fps; spinners and the clock at 4.
        let tick = {
            let s = state.lock().unwrap();
            if slot.get().is_none() && s.error.is_none() { 80 } else if s.loading || s.since.is_some() || ui.cover.lock().unwrap().busy { 250 } else { 2000 }
        };
        if let Some((controls, _)) = &mut media { media_show(controls, &state.lock().unwrap(), &mut media_sent) }
        // Wait for the tick or a key, looking for media keys between short naps.
        // ponytail: 10 wake-ups a second while media keys are on; a waker thread if that ever shows up.
        let until = Instant::now() + Duration::from_millis(tick);
        let key = loop {
            turn_run_loop();
            let cmds: Vec<Cmd> = media.as_ref().map(|(_, rx)| rx.try_iter().filter_map(media_cmd).collect()).unwrap_or_default();
            if !cmds.is_empty() {
                if let Some(e) = slot.get() { for c in cmds { command(e, &mut state.lock().unwrap(), c) } }
                break None;
            }
            let left = until.saturating_duration_since(Instant::now());
            match event::poll(if media.is_some() { left.min(Duration::from_millis(100)) } else { left }) {
                Ok(true) => break Some(event::read()),
                Ok(false) => {}
                Err(e) => break Some(Err(e)),
            }
            if Instant::now() >= until { break None }
        };
        let k = match key {
            Some(Ok(Event::Key(k))) => k,
            Some(Err(_)) => break, // the terminal is gone
            _ => continue, // a resize or a media key just redraws
        };
        if k.kind == KeyEventKind::Press && !ui.key(k, slot.get(), &state, rt.handle(), &ctx) { break }
    }
    let saving = save_resume(&state.lock().unwrap());
    if let Some(t) = saving { let _ = t.join(); }
}

impl Tui {
    /// One key. Returns false to quit.
    fn key(&mut self, k: KeyEvent, e: Option<&Engine>, state: &Shared, rt: &tokio::runtime::Handle, ctx: &egui::Context) -> bool {
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) { return false }
        if let Some(q) = &mut self.search {
            match k.code {
                KeyCode::Esc => self.search = None,
                KeyCode::Backspace => { q.pop(); }
                KeyCode::Char(c) if !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => q.push(c),
                KeyCode::Enter => {
                    let q = self.search.take().unwrap_or_default();
                    if let (Some(e), false) = (e, q.trim().is_empty()) {
                        open_list(rt, e, state, Source::Search(q.trim().into()), ctx);
                        self.pane = Pane::Songs;
                    }
                }
                _ => {}
            }
            return true;
        }
        if std::mem::take(&mut self.all_keys) { return true } // any key closes the list of keys
        if let (Some(_), Some(e)) = (&self.picker, e) { self.pick_key(k, e, state, rt, ctx); return true }
        if k.code == KeyCode::Char('q') { return false }
        if k.code == KeyCode::Char('l') && k.modifiers.contains(KeyModifiers::CONTROL) { self.repaint = true; return true }
        let Some(e) = e else { return true };
        // The lyrics cover the song list: its movement keys and Enter go to the lyrics, never to the hidden list.
        let moves = matches!(k.code, KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k') | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End | KeyCode::Enter);
        // The now-playing view has nothing to move through; Esc (or z) goes back to the lists.
        if self.focus && (moves || k.code == KeyCode::Esc) {
            match k.code {
                KeyCode::Esc => self.focus = false,
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k') => self.lyrics_key(k, e, state), // plain lyrics scroll
                _ => {}
            }
            return true;
        }
        if k.code == KeyCode::Char('f') {
            // The next list this page has, then back to the songs.
            let kinds = kinds(&state.lock().unwrap());
            let at = self.cards.as_ref().and_then(|c| kinds.iter().position(|&k| k == c.kind)).map_or(0, |i| i + 1);
            self.cards = kinds.get(at).map(|&kind| Cards { kind, row: 0, top: 0 });
            if self.cards.is_some() { (self.pane, self.lyrics) = (Pane::Songs, false) }
            return true;
        }
        if self.cards.is_some() && self.pane == Pane::Songs && (moves || k.code == KeyCode::Esc) { self.cards_key(k, e, state, rt, ctx); return true }
        // Like the lyrics, the cards cover the songs: song keys act on the playing one.
        let covered = self.lyrics || self.cards.is_some() || self.focus;
        if self.lyrics && self.pane == Pane::Songs && moves { self.lyrics_key(k, e, state); return true }
        let (lists, rows) = { let s = state.lock().unwrap(); (lists_of(&s).len(), s.tracks.len()) };
        let page = self.body_h.max(1);
        let song_at = |row: usize| state.lock().unwrap().tracks.get(row).and_then(|t| t.id.clone());
        let (held, before) = (song_at(self.row), self.row);
        // What l, i, g, G, c and p act on: the song under the cursor, or the playing one (the lists have focus, or
        // the lyrics are up).
        let pick = { let s = state.lock().unwrap(); if self.pane == Pane::Songs && !covered { s.tracks.get(self.row) } else { s.now.as_ref() }.cloned() };
        let pane = self.pane;
        let open = |src: Option<Source>| -> bool {
            let Some(src) = src else { return false };
            open_list(rt, e, state, src, ctx);
            true
        };
        let (cur, len) = match self.pane { Pane::Lists => (&mut self.side, lists), Pane::Songs => (&mut self.row, rows) };
        let last = len.saturating_sub(1);
        let cmd = |c| command(e, &mut state.lock().unwrap(), c);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => *cur = cur.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *cur = (*cur + 1).min(last),
            KeyCode::PageUp => *cur = cur.saturating_sub(page),
            KeyCode::PageDown => *cur = (*cur + page).min(last),
            KeyCode::Home => *cur = 0,
            KeyCode::End => *cur = last,
            KeyCode::Tab | KeyCode::BackTab => self.pane = if self.pane == Pane::Lists { Pane::Songs } else { Pane::Lists },
            KeyCode::Enter if self.pane == Pane::Lists => {
                let src = lists_of(&state.lock().unwrap()).into_iter().nth(self.side).map(|(src, _)| src);
                if let Some(src) = src {
                    open_list(rt, e, state, src, ctx);
                    self.pane = Pane::Songs;
                }
            }
            KeyCode::Enter => {
                let mut s = state.lock().unwrap();
                if let Some(t) = s.tracks.get(self.row) {
                    if let Some(id) = t.id.clone() { forget_unplayable(&mut s.unplayable, &id) } // another try
                    if s.view == Some(Source::Queue) {
                        let at = s.qi + self.row; // picking from the queue jumps ahead in it
                        s.jump_to(&e.player(), at);
                    } else {
                        let (list, src) = (s.tracks.clone(), s.view.clone());
                        s.start(&e.player(), list, Some(self.row), src);
                    }
                }
            }
            KeyCode::Char(' ') => cmd(Cmd::Toggle),
            KeyCode::Char('n') => cmd(Cmd::Next),
            KeyCode::Char('b') => cmd(Cmd::Prev),
            KeyCode::Right => cmd(Cmd::SeekBy(10_000)),
            KeyCode::Left => cmd(Cmd::SeekBy(-10_000)),
            KeyCode::Char(c @ ('+' | '=' | '-')) => {
                // Whole 5% steps, like the window's + and -: 54% goes up to 60, down to 50.
                // While muted, from the volume before the mute (and unmuted): a muted `-` would otherwise save 0%.
                let from = self.muted.take().unwrap_or(self.volume);
                self.volume = ((from + if c == '-' { -0.05 } else { 0.05 }) * 20.0).round().clamp(0.0, 20.0) / 20.0;
                e.mixer.set_volume((self.volume * u16::MAX as f32) as u16);
                set_setting("volume", &format!("{:.2}", self.volume));
            }
            KeyCode::Char('s') => cmd(Cmd::Shuffle),
            KeyCode::Char('m') => {
                // Not saved: the next start comes back at the volume from before.
                self.volume = match self.muted.take() { Some(v) => v, None => { self.muted = Some(self.volume); 0.0 } };
                e.mixer.set_volume((self.volume * u16::MAX as f32) as u16);
            }
            KeyCode::Char('r') => state.lock().unwrap().cycle_repeat(),
            KeyCode::Char('/') => (self.search, self.focus) = (Some(String::new()), false), // typed where it shows
            KeyCode::Char('l') => if let Some(id) = pick.and_then(|t| t.id) { toggle_like(rt, e, state, id) },
            // a: to the end of the queue, A: right after this song.
            KeyCode::Char(c @ ('a' | 'A')) if pane == Pane::Songs && !covered => if let Some(t) = pick {
                state.lock().unwrap().enqueue(&e.player(), t, c == 'A');
            },
            KeyCode::Char('i') => {
                let radio = pick.and_then(|t| Some(Source::Radio { track: t.id?, name: t.name }));
                if open(radio) { self.pane = Pane::Songs }
            }
            KeyCode::Char('g') => if open(pick.as_ref().and_then(artist_of)) { self.pane = Pane::Songs },
            KeyCode::Char('G') => if open(pick.as_ref().and_then(album_of)) { self.pane = Pane::Songs },
            KeyCode::Char('c') => if let Some(id) = pick.and_then(|t| t.id) {
                let copied = copy(&format!("https://open.spotify.com/track/{id}"));
                state.lock().unwrap().say(if copied { "Copied the song link".into() } else { "No clipboard to copy to".into() }, !copied);
            },
            KeyCode::Char('.') => {
                let (src, id) = { let s = state.lock().unwrap(); (s.queue_src.clone(), s.now.as_ref().and_then(|t| t.id.clone())) };
                if let Some(src) = src { open_list(rt, e, state, src, ctx) } // no-op when it's already open
                (self.jump, self.pane) = (id, Pane::Songs);
            }
            KeyCode::Char('?') => self.all_keys = true,
            KeyCode::Char('z') => (self.focus, self.lyr_top) = (!self.focus, 0),
            KeyCode::Char('v') => (self.lyrics, self.next_row, self.lyr_top) = (!self.lyrics, 0, 0),
            KeyCode::Char('p') => if let Some(t) = pick.filter(|t| t.id.is_some()) {
                ask_members(rt, e, state); // the √ marks: what's in your playlists, read once
                self.picker = Some(Picker { track: t, row: 0, top: 0, name: String::new() });
            },
            KeyCode::Char('o') => state.lock().unwrap().cycle_sort(), // the cursor follows its song (`sel`)
            KeyCode::Char('t') => {
                self.style = if self.style == Style::Card { Style::Board } else { Style::Card };
                set_setting("cli_style", if self.style == Style::Board { "board" } else { "card" });
            }
            _ => {}
        }
        // Moving picks a new song; anything else keeps the one that was under the cursor.
        self.sel = if self.row != before { song_at(self.row) } else { held };
        self.side_src = lists_of(&state.lock().unwrap()).get(self.side).map(|(src, _)| src.clone()); // where the keys put it
        true
    }

    /// The whole screen, every cell, as one write.
    /// `p` is open: move over your playlists, Enter adds (or, on the last row, makes the one being named), Esc closes.
    fn pick_key(&mut self, k: KeyEvent, e: &Engine, state: &Shared, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        let Some(p) = &mut self.picker else { return };
        let lists = own_lists(&state.lock().unwrap(), &self.me);
        let naming = p.row == lists.len();
        match k.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => p.row = p.row.saturating_sub(1),
            KeyCode::Down => p.row = (p.row + 1).min(lists.len()),
            KeyCode::Char('k') if !naming => p.row = p.row.saturating_sub(1),
            KeyCode::Char('j') if !naming => p.row += 1,
            KeyCode::Backspace if naming => { p.name.pop(); }
            KeyCode::Char(c) if naming && !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => p.name.push(c),
            KeyCode::Enter => {
                let id = p.track.id.clone().unwrap_or_default();
                match lists.get(p.row) {
                    Some((pid, name)) => {
                        let has = state.lock().unwrap().members.get(pid).is_some_and(|m| m.contains(&id));
                        add_to(rt, e, state, pid.clone(), name.clone(), id, !has, ctx);
                        self.picker = None
                    }
                    None if p.name.trim().is_empty() => {}
                    None => { new_playlist(rt, e, state, p.name.trim().to_string(), Some(id), ctx); self.picker = None }
                }
            }
            _ => {}
        }
    }

    /// `f` is open: move through its list, Enter opens the one under the cursor, Esc goes back to the songs.
    fn cards_key(&mut self, k: KeyEvent, e: &Engine, state: &Shared, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        let Some(c) = &mut self.cards else { return };
        let mut s = state.lock().unwrap();
        let rows = cards_of(&s, c.kind);
        let (last, page) = (rows.len().saturating_sub(1), self.body_h.max(1));
        match k.code {
            KeyCode::Esc => { drop(s); self.cards = None; }
            KeyCode::Up | KeyCode::Char('k') => c.row = c.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => c.row = (c.row + 1).min(last),
            KeyCode::PageUp => c.row = c.row.saturating_sub(page),
            KeyCode::PageDown => c.row = (c.row + page).min(last),
            KeyCode::Home => c.row = 0,
            KeyCode::End => c.row = last,
            KeyCode::Enter => {
                let Some(src) = rows.into_iter().nth(c.row).and_then(|(_, _, src)| src) else { return };
                // A playlist from search isn't in the library: keep it for its name and owner.
                if c.kind == Kind::Playlists { s.visiting = s.found.as_ref().and_then(|f| f.playlists.get(c.row).cloned()) }
                drop(s);
                open_list(rt, e, state, src, ctx); // the new page closes the cards
            }
            _ => {}
        }
    }

    /// Up and down in the lyrics view: through up next when there are no lyrics (Enter plays one), else through
    /// lyrics without timings. Synced lyrics scroll themselves, so they take no keys.
    fn lyrics_key(&mut self, k: KeyEvent, e: &Engine, state: &Shared) {
        let mut s = state.lock().unwrap();
        let lines = match s.lyrics.as_ref().filter(|(id, _)| Some(id) == s.now.as_ref().and_then(|t| t.id.as_ref())) {
            Some((_, Lyrics::Lines { synced: true, .. })) => return,
            Some((_, Lyrics::Lines { lines, .. })) => Some(lines.len()),
            _ => None,
        };
        let missing = lines.is_none();
        let (cur, last) = match lines { None => (&mut self.next_row, UP_NEXT - 1), Some(n) => (&mut self.lyr_top, n.saturating_sub(1)) };
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => *cur = cur.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *cur = (*cur + 1).min(last),
            KeyCode::Enter if missing => {
                if let Some(&(i, _)) = s.up_next(UP_NEXT).get(self.next_row) {
                    s.jump_to(&e.player(), i);
                    self.next_row = 0;
                }
            }
            _ => {}
        }
    }

    /// What the songs box holds instead of the songs, if anything: the keys (`?`), the playlist picker (`p`) or
    /// the lyrics (`v`). Its title, what the top right says, and `h` rows `w` wide on `bg`.
    fn instead(&mut self, s: &State, w: usize, h: usize, bg: Option<C>) -> Option<(String, String, Vec<String>)> {
        let blank = || Seg::new(w, bg).done();
        if self.all_keys {
            // As many columns as fit, each a run of whole groups, top to bottom.
            // ponytail: a short window cuts the longest column off; a page key if that ever matters.
            let col_w = 34;
            let cols = (w / col_w).clamp(1, ALL_KEYS.len());
            let columns: Vec<Vec<(&str, &str)>> = ALL_KEYS.chunks(ALL_KEYS.len().div_ceil(cols)).map(|groups| {
                groups.iter().enumerate().flat_map(|(i, (group, keys))| {
                    let gap = (i > 0).then_some(("", ""));
                    gap.into_iter().chain([(*group, "")]).chain(keys.iter().copied())
                }).collect()
            }).collect();
            let rows = (0..h).map(|r| {
                let mut row = Seg::new(w, bg);
                for column in &columns {
                    match column.get(r) {
                        Some((g, "")) => { row.col(g, red(), col_w); }
                        Some((k, l)) => { row.col(&format!("  {k}"), FG, 15).col(l, DIM, col_w - 15); }
                        None => { row.pad(col_w); }
                    }
                }
                row.done()
            }).collect();
            return Some(("keys".into(), "any key closes".into(), rows));
        }
        if let Some(p) = &mut self.picker {
            let lists = own_lists(s, &self.me);
            let id = p.track.id.clone().unwrap_or_default();
            p.row = p.row.min(lists.len());
            scroll(&mut p.top, p.row, h);
            let rows = (p.top..p.top + h).map(|i| {
                let mut row = Seg::new(w, if i == p.row { Some(cursor()) } else { bg });
                match lists.get(i) {
                    Some((pid, name)) => {
                        let has = s.members.get(pid).is_some_and(|m| m.contains(&id));
                        row.put(name, if has { DIM } else { FG });
                        if has { row.put("  √ in it, Enter takes it out", FAINT); }
                    }
                    None if i == lists.len() => {
                        row.put("+ ", red());
                        if p.name.is_empty() && i != p.row { row.put("new playlist", FAINT); } else { row.put(&p.name, FG).put("▌", red()); }
                    }
                    None => {}
                }
                row.done()
            }).collect();
            return Some((format!("add \u{201c}{}\u{201d} to", p.track.name), "enter adds · esc".into(), rows));
        }
        if let Some(c) = &mut self.cards {
            let rows = cards_of(s, c.kind);
            c.row = c.row.min(rows.len().saturating_sub(1));
            scroll(&mut c.top, c.row, h);
            let name_w = w * 11 / 20;
            let lines = (c.top..c.top + h).map(|i| {
                let mut row = Seg::new(w, if i == c.row { Some(cursor()) } else { bg });
                if let Some((name, note, _)) = rows.get(i) { row.col(name, FG, name_w).put(note, DIM); }
                row.done()
            }).collect();
            let more = if kinds(s).len() > 1 { "f next" } else { "f songs" };
            let hint = if c.kind == Kind::Shows { format!("{more} · esc") } else { format!("enter opens · {more} · esc") };
            return Some((format!("{} · {}", c.kind.name(), rows.len()), hint, lines));
        }
        if !self.lyrics { return None }
        let Some(now) = &s.now else { return Some(("lyrics".into(), String::new(), vec![center(&[("nothing playing".into(), FAINT)], w, bg)].into_iter().chain(std::iter::repeat_with(blank)).take(h).collect())) };
        let lyrics = s.lyrics.as_ref().filter(|(id, _)| now.id.as_ref() == Some(id)).map(|(_, l)| l);
        let next = s.up_next(UP_NEXT);
        let pos = s.position();
        let next_row = |row: &mut Seg, k: usize, (_, t): &(usize, &api::Track), lit: bool| {
            let when = match k { 0 => format!("IN {}", clock(now.duration_ms.saturating_sub(pos))), 1 => "THEN".into(), _ => String::new() };
            let rest = w.saturating_sub(11 + 6);
            row.col(&when, if lit { FG } else { DIM }, 11).col(&t.name, FG, rest * 3 / 5).col(&t.artist_line(), if lit { DIM } else { SUB }, rest - rest * 3 / 5).right(&[(clock(t.duration_ms), DIM)], 0);
        };
        let mut rows: Vec<String> = Vec::with_capacity(h);
        let (title, kind) = (format!("lyrics · {}", now.name), match lyrics {
            Some(Lyrics::Lines { synced: true, .. }) => "synced",
            Some(Lyrics::Lines { .. }) => "plain",
            Some(Lyrics::Missing) => "none",
            _ => "loading",
        });
        match lyrics {
            Some(Lyrics::Lines { synced, lines }) => {
                // The words, then a rule and the next two songs under them.
                let room = h.saturating_sub(3);
                let cur = if *synced { lines.iter().rposition(|(at, _)| *at <= pos) } else { None };
                let first = match cur { Some(c) => c.saturating_sub(room / 2), None => self.lyr_top.min(lines.len().saturating_sub(1)) };
                for r in 0..room {
                    let i = first + r;
                    let color = match (cur, lines.get(i)) {
                        (_, None) => FG,
                        (None, _) => FG,
                        (Some(c), _) if i == c => BRIGHT,
                        (Some(c), _) if i + 1 == c => SUB,
                        (Some(c), _) if i < c => FAINT,
                        (Some(c), _) if i == c + 1 => FG,
                        _ => DIM,
                    };
                    rows.push(lines.get(i).map_or_else(blank, |(_, words)| center(&[(words.clone(), color)], w, bg)));
                }
                rows.push(Seg::new(w, bg).put(&"─".repeat(w), GUTTER).done());
                for k in 0..2 {
                    let mut row = Seg::new(w, bg);
                    if let Some(n) = next.get(k) { next_row(&mut row, k, n, false) }
                    rows.push(row.done());
                }
            }
            _ => {
                // None (or still coming): the window's meter and line, and up next to pick from.
                let t = if s.since.is_some() { self.born.elapsed().as_secs_f32() } else { 0.0 };
                let bars: String = (0..21).map(|i| BARS[(((t * 5.0 - i as f32 * 0.45).sin() + 1.0) * 3.5).round() as usize]).collect();
                let label = if kind == "loading" { format!("{} loading lyrics", self.spin()) } else { "no lyrics · 歌詞なし · up next".into() };
                rows.extend([blank(), center(&[(bars, red())], w, bg), blank(), center(&[(label, DIM)], w, bg), blank()]);
                self.next_row = self.next_row.min(next.len().saturating_sub(1));
                for (k, n) in next.iter().enumerate() {
                    let lit = k == self.next_row && self.pane == Pane::Songs;
                    let mut row = Seg::new(w, if lit { Some(cursor()) } else { bg });
                    next_row(&mut row, k, n, lit);
                    rows.push(row.done());
                }
            }
        }
        rows.resize_with(h, blank);
        rows.truncate(h);
        Some((title, kind.into(), rows))
    }

    fn draw(&mut self, s: &State, w: usize, h: usize, starting: bool) -> Vec<String> {
        if w < 60 || h < 18 {
            let mut lines = vec![center(&[("make the window a little bigger".into(), DIM)], w, None)];
            lines.resize_with(h, || Seg::new(w, None).done());
            return lines;
        }
        if starting && s.error.is_none() { return self.splash(s, w, h) }
        let lists = lists_of(s);
        if let Some(i) = self.side_src.as_ref().and_then(|src| lists.iter().position(|(l, _)| l == src)) { self.side = i }
        if s.view != self.seen {
            // A new list: start at its top, and point the sidebar at it.
            (self.row, self.row_top, self.seen, self.sel, self.cards) = (0, 0, s.view.clone(), None, None);
            if let Some(i) = lists.iter().position(|(src, _)| Some(src) == s.view.as_ref()) { self.side = i }
        }
        if let Some(id) = &self.sel
            && s.tracks.get(self.row).and_then(|t| t.id.as_ref()) != Some(id)
            && let Some(i) = s.tracks.iter().position(|t| t.id.as_ref() == Some(id))
        {
            self.row = i; // songs moved (a sorted page arrived, or `o`): follow the one under the cursor
        }
        if let Some(id) = &self.jump {
            // `.`: land on the playing song once its list has it (or give up once the list is done loading).
            match s.tracks.iter().position(|t| t.id.as_ref() == Some(id)) {
                Some(i) => (self.row, self.jump) = (i, None),
                None if !s.loading => self.jump = None,
                None => {}
            }
        }
        self.side = self.side.min(lists.len().saturating_sub(1));
        self.side_src = lists.get(self.side).map(|(src, _)| src.clone());
        self.row = self.row.min(s.tracks.len().saturating_sub(1));
        // The library is as wide as its longest name, within reason.
        let longest = lists.iter().map(|(_, n)| n.width()).max().unwrap_or(0);
        if self.focus && !self.all_keys && self.picker.is_none() { return self.now_view(s, w, h) }
        match self.style {
            Style::Card => self.card(s, &lists, longest, w, h),
            Style::Board => self.board(s, &lists, longest, w, h),
        }
    }

    /// Until the engine is up: the name types itself out over a ripple of equaliser bars, with what the
    /// engine is doing underneath.
    fn splash(&self, s: &State, w: usize, h: usize) -> Vec<String> {
        let bg = if self.style == Style::Board { Some(BG) } else { None };
        let t = self.born.elapsed().as_secs_f32();
        let row = |parts: &[(String, C)]| center(parts, w, bg);
        let bars: String = (0..21).map(|i| BARS[(((t * 5.0 - i as f32 * 0.45).sin() + 1.0) * 3.5).round() as usize]).collect();
        let name: String = "r e s o n a".chars().take((t * 22.0) as usize).collect();
        let status = if s.status.is_empty() { "starting…" } else { &s.status };
        let mut lines = vec![row(&[]); h.saturating_sub(7) / 2];
        lines.extend([row(&[(bars, red())]), row(&[]), row(&[(format!("{name:<11}"), BRIGHT)]), row(&[]), row(&[(status.into(), DIM)]), row(&[]), row(&[("q quits".into(), GUTTER)])]);
        lines.resize_with(h, || row(&[]));
        lines
    }

    /// A braille spinner on the splash clock.
    fn spin(&self) -> &'static str {
        SPIN[(self.born.elapsed().as_millis() / 80) as usize % SPIN.len()]
    }

    /// A centred card of titled boxes on the terminal's own background, filling the window.
    fn card(&mut self, s: &State, lists: &[(Source, String)], longest: usize, w: usize, h: usize) -> Vec<String> {
        let blank = || Seg::new(w, None).done();
        let cw = (w - 4).min(220);
        let x0 = (w - cw) / 2;
        let line = |content: String| format!("{}{content}{}", Seg::new(x0, None).done(), Seg::new(w - x0 - cw, None).done());
        let body_h = h - 14;
        self.body_h = body_h;
        let mut lines: Vec<String> = Vec::with_capacity(h);
        lines.push(blank());

        // Brand, then whatever the engine has to say.
        lines.push(line(center(&[("resona".into(), FG), (concat!("  v", env!("CARGO_PKG_VERSION")).into(), GUTTER)], cw, None)));
        let note = match (&s.error, s.status.is_empty(), s.notice()) {
            (Some(err), _, _) => vec![(short_err(err), red())],
            (None, false, _) => vec![(s.status.clone(), DIM)],
            (None, true, Some(n)) => vec![(n.text.clone(), if n.error { red() } else { SUB })],
            _ => vec![],
        };
        lines.push(line(center(&note, cw, None)));

        // Lists and songs, side by side; the focused box is the red one.
        let lw = (longest + 4).clamp(22, (cw / 3).max(22));
        let rw = cw - lw - 1;
        let instead = self.instead(s, rw - 4, body_h, None);
        let shown = self.song_lines(s, body_h);
        let edge = |pane| if self.pane == pane { red() } else { FAINT };
        let songs_edge = if self.picker.is_some() || self.all_keys { red() } else { edge(Pane::Songs) };
        let (head, count) = match &instead { Some((t, c, _)) => (t.clone(), c.clone()), None => (title(s, lists), count(s, self.spin())) };
        lines.push(line(format!("{} {}",
            top(lw, edge(Pane::Lists), &[("library".into(), FG)], &[(lists.len().to_string(), DIM)]),
            top(rw, songs_edge, &[(head, FG)], &[(count, DIM)]))));
        scroll(&mut self.side_top, self.side, body_h);
        let cursor_bg = |on: bool, pane| if !on { None } else if self.pane == pane { Some(cursor()) } else { Some(dull()) };
        for i in 0..body_h {
            let mut left = Seg::new(lw - 4, None);
            if let Some((src, name)) = lists.get(self.side_top + i) {
                let open = s.view.as_ref() == Some(src);
                let sel = self.side_top + i == self.side;
                left = Seg::new(lw - 4, cursor_bg(sel, Pane::Lists));
                left.put(name, if open { red() } else if sel { FG } else { DIM });
            }
            let right = match (&instead, shown.get(i)) {
                (Some((_, _, rows)), _) => rows[i].clone(),
                (None, Some(&Line::Song(at))) => song_row(s, at, rw - 4, false, cursor_bg(at == self.row, Pane::Songs), at == self.row).done(),
                (None, Some(&Line::Day(ms))) => day_row(ms, rw - 4, 0, None),
                (None, None) => {
                    let mut right = Seg::new(rw - 4, None);
                    if i == 0 && s.tracks.is_empty() { right.put(if s.loading { self.spin() } else if s.view.is_none() { "…" } else { "nothing here" }, GUTTER); }
                    right.done()
                }
            };
            lines.push(line(format!("{} {}", side(edge(Pane::Lists), left.done()), side(songs_edge, right))));
        }
        lines.push(line(format!("{} {}", bottom(lw, edge(Pane::Lists)), bottom(rw, songs_edge))));
        lines.push(blank());

        // The player: song and how it's playing, then the dotted progress line.
        let state_word = match (&s.now, s.since.is_some()) {
            (None, _) => "idle",
            (Some(_), true) => "▶ playing",
            (Some(_), false) => "‖ paused",
        };
        lines.push(line(top(cw, SUB, &[(state_word.into(), FG)], &[(format!("vol {:.0}%", self.volume * 100.0), DIM)])));
        let flags: Vec<&str> = flags(s).collect();
        let mut song = Seg::new(cw - 4, None);
        let mut bar = Seg::new(cw - 4, None);
        match &s.now {
            Some(t) => {
                let flags = vec![(flags.join(" · "), SUB)];
                let room = song.room().saturating_sub(width(&flags) + 2);
                song.put_in(&t.name, FG, room);
                let room = room.saturating_sub(song.used);
                song.put_in(&format!("  {}", t.artist_line()), SUB, room).right(&flags, 0);
                let (pos, dur) = (s.position(), t.duration_ms.max(1));
                let (a, b) = (clock(pos), clock(dur));
                let bar_w = (cw - 4).saturating_sub(a.len() + b.len() + 2);
                let filled = (pos as usize * bar_w / dur as usize).min(bar_w);
                bar.put(&a, DIM).put(" ", DIM).put(&"⣿".repeat(filled), red()).put(&"⣀".repeat(bar_w - filled), GUTTER).put(" ", DIM).put(&b, DIM);
            }
            None => { song.put("nothing playing", GUTTER); }
        }
        lines.push(line(side(SUB, song.done())));
        lines.push(line(side(SUB, bar.done())));
        lines.push(line(bottom(cw, SUB)));
        lines.push(blank());

        // Keys, or the search being typed.
        match &self.search {
            Some(q) => {
                lines.push(line(center(&[("/ ".into(), red()), (q.clone(), FG), ("▌".into(), red())], cw, None)));
                lines.push(line(center(&keys(&[("enter", "search"), ("esc", "cancel")]), cw, None)));
            }
            None => {
                lines.push(line(center(&keys(&KEYS[..6]), cw, None)));
                lines.push(line(center(&keys(&KEYS[6..]), cw, None)));
            }
        }
        lines.resize_with(h, blank);
        lines
    }

    /// Full width on a painted background: a header, lists and songs, the player, the keys.
    fn board(&mut self, s: &State, lists: &[(Source, String)], longest: usize, w: usize, h: usize) -> Vec<String> {
        let mut lines = Vec::with_capacity(h);
        let inner = w - 4;
        // The body's sizes, needed first: the header names what the songs box shows.
        let (body_h, lw) = (h - 8, (longest + 2).clamp(18, ((w - 4) / 3).max(18)));
        let instead = self.instead(s, w - 4 - lw - 3 - 2, body_h, Some(BG));
        let rule = || Seg::new(w, Some(BG)).pad(2).put(&"─".repeat(inner), RULE).done();

        // Header: brand hard left, what's open hard right.
        lines.push(Seg::new(w, Some(BG)).done());
        let mut status: Vec<(String, C)> = Vec::new();
        if let Some(err) = &s.error { status.push((format!("{}  ·  ", short_err(err)), red())) }
        if !s.status.is_empty() {
            status.push((s.status.clone(), DIM));
        } else {
            let (head, count) = match &instead { Some((t, c, _)) => (t.clone(), c.clone()), None => (title(s, lists), count(s, self.spin())) };
            status.push((head, DIM));
            status.push((format!("  ·  {count}"), SUB));
            if let Some(n) = s.notice() { status.push((format!("  ·  {}", n.text), if n.error { red() } else { SUB })) }
        }
        let mut head = Seg::new(w, Some(BG));
        head.pad(2).put("▌ ", red()).put("resona", FG).put(concat!(" v", env!("CARGO_PKG_VERSION")), GUTTER);
        lines.push(head.right(&status, 2).done());
        lines.push(rule());

        // Body: lists on the left under their two headings, songs on the right under their column heads.
        // The bar only marks what's open and what's playing.
        enum Side { Head(&'static str, String), Gap, List(usize) }
        let lib = lists.iter().take_while(|(src, _)| !matches!(src, Source::Playlist(_))).count();
        let mut side = vec![Side::Head("library", String::new())];
        side.extend((0..lib).map(Side::List));
        side.extend([Side::Gap, Side::Head("playlists", (lists.len() - lib).to_string())]);
        side.extend((lib..lists.len()).map(Side::List));
        let at = side.iter().position(|l| matches!(l, Side::List(i) if *i == self.side)).unwrap_or(0);
        if at > 0 && matches!(side[at - 1], Side::Head(..)) { scroll(&mut self.side_top, at - 1, body_h) } // its heading too
        scroll(&mut self.side_top, at, body_h);
        let rw = inner - lw - 3;
        let songs_h = if instead.is_some() { body_h } else { body_h - 1 };
        self.body_h = songs_h;
        let shown = self.song_lines(s, songs_h);
        let cursor_bg = |on: bool, pane| Some(if !on { BG } else if self.pane == pane { cursor() } else { dull() });
        for i in 0..body_h {
            let mut left = Seg::new(lw, Some(BG));
            match side.get(self.side_top + i) {
                Some(Side::Head(name, n)) => { left.put(&format!("  {name}"), FAINT).right(&[(n.clone(), FAINT)], 1); }
                Some(&Side::List(j)) => {
                    let (src, name) = &lists[j];
                    let open = s.view.as_ref() == Some(src);
                    left = Seg::new(lw, cursor_bg(j == self.side, Pane::Lists));
                    left.put(if open { "▌ " } else { "  " }, red()).put(name, if open || j == self.side { FG } else { DIM });
                }
                Some(Side::Gap) | None => {}
            }
            let right = match (&instead, i.checked_sub(1).map(|i| shown.get(i))) {
                (Some((_, _, rows)), _) => format!("{}{}", Seg::new(2, Some(BG)).done(), rows[i]),
                (None, None) => song_head(s, rw),
                (None, Some(Some(&Line::Song(at)))) => song_row(s, at, rw, true, cursor_bg(at == self.row, Pane::Songs), at == self.row).done(),
                (None, Some(Some(&Line::Day(ms)))) => day_row(ms, rw, 2, Some(BG)),
                (None, Some(None)) => {
                    let mut right = Seg::new(rw, Some(BG));
                    if i == 1 && s.tracks.is_empty() { right.pad(2).put(if s.loading { self.spin() } else if s.view.is_none() { "…" } else { "nothing here" }, FAINT); }
                    right.done()
                }
            };
            lines.push(format!("{}{}{}{}{}", Seg::new(2, Some(BG)).done(), left.done(), Seg::new(3, Some(BG)).done(), right, Seg::new(2, Some(BG)).done()));
        }

        // Player, three lines beside the cover: the song and how it's playing, its artists and album, where it is.
        lines.push(rule());
        let art = match &s.now { Some(_) => self.art(6, 3), None => vec![Seg::new(6, Some(PANEL)).done(); 3] };
        let player = |k: usize| { let mut p = Seg::new(w, Some(PANEL)); p.pad(2).raw(art[k].clone(), 6).pad(3); p };
        let mut right: Vec<(String, C)> = flags(s).map(|f| (format!("{f}   "), SUB)).collect();
        right.push((format!("vol {:.0}%", self.volume * 100.0), DIM));
        let (mut one, mut two, mut three) = (player(0), player(1), player(2));
        match &s.now {
            Some(t) => {
                let room = one.room().saturating_sub(width(&right) + 4);
                one.put(if s.since.is_some() { "▶  " } else { "‖  " }, red()).put_in(&t.name, BRIGHT, room.saturating_sub(3));
                let album = if t.album.name.is_empty() { String::new() } else { format!("  ·  {}", t.album.name) };
                two.pad(3).put(&t.artist_line(), SUB).put(&album, FAINT);
                let mut bar = Seg::new(three.room().saturating_sub(3 + 2), Some(PANEL));
                progress(&mut bar, s.position(), t.duration_ms);
                let bar_w = bar.max;
                three.pad(3).raw(bar.done(), bar_w);
            }
            None => { one.put("nothing playing", FAINT); }
        }
        lines.extend([one.right(&right, 2).done(), two.done(), three.done()]);

        // Keys for the pane the cursor is in, or the search being typed.
        let mut keys = Seg::new(w, Some(BG));
        keys.pad(2);
        match &self.search {
            Some(q) => { keys.put("/ ", red()).put(q, FG).put("▌", red()).put("   enter search   esc cancel", DIM); }
            None => {
                let (name, pairs) = if self.pane == Pane::Lists { ("lists  ", LIST_KEYS) } else { ("songs  ", SONG_KEYS) };
                keys.put(name, FAINT);
                for (i, (k, label)) in pairs.iter().enumerate() {
                    keys.put(if i == 0 { "" } else { "   " }, DIM).put(k, FG).put(" ", DIM).put(label, DIM);
                }
                // Hard right, the keys that work anywhere; only `?` when the line is short of room.
                let always: &[(&str, &str)] = &[("space", " pause   "), ("z", " now playing   "), ("?", " all keys")];
                let always = if keys.room() >= 3 + 40 + 2 { always } else { &always[2..] }; // gap, the keys, margin
                keys.pad(3).right(&always.iter().flat_map(|(k, l)| [(k.to_string(), FG), (l.to_string(), DIM)]).collect::<Vec<_>>(), 2);
            }
        }
        lines.push(keys.done());
        lines
    }

    /// `z`: the playing song large. Its cover in half-blocks on the left; on the right the song, where it is, the
    /// lyrics (the current line a third of the way down) and the next two songs.
    fn now_view(&mut self, s: &State, w: usize, h: usize) -> Vec<String> {
        let bg = if self.style == Style::Board { Some(BG) } else { None };
        let blank = || Seg::new(w, bg).done();
        let mut lines = vec![blank()];
        let Some(now) = &s.now else {
            lines.resize_with(h / 2, blank);
            lines.push(center(&[("nothing playing · z goes back".into(), DIM)], w, bg));
            lines.resize_with(h, blank);
            return lines;
        };
        // Cells are about twice as tall as wide: a square cover is twice as many columns as rows.
        let rows = (h - 4).min(w / 5);
        let (x, cols) = (4, rows * 2);
        let rw = w - x - cols - 5 - 4;
        let art = self.art(cols, rows);
        let line = || Seg::new(rw, bg);
        let mut right: Vec<String> = Vec::with_capacity(rows);
        right.push(line().put("now playing", FAINT).done());
        right.push(line().done());
        right.push(line().put(&now.name, BRIGHT).done());
        right.push(line().put(&now.artist_line(), SUB).done());
        right.push(line().put(&now.album.name, FAINT).done());
        right.push(line().done());
        let pos = s.position();
        let mut bar = line();
        progress(&mut bar, pos, now.duration_ms);
        right.push(bar.done());
        let mut state: Vec<(String, C)> = vec![(if s.since.is_some() { "▶ playing" } else { "‖ paused" }.into(), FG)];
        state.extend(flags(s).map(|f| (format!("   {f}"), SUB)));
        let liked = now.id.as_ref().is_some_and(|id| s.liked.contains(id));
        let mut st = line();
        for (t, c) in &state { st.put(t, *c); }
        right.push(st.right(&[(if liked { "♥   " } else { "" }.into(), red()), (format!("vol {:.0}%", self.volume * 100.0), DIM)], 0).done());
        right.push(line().done());

        // The lyrics fill what's left over up next (four lines: a gap, its heading, two songs), when there's room.
        let next = s.up_next(2);
        let tail = if rows >= right.len() + 8 && !next.is_empty() { 3 + next.len() } else { 0 };
        let room = rows.saturating_sub(right.len() + tail);
        let lyrics = s.lyrics.as_ref().filter(|(id, _)| now.id.as_ref() == Some(id)).map(|(_, l)| l);
        match lyrics {
            Some(Lyrics::Lines { synced, lines: words }) => {
                let cur = if *synced { words.iter().rposition(|(at, _)| *at <= pos) } else { None };
                let first = match cur { Some(c) => c.saturating_sub(room / 3), None => self.lyr_top.min(words.len().saturating_sub(1)) };
                for (i, (_, text)) in words.iter().enumerate().skip(first).take(room) {
                    let (mark, color) = match cur { Some(c) if i == c => ("▌ ", BRIGHT), Some(c) if i < c => ("  ", FAINT), Some(_) => ("  ", DIM), None => ("  ", FG) };
                    right.push(line().put(mark, red()).put(text, color).done());
                }
            }
            Some(Lyrics::Missing) => right.push(line().put("no lyrics", FAINT).done()),
            _ => right.push(line().put(&format!("{} lyrics", self.spin()), FAINT).done()),
        }
        if tail > 0 {
            right.resize_with(rows - tail, || line().done());
            right.push(line().done());
            right.push(line().put("up next", FAINT).done());
            for (_, t) in &next {
                let mut row = line();
                let artist = format!("  {}", t.artist_line());
                let room = row.room().saturating_sub(6);
                row.put_in(&t.name, FG, room);
                let room = room.saturating_sub(row.used);
                right.push(row.put_in(&artist, SUB, room).right(&[(clock(t.duration_ms), DIM)], 0).done());
            }
        }
        right.resize_with(rows, || line().done());
        for (cover, words) in art.into_iter().zip(right) {
            lines.push(Seg::new(w, bg).pad(x).raw(cover, cols).pad(5).raw(words, rw).done());
        }
        lines.resize_with(h - 2, blank);
        // Notes and errors, as the lists' header would show them, over the keys.
        let note = match (&s.error, s.notice()) {
            (Some(err), _) => Some((short_err(err), red())),
            (None, Some(n)) => Some((n.text.clone(), if n.error { red() } else { SUB })),
            _ => None,
        };
        if let Some((text, c)) = note { lines[h - 3] = Seg::new(w, bg).pad(x).put(&text, c).done() }
        let mut keys = Seg::new(w, bg);
        keys.pad(x);
        for (i, (k, label)) in [("space", "pause"), ("n b", "next / back"), ("←→", "seek"), ("l", "like"), ("z", "back to the lists")].iter().enumerate() {
            keys.put(if i == 0 { "" } else { "   " }, DIM).put(k, FG).put(" ", DIM).put(label, DIM);
        }
        lines.extend([keys.done(), blank()]);
        lines
    }

    /// The songs box's `h` lines from `row_top`, scrolled so the cursor's song (and a day divider right over it) shows.
    fn song_lines(&mut self, s: &State, h: usize) -> Vec<Line> {
        let lines = lines_of(s);
        let at = lines.iter().position(|&l| l == Line::Song(self.row)).unwrap_or(0);
        if at > 0 && matches!(lines[at - 1], Line::Day(_)) { scroll(&mut self.row_top, at - 1, h) }
        scroll(&mut self.row_top, at, h);
        lines.into_iter().skip(self.row_top).take(h).collect()
    }

    /// Downloads the playing song's cover once it changes, for the board's player and `z` (the card shows none).
    fn want_cover(&self, rt: &tokio::runtime::Handle, e: &Engine, s: &State) {
        if self.style == Style::Card && !self.focus { return }
        let url = s.now.as_ref().and_then(|t| t.album.images.first()).map_or(String::new(), |i| i.url.clone());
        let mut cover = self.cover.lock().unwrap();
        if cover.url == url { return }
        *cover = Cover { url: url.clone(), busy: !url.is_empty(), ..Default::default() }; // no art: the plain block
        if url.is_empty() { return }
        let (slot, http) = (self.cover.clone(), e.http.clone());
        // ponytail: a failed download leaves the plain block until the next song; retry if that shows up.
        rt.spawn(async move {
            let bytes = async { http.get(&url).send().await.ok()?.bytes().await.ok() }.await;
            let img = bytes.and_then(|b| image::load_from_memory(&b).ok()).map(|i| Arc::new(i.thumbnail(160, 160).to_rgb8()));
            let mut cover = slot.lock().unwrap();
            if cover.url == url { (cover.img, cover.busy) = (img, false) } // still the playing song's
        });
    }

    /// The cover in `cols` × `rows` half-blocks, made once per cover and size.
    fn art(&self, cols: usize, rows: usize) -> Vec<String> {
        let mut cover = self.cover.lock().unwrap();
        if let Some((size, art)) = &cover.drawn && *size == (cols, rows) { return art.clone() }
        let art = half_blocks(cover.img.as_deref(), cols, rows);
        if cover.img.is_some() { cover.drawn = Some(((cols, rows), art.clone())) }
        art
    }
}

/// A line of the songs box: a song, or (New releases, sorted by date) a divider where the release day changes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Line { Song(usize), Day(i64) }

fn lines_of(s: &State) -> Vec<Line> {
    let dated = s.dated();
    let mut out = Vec::with_capacity(s.tracks.len());
    for (i, t) in s.tracks.iter().enumerate() {
        if dated && (i == 0 || s.tracks[i - 1].added != t.added) { out.push(Line::Day(t.added)) }
        out.push(Line::Song(i));
    }
    out
}

/// `0:42 ━━━━━━──────── 2:19`, filling the rest of `seg`.
fn progress(seg: &mut Seg, pos: u32, dur: u32) {
    let (a, b) = (clock(pos), clock(dur));
    let bar_w = seg.room().saturating_sub(a.len() + b.len() + 2);
    let filled = (pos as usize * bar_w / dur.max(1) as usize).min(bar_w);
    seg.put(&a, DIM).put(" ", DIM).put(&"━".repeat(filled), red()).put(&"─".repeat(bar_w - filled), RULE).put(" ", DIM).put(&b, DIM);
}

/// The play modes that are on.
fn flags(s: &State) -> impl Iterator<Item = &'static str> {
    [(s.shuffle, "shuffle"), (s.repeat == Repeat::All, "repeat"), (s.repeat == Repeat::One, "repeat one")].into_iter().filter(|(on, _)| *on).map(|(_, l)| l)
}

/// `── fri 30 oct ─────`, `indent` in, `w` wide.
fn day_row(ms: i64, w: usize, indent: usize, bg: Option<C>) -> String {
    let mut row = Seg::new(w, bg);
    row.pad(indent).put(&format!("── {} ", day_label(ms).to_lowercase()), SUB);
    let n = row.room();
    row.put(&"─".repeat(n), RULE).done()
}

/// The picture in `cols` × `rows` cells, two pixels to a cell: ▀ in the top pixel's colour over the bottom one's.
/// A plain block until it's downloaded.
fn half_blocks(img: Option<&image::RgbImage>, cols: usize, rows: usize) -> Vec<String> {
    let Some(img) = img else { return vec![Seg::new(cols, Some(RULE)).done(); rows] };
    let img = image::imageops::resize(img, cols as u32, rows as u32 * 2, image::imageops::FilterType::Triangle);
    (0..rows as u32).map(|y| (0..cols as u32).map(|x| {
        let (a, b) = (img.get_pixel(x, 2 * y), img.get_pixel(x, 2 * y + 1));
        format!("\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀", a[0], a[1], a[2], b[0], b[1], b[2])
    }).collect()).collect()
}

#[test]
fn release_days_divide_new_releases_and_covers_take_two_pixels_a_cell() {
    let mut s = State::default();
    s.view = Some(Source::NewReleases);
    s.tracks = [5, 5, 3].iter().map(|&added| api::Track { added, ..Default::default() }).collect();
    assert_eq!(lines_of(&s), [Line::Day(5), Line::Song(0), Line::Song(1), Line::Day(3), Line::Song(2)]);
    s.view = Some(Source::Liked);
    assert_eq!(lines_of(&s).len(), 3); // other lists have no dividers
    let img = image::RgbImage::from_fn(1, 2, |_, y| image::Rgb(if y == 0 { [255, 0, 0] } else { [0, 0, 255] }));
    assert_eq!(half_blocks(Some(&img), 1, 1), ["\x1b[38;2;255;0;0m\x1b[48;2;0;0;255m▀"]);
}

/// The column heads over the board's songs, lined up with `song_row`.
fn song_head(s: &State, w: usize) -> String {
    let (num_w, name_w, artist_w, album_w) = widths(s, w, true);
    let mut row = Seg::new(w, Some(BG));
    row.pad(2).col("#", FAINT, num_w + 2).col("title", FAINT, name_w).col("artist", FAINT, artist_w);
    if album_w > 0 { row.col("album", FAINT, album_w); }
    row.right(&[("time".into(), FAINT)], 0).done()
}

/// Number, title, artist and album widths for song rows `w` wide (album 0 when it doesn't fit). Title and artist are
/// capped, so a wide terminal gets an album column rather than wide gaps.
fn widths(s: &State, w: usize, bar: bool) -> (usize, usize, usize, usize) {
    let num_w = s.tracks.len().max(10).to_string().len();
    let rest = w.saturating_sub(if bar { 2 } else { 0 } + num_w + 2 + 2 + 6);
    if rest < 90 { return (num_w, rest * 11 / 20, rest - rest * 11 / 20, 0) }
    let (name, artist) = ((rest * 3 / 8).min(44), (rest * 3 / 10).min(34));
    (num_w, name, artist, rest - name - artist)
}

/// One song row, `w` wide, the same in both styles: number (▶ when playing), name, artists, heart, length.
/// `bar` adds the board's red bar in front of the playing song; `bg` is the row's background (lit when `sel`).
fn song_row(s: &State, at: usize, w: usize, bar: bool, bg: Option<C>, sel: bool) -> Seg {
    let t = &s.tracks[at];
    let playing = t.id.is_some() && t.id == s.now.as_ref().and_then(|n| n.id.clone());
    let gone = t.id.as_ref().is_some_and(|id| s.unplayable.contains(id));
    let liked = t.id.as_ref().is_some_and(|id| s.liked.contains(id));
    let (num_w, name_w, artist_w, album_w) = widths(s, w, bar);
    // Faded songs and the number column step up a shade on the selected row; red would vanish on it.
    let (faded, sub) = if sel { (FG, DIM) } else { (FAINT, SUB) };
    let accent = if sel { BRIGHT } else { red() };
    let hand = s.view == Some(Source::Queue) && (1..=s.queued).contains(&at); // queued by hand, in the queue view
    let num = if playing { "▶".to_string() } else if hand { "+".to_string() } else { (at + 1).to_string() };
    let mut row = Seg::new(w, bg);
    if bar { row.put(if playing { "▌ " } else { "  " }, red()); }
    row.col(&format!("{num:>num_w$}"), if playing || hand { accent } else { faded }, num_w + 2)
        .col(&t.name, if playing { accent } else if gone { faded } else { FG }, name_w)
        .col(&t.artist_line(), if gone { faded } else { sub }, artist_w)
        .col(&t.album.name, if sel { DIM } else { FAINT }, album_w)
        .col(if liked { "♥" } else { "" }, accent, 2)
        .right(&[(clock(t.duration_ms), DIM)], 0);
    row
}

fn count(s: &State, spin: &str) -> String {
    match (s.loading, s.expected) {
        (true, Some(n)) => format!("{spin} loading {}/{n}", s.tracks.len()),
        (true, None) => format!("{spin} loading"),
        _ => match (s.sort_label(), &s.artist) {
            (Some(l), _) => format!("{} songs · {l}", s.tracks.len()),
            // Artist pages: their numbers, and the next show (`f` lists them all).
            (None, Some(a)) => {
                let o = a.overview.as_ref();
                let stats = o.map_or(String::new(), |o| format!(" · {} listeners · {} followers", short(o.listeners), short(o.followers)));
                let live = o.and_then(|o| o.shows.first()).map_or(String::new(), |w| format!(" · live {}", w.day));
                format!("{} songs{stats} · popularity {}{live}", s.tracks.len(), a.popularity)
            }
            (None, None) if s.found.is_some() => format!("{} songs · f: artists, albums, playlists", s.tracks.len()),
            (None, None) => format!("{} songs", s.tracks.len()),
        },
    }
}

/// "18.2M", "950K", "999": a count short enough for the status line.
fn short(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        10_000.. => format!("{}K", n / 1000),
        1_000.. => format!("{:.1}K", n as f64 / 1e3),
        _ => n.to_string(),
    }
}

#[test]
fn shortens_counts() {
    assert_eq!([short(18_208_296), short(950_412), short(4_321), short(999)], ["18.2M", "950K", "4.3K", "999"]);
}

/// `k label · k label`, keys bright and labels dim.
fn keys(pairs: &[(&str, &str)]) -> Vec<(String, C)> {
    let mut out = Vec::new();
    for (i, (k, label)) in pairs.iter().enumerate() {
        if i > 0 { out.push((" · ".into(), GUTTER)) }
        out.push((k.to_string(), FG));
        out.push((format!(" {label}"), DIM));
    }
    out
}

fn center(parts: &[(String, C)], w: usize, bg: Option<C>) -> String {
    let mut s = Seg::new(w, bg);
    s.pad(w.saturating_sub(width(parts)) / 2);
    for (t, c) in parts { s.put(t, *c); }
    s.done()
}

/// `╭ left ───── right ╮`, `w` wide.
fn top(w: usize, c: C, left: &[(String, C)], right: &[(String, C)]) -> String {
    let rw = width(right);
    let tail = if rw > 0 { rw + 2 } else { 0 };
    let mut s = Seg::new(w, None);
    s.put("╭ ", c);
    let mut room = w.saturating_sub(5 + tail);
    for (t, col) in left {
        let before = s.used;
        s.put_in(t, *col, room);
        room -= s.used - before;
    }
    s.put(" ", c);
    let fill = s.room().saturating_sub(1 + tail);
    s.put(&"─".repeat(fill), c);
    if rw > 0 {
        s.put(" ", c);
        for (t, col) in right { s.put(t, *col); }
        s.put(" ", c);
    }
    s.put("╮", c).done()
}

fn bottom(w: usize, c: C) -> String {
    Seg::new(w, None).put(&format!("╰{}╯", "─".repeat(w.saturating_sub(2))), c).done()
}

/// A finished `w - 4` segment between the box's sides.
fn side(c: C, inner: String) -> String {
    format!("{}{inner}{}", Seg::new(2, None).put("│ ", c).done(), Seg::new(2, None).put(" │", c).done())
}

/// Liked Songs, New releases, then every playlist: what the left pane lists.
fn lists_of(s: &State) -> Vec<(Source, String)> {
    let queue = s.lists_queue().then(|| (Source::Queue, "Queue".to_string()));
    [(Source::Liked, "Liked Songs".to_string()), (Source::NewReleases, "New releases".to_string())].into_iter()
        .chain(queue)
        .chain(s.playlists.iter().map(|p| (Source::Playlist(p.id.clone()), p.name.clone())))
        .collect()
}

/// A closed terminal tab quits the player, even when the terminal app starts its shells with hangups ignored.
/// crossterm can't tell: on a terminal that's gone, its read gets 0 bytes or EIO and it loops on waiting for
/// "would block", so the key wait never returns and the music plays on. The terminal does say POLLHUP.
// ponytail: looked at twice a second; drop it if crossterm ever returns that error itself.
#[cfg(unix)]
#[allow(unsafe_code)]
fn quit_on_hangup() {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() { return } // a pipe's end would look the same
    std::thread::spawn(|| loop {
        let mut tty = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd, and a zero timeout.
        if unsafe { libc::poll(&mut tty, 1, 0) } > 0 && tty.revents & libc::POLLHUP != 0 { std::process::exit(0) }
        std::thread::sleep(Duration::from_millis(500));
    });
}

/// macOS delivers media keys on the main run loop, which a terminal program never turns: give it one turn.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn turn_run_loop() {
    use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
    // SAFETY: reads a constant CoreFoundation defines.
    CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.0, true);
}
#[cfg(not(target_os = "macos"))]
fn turn_run_loop() {}

/// The song link onto the clipboard, through whichever copy tool this system has. False when none ran.
/// ponytail: no clipboard over SSH; OSC 52 would reach the local terminal's if that's ever wanted.
fn copy(text: &str) -> bool {
    use std::process::{Command, Stdio};
    let tools: &[&[&str]] = &[&["pbcopy"], &["wl-copy"], &["xclip", "-selection", "clipboard"], &["clip"]];
    tools.iter().any(|t| {
        let Ok(mut child) = Command::new(t[0]).args(&t[1..]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
        let wrote = child.stdin.take().is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|s| s.success()) && wrote
    })
}

fn title(s: &State, lists: &[(Source, String)]) -> String {
    match &s.view {
        Some(Source::Search(q)) => format!("search \u{201c}{q}\u{201d}"),
        Some(Source::Artist { name, .. } | Source::Album { name, .. }) => name.clone(),
        Some(Source::Playlist(id)) if !lists.iter().any(|(src, _)| src == &Source::Playlist(id.clone())) => s.playlist(id).map_or(String::new(), |p| p.name.clone()), // from search
        Some(Source::Queue) => "queue".into(),
        Some(Source::Radio { name, .. }) => format!("{name} radio"),
        Some(v) => lists.iter().find(|(src, _)| src == v).map_or(String::new(), |(_, n)| n.clone()),
        None => String::new(),
    }
}

/// Keeps `cur` inside the `h` rows shown from `top`.
fn scroll(top: &mut usize, cur: usize, h: usize) {
    if cur < *top { *top = cur }
    if cur >= *top + h { *top = cur + 1 - h }
}

fn clock(ms: u32) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// The lines of `next` that differ from `prev`, each moved to its row; nothing when nothing changed.
/// One synchronized update, so the terminal never shows half a frame.
fn paint(prev: &[String], next: &[String]) -> String {
    let changed: String = next.iter().enumerate()
        .filter(|(i, l)| prev.get(*i) != Some(l))
        .map(|(i, l)| format!("\x1b[{};1H{l}\x1b[0m", i + 1))
        .collect();
    if changed.is_empty() { changed } else { format!("\x1b[?2026h{changed}\x1b[?2026l") }
}

fn width(parts: &[(String, C)]) -> usize {
    parts.iter().map(|(t, _)| fit(t, usize::MAX).1).sum()
}

/// `text` cut to `room` cells, with … where it was cut. Control characters are dropped.
fn fit(text: &str, room: usize) -> (String, usize) {
    let cells = |c: char| c.width().unwrap_or(0);
    let chars = || text.chars().filter(|c| !c.is_control());
    let full: usize = chars().map(cells).sum();
    if full <= room { return (chars().collect(), full) }
    if room == 0 { return (String::new(), 0) }
    let (mut s, mut used) = (String::new(), 0);
    for c in chars() {
        if used + cells(c) + 1 > room { break }
        s.push(c);
        used += cells(c);
    }
    s.push('…');
    (s, used + 1)
}

fn bg_code(bg: Option<C>) -> String {
    bg.map_or("\x1b[49m".into(), |c| format!("\x1b[48;2;{};{};{}m", c.0, c.1, c.2))
}

/// A run of cells on one background, exactly `max` wide once done.
struct Seg { out: String, used: usize, max: usize, bg: Option<C> }

impl Seg {
    /// `bg` None keeps the terminal's own background.
    fn new(max: usize, bg: Option<C>) -> Self {
        Seg { out: bg_code(bg), used: 0, max, bg }
    }
    fn room(&self) -> usize { self.max - self.used }
    fn put(&mut self, text: &str, c: C) -> &mut Self { self.put_in(text, c, self.room()) }
    fn put_in(&mut self, text: &str, c: C, room: usize) -> &mut Self {
        let (s, w) = fit(text, room.min(self.room()));
        self.out += &format!("\x1b[38;2;{};{};{}m{s}", c.0, c.1, c.2);
        self.used += w;
        self
    }
    /// `text` in exactly `w` cells, the last two kept clear as the gap to the next column.
    fn col(&mut self, text: &str, c: C, w: usize) -> &mut Self {
        let start = self.used;
        self.put_in(text, c, if w > 2 { w - 2 } else { w });
        self.pad(w - (self.used - start))
    }
    fn pad(&mut self, n: usize) -> &mut Self {
        let n = n.min(self.room());
        self.out += &" ".repeat(n);
        self.used += n;
        self
    }
    /// Another finished segment of `w` cells (with its own background), then back to ours.
    fn raw(&mut self, seg: String, w: usize) -> &mut Self {
        self.out += &seg;
        self.out += &bg_code(self.bg);
        self.used += w.min(self.room());
        self
    }
    /// `parts` flush right, `margin` cells from the edge.
    fn right(&mut self, parts: &[(String, C)], margin: usize) -> &mut Self {
        self.pad(self.room().saturating_sub(width(parts) + margin));
        for (t, c) in parts { self.put(t, *c); }
        self
    }
    fn done(&mut self) -> String {
        self.pad(self.room());
        std::mem::take(&mut self.out)
    }
}

/// Raw mode on the alternate screen, cursor hidden, no line wrap; all undone on drop (and on a panic).
struct Term;

impl Term {
    fn enter() -> std::io::Result<Term> {
        terminal::enable_raw_mode()?;
        print!("\x1b[?1049h\x1b[?25l\x1b[?7l");
        std::io::stdout().flush()?;
        Ok(Term)
    }
}

/// Hands the terminal back (main screen, cursor, cooked mode) from outside the player, before a forced exit.
pub fn leave_terminal() {
    drop(Term)
}

impl Drop for Term {
    fn drop(&mut self) {
        print!("\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
        let _ = std::io::stdout().flush();
        let _ = terminal::disable_raw_mode();
    }
}

#[test]
fn fit_cuts_by_cells_and_rows_come_out_exactly_wide() {
    assert_eq!(fit("hello", 5), ("hello".into(), 5));
    assert_eq!(fit("hello", 3), ("he…".into(), 3));
    assert_eq!(fit("日本語", 4), ("日…".into(), 3)); // wide characters count two
    assert_eq!(clock(185_400), "3:05");
    let mut s = Seg::new(20, None);
    s.col("a long song name here", FG, 8).right(&[(clock(61_000), DIM)], 0);
    assert_eq!(s.used, 20);
    let boxed = top(30, red(), &[("a title far too long for it".into(), FG)], &[("68".into(), DIM)]);
    let plain: String = boxed.split('\x1b').map(|p| p.split_once('m').map_or(p, |(_, t)| t)).collect();
    assert_eq!(fit(&plain, usize::MAX).1, 30); // titled borders stay exactly as wide as their box
    let (a, b) = (vec!["x".to_string(), "y".into()], vec!["x".to_string(), "z".into()]);
    assert_eq!(paint(&a, &a), ""); // an unchanged screen sends nothing
    assert_eq!(paint(&a, &b), "\x1b[?2026h\x1b[2;1Hz\x1b[0m\x1b[?2026l"); // only row 2
    let mut top = 0;
    scroll(&mut top, 30, 10);
    assert_eq!(top, 21); // the cursor sits on the last visible row
}

