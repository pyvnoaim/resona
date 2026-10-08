//! `resona cli`: the player full-screen in a terminal, dressed like flow (a centred card of titled boxes, Wired's red).
//! Shares the engine, `open_list` and `command` with the window, so it plays the same queue the same way.

use std::{
    io::Write,
    sync::{Arc, OnceLock},
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use eframe::egui;
use librespot::playback::mixer::Mixer;
use unicode_width::UnicodeWidthChar;

use crate::{command, engine::*, open_list, settings::*, state::*, ui::short_err};

type C = (u8, u8, u8);
const PANEL: C = (0x1e, 0x21, 0x28);
const CURSOR: C = (0x25, 0x2b, 0x35);
const FG: C = (0xc9, 0xd1, 0xd9);
const DIM: C = (0x6b, 0x72, 0x80);
const SUB: C = (0x5f, 0x67, 0x73);
const FAINT: C = (0x4a, 0x51, 0x5b);
const GUTTER: C = (0x34, 0x3a, 0x44);
const RED: C = (0xe0, 0x28, 0x2e);

const KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"), ("tab", "pane"), ("enter", "play"), ("/", "search"), ("q", "quit"),
    ("space", "pause"), ("n b", "next / back"), ("←→", "seek"), ("+-", "volume"), ("s", "shuffle"), ("r", "repeat"),
];

#[derive(Clone, Copy, PartialEq)]
enum Pane { Lists, Songs }

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
}

