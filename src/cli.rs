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

use crate::{command, engine::*, open_list, settings::*, state::*, toggle_like, ui::short_err};

type C = (u8, u8, u8);
const BG: C = (0x1a, 0x1d, 0x23);
const RULE: C = (0x2c, 0x31, 0x3a);
const PANEL: C = (0x1e, 0x21, 0x28);
/// The selected row: clearly lit in the focused pane, a softer `DULL` in the other.
const CURSOR: C = (0x4a, 0x1d, 0x23);
const DULL: C = (0x2e, 0x21, 0x26);
const FG: C = (0xc9, 0xd1, 0xd9);
/// The playing song's name and ▶ on the selected (red) row, where red on red would vanish.
const BRIGHT: C = (0xf4, 0xf6, 0xf8);
const DIM: C = (0x6b, 0x72, 0x80);
const SUB: C = (0x5f, 0x67, 0x73);
const FAINT: C = (0x4a, 0x51, 0x5b);
const GUTTER: C = (0x34, 0x3a, 0x44);
const RED: C = (0xe0, 0x28, 0x2e);

const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"), ("tab", "pane"), ("enter", "play"), ("/", "search"), (".", "playing"), ("q", "quit"),
    ("space", "pause"), ("n b", "next / back"), ("←→", "seek"), ("+-", "volume"), ("s", "shuffle"), ("r", "repeat"),
    ("l", "like"), ("a", "queue"), ("o", "sort"), ("t", "style"), ("^L", "redraw"),
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
        pane: Pane::Songs, side: 0, side_top: 0, row: 0, row_top: 0, search: None, seen: None, body_h: 1, jump: None, sel: None, repaint: false, born: Instant::now(),
        style: if setting("cli_style").as_deref() == Some("board") { Style::Board } else { Style::Card },
        volume: setting("volume").and_then(|v| v.parse().ok()).unwrap_or(0.7),
    };
    let mut restored = false;
    let mut out = std::io::stdout();
    // What's on screen now, so each tick only sends the lines that changed (the clock, mostly).
    let (mut shown, mut shown_size): (Vec<String>, (u16, u16)) = (Vec::new(), (0, 0));
    loop {
        if let (Some(e), false) = (slot.get(), restored) {
            restored = true;
            e.mixer.set_volume((ui.volume * u16::MAX as f32) as u16);
            let last = setting("last").and_then(|k| Source::from_key(&k)).unwrap_or(Source::Liked);
            open_list(rt.handle(), e, &state, last, &ctx);
        }
        let (w, h) = terminal::size().unwrap_or((80, 24));
        if (w, h) != shown_size || std::mem::take(&mut ui.repaint) { (shown, shown_size) = (Vec::new(), (w, h)) } // resize or Ctrl+L: everything
        let lines = ui.draw(&state.lock().unwrap(), w as usize, h as usize, slot.get().is_none());
        let update = paint(&shown, &lines);
        if !update.is_empty() { let _ = out.write_all(update.as_bytes()).and_then(|_| out.flush()); }
        shown = lines;
        // Idle costs nothing: the clock only ticks while something moves.
        // The splash at ~12 fps; spinners and the clock at 4.
        let tick = {
            let s = state.lock().unwrap();
            if slot.get().is_none() && s.error.is_none() { 80 } else if s.loading || s.since.is_some() { 250 } else { 2000 }
        };
        if !event::poll(Duration::from_millis(tick)).unwrap_or(false) { continue }
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
        if k.code == KeyCode::Char('l') && k.modifiers.contains(KeyModifiers::CONTROL) { self.repaint = true; return true }
        let Some(e) = e else { return true };
        let (lists, rows) = { let s = state.lock().unwrap(); (1 + s.playlists.len(), s.tracks.len()) };
        let page = self.body_h.max(1);
        let song_at = |row: usize| state.lock().unwrap().tracks.get(row).and_then(|t| t.id.clone());
        let (held, before) = (song_at(self.row), self.row);
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
                    s.start(&e.player(), list, Some(self.row), src);
                }
            }
            KeyCode::Char(' ') => cmd(Cmd::Toggle),
            KeyCode::Char('n') => cmd(Cmd::Next),
            KeyCode::Char('b') => cmd(Cmd::Prev),
            KeyCode::Right => cmd(Cmd::SeekBy(10_000)),
            KeyCode::Left => cmd(Cmd::SeekBy(-10_000)),
            KeyCode::Char(c @ ('+' | '=' | '-')) => {
                // Whole 5% steps, like the window's + and -: 54% goes up to 60, down to 50.
                self.volume = ((self.volume + if c == '-' { -0.05 } else { 0.05 }) * 20.0).round().clamp(0.0, 20.0) / 20.0;
                e.mixer.set_volume((self.volume * u16::MAX as f32) as u16);
                set_setting("volume", &format!("{:.2}", self.volume));
            }
            KeyCode::Char('s') => cmd(Cmd::Shuffle),
            KeyCode::Char('r') => state.lock().unwrap().cycle_repeat(),
            KeyCode::Char('/') => self.search = Some(String::new()),
            // The selected song, or the playing one while the lists have focus.
            KeyCode::Char('l') => {
                let id = {
                    let s = state.lock().unwrap();
                    let t = if self.pane == Pane::Songs { s.tracks.get(self.row) } else { s.now.as_ref() };
                    t.and_then(|t| t.id.clone())
                };
                if let Some(id) = id { toggle_like(rt, e, state, id) }
            }
            KeyCode::Char('a') if self.pane == Pane::Songs => {
                let mut s = state.lock().unwrap();
                if let Some(t) = s.tracks.get(self.row).cloned() {
                    s.notice = Some(format!("Queued: \u{201c}{}\u{201d}", t.name));
                    s.enqueue(&e.player(), t, false);
                }
            }
            KeyCode::Char('.') => {
                let (src, id) = { let s = state.lock().unwrap(); (s.queue_src.clone(), s.now.as_ref().and_then(|t| t.id.clone())) };
                if let Some(src) = src { open_list(rt, e, state, src, ctx) } // no-op when it's already open
                (self.jump, self.pane) = (id, Pane::Songs);
            }
            KeyCode::Char('o') => state.lock().unwrap().cycle_sort(), // the cursor follows its song (`sel`)
            KeyCode::Char('t') => {
                self.style = if self.style == Style::Card { Style::Board } else { Style::Card };
                set_setting("cli_style", if self.style == Style::Board { "board" } else { "card" });
            }
            _ => {}
        }
        // Moving picks a new song; anything else keeps the one that was under the cursor.
        self.sel = if self.row != before { song_at(self.row) } else { held };
        true
    }

    /// The whole screen, every cell, as one write.
    fn draw(&mut self, s: &State, w: usize, h: usize, starting: bool) -> Vec<String> {
        if w < 60 || h < 18 {
            let mut lines = vec![center(&[("make the window a little bigger".into(), DIM)], w, None)];
            lines.resize_with(h, || Seg::new(w, None).done());
            return lines;
        }
        if starting && s.error.is_none() { return self.splash(s, w, h) }
        let lists = lists_of(s);
        if s.view != self.seen {
            // A new list: start at its top, and point the sidebar at it.
            (self.row, self.row_top, self.seen, self.sel) = (0, 0, s.view.clone(), None);
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
        self.row = self.row.min(s.tracks.len().saturating_sub(1));
        // The library is as wide as its longest name, within reason.
        let longest = lists.iter().map(|(_, n)| n.width()).max().unwrap_or(0);
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
        lines.extend([row(&[(bars, RED)]), row(&[]), row(&[(format!("{name:<11}"), BRIGHT)]), row(&[]), row(&[(status.into(), DIM)]), row(&[]), row(&[("q quits".into(), GUTTER)])]);
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
        let note = match (&s.error, s.status.is_empty(), &s.notice) {
            (Some(err), _, _) => vec![(short_err(err), RED)],
            (None, false, _) => vec![(s.status.clone(), DIM)],
            (None, true, Some(n)) => vec![(n.clone(), SUB)],
            _ => vec![],
        };
        lines.push(line(center(&note, cw, None)));

        // Lists and songs, side by side; the focused box is the red one.
        let lw = (longest + 4).clamp(22, (cw / 3).max(22));
        let rw = cw - lw - 1;
        let edge = |pane| if self.pane == pane { RED } else { FAINT };
        let count = count(s, self.spin());
        lines.push(line(format!("{} {}",
            top(lw, edge(Pane::Lists), &[("library".into(), FG)], &[(lists.len().to_string(), DIM)]),
            top(rw, edge(Pane::Songs), &[(title(s, lists), FG)], &[(count, DIM)]))));
        scroll(&mut self.side_top, self.side, body_h);
        scroll(&mut self.row_top, self.row, body_h);
        let cursor_bg = |on: bool, pane| if !on { None } else if self.pane == pane { Some(CURSOR) } else { Some(DULL) };
        for i in 0..body_h {
            let mut left = Seg::new(lw - 4, None);
            if let Some((src, name)) = lists.get(self.side_top + i) {
                let open = s.view.as_ref() == Some(src);
                let sel = self.side_top + i == self.side;
                left = Seg::new(lw - 4, cursor_bg(sel, Pane::Lists));
                left.put(name, if open { RED } else if sel { FG } else { DIM });
            }
            let at = self.row_top + i;
            let mut right = Seg::new(rw - 4, None);
            if at < s.tracks.len() {
                right = song_row(s, at, rw - 4, false, cursor_bg(at == self.row, Pane::Songs), at == self.row);
            } else if i == 0 && s.tracks.is_empty() {
                right.put(if s.loading { self.spin() } else if s.view.is_none() { "…" } else { "nothing here" }, GUTTER);
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
                lines.push(line(center(&[("/ ".into(), RED), (q.clone(), FG), ("▌".into(), RED)], cw, None)));
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
        let rule = || Seg::new(w, Some(BG)).pad(2).put(&"─".repeat(inner), RULE).done();

        // Header: brand hard left, what's open hard right.
        lines.push(Seg::new(w, Some(BG)).done());
        let mut status: Vec<(String, C)> = Vec::new();
        if let Some(err) = &s.error { status.push((format!("{}  ·  ", short_err(err)), RED)) }
        if !s.status.is_empty() {
            status.push((s.status.clone(), DIM));
        } else {
            status.push((title(s, lists), DIM));
            status.push((format!("  ·  {}", count(s, self.spin())), SUB));
            if let Some(n) = &s.notice { status.push((format!("  ·  {n}"), SUB)) }
        }
        let mut head = Seg::new(w, Some(BG));
        head.pad(2).put("▌ ", RED).put("resona", FG).put(concat!(" v", env!("CARGO_PKG_VERSION")), GUTTER);
        lines.push(head.right(&status, 2).done());
        lines.push(rule());

        // Body: lists on the left, songs on the right. The bar only marks what's open and what's playing.
        let body_h = h - 6;
        self.body_h = body_h;
        let lw = (longest + 2).clamp(18, (inner / 3).max(18));
        let rw = inner - lw - 3;
        scroll(&mut self.side_top, self.side, body_h);
        scroll(&mut self.row_top, self.row, body_h);
        let cursor_bg = |on: bool, pane| Some(if !on { BG } else if self.pane == pane { CURSOR } else { DULL });
        for i in 0..body_h {
            let mut left = Seg::new(lw, Some(BG));
            if let Some((src, name)) = lists.get(self.side_top + i) {
                let open = s.view.as_ref() == Some(src);
                let sel = self.side_top + i == self.side;
                left = Seg::new(lw, cursor_bg(sel, Pane::Lists));
                left.put(if open { "▌ " } else { "  " }, RED).put(name, if open || sel { FG } else { DIM });
            }
            let at = self.row_top + i;
            let mut right = Seg::new(rw, Some(BG));
            if at < s.tracks.len() {
                right = song_row(s, at, rw, true, cursor_bg(at == self.row, Pane::Songs), at == self.row);
            } else if i == 0 && s.tracks.is_empty() {
                right.pad(2).put(if s.loading { self.spin() } else if s.view.is_none() { "…" } else { "nothing here" }, FAINT);
            }
            lines.push(format!("{}{}{}{}{}", Seg::new(2, Some(BG)).done(), left.done(), Seg::new(3, Some(BG)).done(), right.done(), Seg::new(2, Some(BG)).done()));
        }

        // Player: the song left, where it is and how it's playing right.
        lines.push(rule());
        let mut right: Vec<(String, C)> = Vec::new();
        if let Some(t) = &s.now {
            let (pos, dur) = (s.position(), t.duration_ms.max(1));
            let bar_w = (inner / 5).clamp(10, 40);
            let filled = (pos as usize * bar_w / dur as usize).min(bar_w);
            right.push((format!("{}  ", clock(pos)), DIM));
            right.push(("━".repeat(filled), RED));
            right.push(("─".repeat(bar_w - filled), RULE));
            right.push((format!("  {}", clock(dur)), DIM));
        }
        for (on, label) in [(s.shuffle, "shuffle"), (s.repeat == Repeat::All, "repeat"), (s.repeat == Repeat::One, "repeat one")] {
            if on { right.push((format!("   {label}"), SUB)) }
        }
        right.push((format!("   vol {:.0}%", self.volume * 100.0), DIM));
        let mut player = Seg::new(w, Some(PANEL));
        player.pad(2);
        let room = player.room().saturating_sub(width(&right) + 4);
        match &s.now {
            Some(t) => {
                let mut song = Seg::new(room, Some(PANEL));
                song.put("▌ ", RED).put(if s.since.is_some() { "▶  " } else { "‖  " }, FG).put(&t.name, FG).put("  ", FG).put(&t.artist_line(), SUB);
                player.raw(song.done(), room);
            }
            None => { player.put("  nothing playing", FAINT); }
        }
        lines.push(player.right(&right, 2).done());

        // Keys, or the search being typed.
        let mut keys = Seg::new(w, Some(BG));
        keys.pad(2);
        match &self.search {
            Some(q) => { keys.put("/ ", RED).put(q, FG).put("▌", RED).put("   enter search   esc cancel", DIM); }
            None => for (i, (k, label)) in KEYS.iter().enumerate() {
                keys.put(if i == 0 { "" } else { "   " }, DIM).put(k, FG).put(" ", DIM).put(label, DIM);
            },
        }
        lines.push(keys.done());
        lines
    }
}

/// One song row, `w` wide, the same in both styles: number (▶ when playing), name, artists, heart, length.
/// `bar` adds the board's red bar in front of the playing song; `bg` is the row's background (lit when `sel`).
fn song_row(s: &State, at: usize, w: usize, bar: bool, bg: Option<C>, sel: bool) -> Seg {
    let t = &s.tracks[at];
    let playing = t.id.is_some() && t.id == s.now.as_ref().and_then(|n| n.id.clone());
    let gone = t.id.as_ref().is_some_and(|id| s.unplayable.contains(id));
    let liked = t.id.as_ref().is_some_and(|id| s.liked.contains(id));
    let num_w = s.tracks.len().max(10).to_string().len();
    let rest = w.saturating_sub(if bar { 2 } else { 0 } + num_w + 2 + 2 + 6);
    let (name_w, artist_w) = (rest * 11 / 20, rest - rest * 11 / 20);
    // Faded songs and the number column step up a shade on the selected row; red would vanish on it.
    let (faded, sub) = if sel { (FG, DIM) } else { (FAINT, SUB) };
    let accent = if sel { BRIGHT } else { RED };
    let num = if playing { "▶".to_string() } else { (at + 1).to_string() };
    let mut row = Seg::new(w, bg);
    if bar { row.put(if playing { "▌ " } else { "  " }, RED); }
    row.col(&format!("{num:>num_w$}"), if playing { accent } else { faded }, num_w + 2)
        .col(&t.name, if playing { accent } else if gone { faded } else { FG }, name_w)
        .col(&t.artist_line(), if gone { faded } else { sub }, artist_w)
        .col(if liked { "♥" } else { "" }, accent, 2)
        .right(&[(clock(t.duration_ms), DIM)], 0);
    row
}

fn count(s: &State, spin: &str) -> String {
    match (s.loading, s.expected) {
        (true, Some(n)) => format!("{spin} loading {}/{n}", s.tracks.len()),
        (true, None) => format!("{spin} loading"),
        _ => match s.sort_label() {
            Some(l) => format!("{} songs · {l}", s.tracks.len()),
            None => format!("{} songs", s.tracks.len()),
        },
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
    let (a, b) = (vec!["x".to_string(), "y".into()], vec!["x".to_string(), "z".into()]);
    assert_eq!(paint(&a, &a), ""); // an unchanged screen sends nothing
    assert_eq!(paint(&a, &b), "\x1b[?2026h\x1b[2;1Hz\x1b[0m\x1b[?2026l"); // only row 2
    let mut top = 0;
    scroll(&mut top, 30, 10);
    assert_eq!(top, 21); // the cursor sits on the last visible row
}