pub fn run() {
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
        pane: Pane::Songs, side: 0, side_top: 0, row: 0, row_top: 0, search: None, seen: None, body_h: 1,
        volume: setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7),
    };
    let mut restored = false;
    let mut out = std::io::stdout();
    loop {
        if let (Some(e), false) = (slot.get(), restored) {
            restored = true;
            e.mixer.set_volume((ui.volume * u16::MAX as f32) as u16);
            let last = setting("last").and_then(|k| Source::from_key(&k)).unwrap_or(Source::Liked);
            open_list(rt.handle(), e, &state, last, &ctx);
        }
        let (w, h) = terminal::size().unwrap_or((80, 24));
        let frame = ui.draw(&state.lock().unwrap(), w as usize, h as usize);
        let _ = out.write_all(frame.as_bytes()).and_then(|_| out.flush());
        // Idle costs nothing: the clock only ticks while something moves.
        let busy = { let s = state.lock().unwrap(); s.since.is_some() || s.loading || (slot.get().is_none() && s.error.is_none()) };
        if !event::poll(Duration::from_millis(if busy { 250 } else { 2000 })).unwrap_or(false) { continue }
        let Ok(Event::Key(k)) = event::read() else { continue }; // a resize just redraws
        if k.kind == KeyEventKind::Press && !ui.key(k, slot.get(), &state, rt.handle(), &ctx) { break }
    }
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
        if k.code == KeyCode::Char('q') { return false }
        let Some(e) = e else { return true };
        let (lists, rows) = { let s = state.lock().unwrap(); (1 + s.playlists.len(), s.tracks.len()) };
        let page = self.body_h.max(1);
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
                    let (list, src) = (s.tracks.clone(), s.view.clone());
                    s.start(&e.player, list, Some(self.row), src);
                }
            }
            KeyCode::Char(' ') => cmd(Cmd::Toggle),
            KeyCode::Char('n') => cmd(Cmd::Next),
            KeyCode::Char('b') => cmd(Cmd::Prev),
            KeyCode::Right => cmd(Cmd::SeekBy(10_000)),
            KeyCode::Left => cmd(Cmd::SeekBy(-10_000)),
            KeyCode::Char(c @ ('+' | '=' | '-')) => {
                self.volume = (self.volume + if c == '-' { -0.05 } else { 0.05 }).clamp(0.0, 1.0);
                e.mixer.set_volume((self.volume * u16::MAX as f32) as u16);
                set_setting("volume", &format!("{:.2}", self.volume));
            }
            KeyCode::Char('s') => state.lock().unwrap().toggle_shuffle(),
            KeyCode::Char('r') => state.lock().unwrap().cycle_repeat(),
            KeyCode::Char('/') => self.search = Some(String::new()),
            _ => {}
        }
        true
    }

    /// The whole screen, every cell, as one write: a centred card of boxes on the terminal's own background.
    fn draw(&mut self, s: &State, w: usize, h: usize) -> String {
        let blank = || Seg::new(w, None).done();
        if w < 60 || h < 18 {
            let mut lines = vec![center(&[("make the window a little bigger".into(), DIM)], w)];
            lines.resize_with(h, blank);
            return frame(lines);
        }
        let lists = lists_of(s);
        if s.view != self.seen {
            // A new list: start at its top, and point the sidebar at it.
            (self.row, self.row_top, self.seen) = (0, 0, s.view.clone());
            if let Some(i) = lists.iter().position(|(src, _)| Some(src) == s.view.as_ref()) { self.side = i }
        }
        self.side = self.side.min(lists.len().saturating_sub(1));
        self.row = self.row.min(s.tracks.len().saturating_sub(1));

        let cw = (w - 2).min(120);
        let x0 = (w - cw) / 2;
        let line = |content: String| format!("{}{content}{}", Seg::new(x0, None).done(), Seg::new(w - x0 - cw, None).done());
        let body_h = (h - 14).min(36);
        self.body_h = body_h;
        let mut lines: Vec<String> = Vec::with_capacity(h);
        for _ in 0..(h - 12 - body_h) / 2 { lines.push(blank()) }

        // Brand, then whatever the engine has to say.
        lines.push(line(center(&[("resona".into(), FG), (concat!("  v", env!("CARGO_PKG_VERSION")).into(), GUTTER)], cw)));
        let note = match (&s.error, s.status.is_empty(), &s.notice) {
            (Some(err), _, _) => vec![(short_err(err), RED)],
            (None, false, _) => vec![(s.status.clone(), DIM)],
            (None, true, Some(n)) => vec![(n.clone(), SUB)],
            _ => vec![],
        };
        lines.push(line(center(&note, cw)));

        // Lists and songs, side by side; the focused box is the red one.
        let lw = (cw / 4).clamp(20, 32);
        let rw = cw - lw - 1;
        let edge = |pane| if self.pane == pane { RED } else { FAINT };
        let count = match (s.loading, s.expected) {
            (true, Some(n)) => format!("loading {}/{n}", s.tracks.len()),
            (true, None) => "loading…".into(),
            _ => format!("{} songs", s.tracks.len()),
        };
        lines.push(line(format!("{} {}",
            top(lw, edge(Pane::Lists), &[("library".into(), FG)], &[(lists.len().to_string(), DIM)]),
            top(rw, edge(Pane::Songs), &[(title(s, &lists), FG)], &[(count, DIM)]))));
        scroll(&mut self.side_top, self.side, body_h);
        scroll(&mut self.row_top, self.row, body_h);
        let now_id = s.now.as_ref().and_then(|t| t.id.as_ref());
        let num_w = s.tracks.len().max(10).to_string().len();
        let rest = (rw - 4).saturating_sub(num_w + 2 + 2 + 6);
        let (name_w, artist_w) = (rest * 11 / 20, rest - rest * 11 / 20);
        let cursor_bg = |on: bool, pane| if !on { None } else if self.pane == pane { Some(CURSOR) } else { Some(PANEL) };
        for i in 0..body_h {
            let mut left = Seg::new(lw - 4, None);
            if let Some((src, name)) = lists.get(self.side_top + i) {
                let open = s.view.as_ref() == Some(src);
                left = Seg::new(lw - 4, cursor_bg(self.side_top + i == self.side, Pane::Lists));
                left.put(name, if open { RED } else { DIM });
            }
            let at = self.row_top + i;
            let mut right = Seg::new(rw - 4, None);
            if let Some(t) = s.tracks.get(at) {
                let playing = t.id.is_some() && t.id.as_ref() == now_id;
                let gone = t.id.as_ref().is_some_and(|id| s.unplayable.contains(id));
                let liked = t.id.as_ref().is_some_and(|id| s.liked.contains(id));
                let num = if playing { "▶".to_string() } else { (at + 1).to_string() };
                right = Seg::new(rw - 4, cursor_bg(at == self.row, Pane::Songs));
                right.col(&format!("{num:>num_w$}"), if playing { RED } else { GUTTER }, num_w + 2)
                    .col(&t.name, if gone { FAINT } else if playing { RED } else { FG }, name_w)
                    .col(&t.artist_line(), if gone { FAINT } else { SUB }, artist_w)
                    .col(if liked { "♥" } else { "" }, RED, 2)
                    .right(&[(clock(t.duration_ms), DIM)], 0);
            } else if i == 0 && s.tracks.is_empty() {
                right.put(if s.loading || s.view.is_none() { "…" } else { "nothing here" }, GUTTER);
            }
            lines.push(line(format!("{} {}", side(edge(Pane::Lists), left.done()), side(edge(Pane::Songs), right.done()))));
        }
        lines.push(line(format!("{} {}", bottom(lw, edge(Pane::Lists)), bottom(rw, edge(Pane::Songs)))));
        lines.push(blank());

        // The player: song and how it's playing, then the dotted progress line.
        let state_word = match (&s.now, s.since.is_some()) {
            (None, _) => "idle",
            (Some(_), true) => "▶ playing",
            (Some(_), false) => "‖ paused",
        };
        lines.push(line(top(cw, SUB, &[(state_word.into(), FG)], &[(format!("vol {:.0}%", self.volume * 100.0), DIM)])));
        let flags: Vec<&str> = [(s.shuffle, "shuffle"), (s.repeat == Repeat::All, "repeat"), (s.repeat == Repeat::One, "repeat one")]
            .into_iter().filter(|(on, _)| *on).map(|(_, l)| l).collect();
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
                bar.put(&a, DIM).put(" ", DIM).put(&"⣿".repeat(filled), RED).put(&"⣀".repeat(bar_w - filled), GUTTER).put(" ", DIM).put(&b, DIM);
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
                lines.push(line(center(&[("/ ".into(), RED), (q.clone(), FG), ("▌".into(), RED)], cw)));
                lines.push(line(center(&keys(&[("enter", "search"), ("esc", "cancel")]), cw)));
            }
            None => {
                lines.push(line(center(&keys(&KEYS[..5]), cw)));
                lines.push(line(center(&keys(&KEYS[5..]), cw)));
            }
        }
        lines.resize_with(h, blank);
        frame(lines)
    }
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

fn center(parts: &[(String, C)], w: usize) -> String {
    let mut s = Seg::new(w, None);
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

/// Liked Songs, then every playlist: what the left pane lists.
fn lists_of(s: &State) -> Vec<(Source, String)> {
    std::iter::once((Source::Liked, "Liked Songs".to_string()))
        .chain(s.playlists.iter().map(|p| (Source::Playlist(p.id.clone()), p.name.clone())))
        .collect()
}

fn title(s: &State, lists: &[(Source, String)]) -> String {
    match &s.view {
        Some(Source::Search(q)) => format!("search \u{201c}{q}\u{201d}"),
        Some(Source::Artist { name, .. } | Source::Album { name, .. }) => name.clone(),
        Some(Source::Queue) => "queue".into(),
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

fn frame(lines: Vec<String>) -> String {
    // Synchronized update, so the terminal never shows half a frame.
    format!("\x1b[?2026h\x1b[H{}\x1b[0m\x1b[?2026l", lines.join("\x1b[0m\r\n"))
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
struct Seg { out: String, used: usize, max: usize }

impl Seg {
    /// `bg` None keeps the terminal's own background.
    fn new(max: usize, bg: Option<C>) -> Self {
        Seg { out: bg_code(bg), used: 0, max }
    }
    fn room(&self) -> usize { self.max - self.used }
    fn put(&mut self, text: &str, c: C) -> &mut Self { self.put_in(text, c, self.room()) }
    fn put_in(&mut self, text: &str, c: C, room: usize) -> &mut Self {
        let (s, w) = fit(text, room.min(self.room()));
        self.out += &format!("\x1b[38;2;{};{};{}m{s}", c.0, c.1, c.2);
        self.used += w;
        self
    }
    /// `text` in exactly `w` cells.
    fn col(&mut self, text: &str, c: C, w: usize) -> &mut Self {
        let start = self.used;
        self.put_in(text, c, w);
        self.pad(w - (self.used - start))
    }
    fn pad(&mut self, n: usize) -> &mut Self {
        let n = n.min(self.room());
        self.out += &" ".repeat(n);
        self.used += n;
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
    let boxed = top(30, RED, &[("a title far too long for it".into(), FG)], &[("68".into(), DIM)]);
    let plain: String = boxed.split('\x1b').map(|p| p.split_once('m').map_or(p, |(_, t)| t)).collect();
    assert_eq!(fit(&plain, usize::MAX).1, 30); // titled borders stay exactly as wide as their box
    let mut top = 0;
    scroll(&mut top, 30, 10);
    assert_eq!(top, 21); // the cursor sits on the last visible row
}
