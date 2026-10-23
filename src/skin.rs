//! Skins: how the app looks. Each skin has a dark and a light palette, its own fonts and its own
//! widgets (window frames, covers, buttons, sliders, rows). main.rs lays things out; this draws them.
//!
//! - Wired: Lain-style cyber/pixel. Square windows with title bars, red on black, pixel fonts.
//! - Swiss: print-style. No chrome, a strict grid, huge type, one red rule.
//! - Ascii: a teletype printout. Everything is a 7-bit character: +--[ frames ]--+, <3, [####----], Courier.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, pos2,
    vec2,
};

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum Skin {
    #[default]
    Wired,
    Swiss,
    Ascii,
}

impl Skin {
    pub const ALL: [Skin; 3] = [Skin::Wired, Skin::Swiss, Skin::Ascii];

    /// The terminal looks: block cursors, zero-padded times, capitals.
    pub fn blocky(self) -> bool {
        self != Skin::Swiss
    }
    pub fn label(self) -> &'static str {
        match self {
            Skin::Wired => "Wired",
            Skin::Swiss => "Swiss",
            Skin::Ascii => "ASCII",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Pal {
    pub ground: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub hover: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub online: Color32,
    /// The playing row: its fill and its text.
    pub selected: Color32,
    pub on_selected: Color32,
    /// Slider rails.
    pub track: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The accent colour. Red is each skin's own; the others swap it in, one shade for dark themes, one for light.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum Accent {
    #[default]
    Red,
    Blue,
    Teal,
    Violet,
    Amber,
    Pink,
    /// No colour: the accent is the skin's own ink, the text colour.
    Mono,
    /// Your own colour: each skin and theme shows a readable shade of it (`readable`).
    Custom([u8; 3]),
}

impl Accent {
    /// The named ones, in swatch order.
    pub const ALL: [Accent; 7] = [Accent::Red, Accent::Blue, Accent::Teal, Accent::Violet, Accent::Amber, Accent::Pink, Accent::Mono];

    /// How it's saved and shown: the name, or `#rrggbb`.
    pub fn key(self) -> String {
        match self {
            Accent::Custom([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
            a => format!("{a:?}"),
        }
    }

    pub fn parse(s: &str) -> Option<Accent> {
        Accent::ALL.into_iter().find(|a| a.key() == s).or_else(|| hex(s).map(Accent::Custom))
    }

    /// (dark theme, light theme); None keeps the skin's red.
    fn shades(self) -> Option<(Color32, Color32)> {
        match self {
            Accent::Red => None,
            Accent::Blue => Some((rgb(70, 140, 255), rgb(22, 82, 204))),
            Accent::Teal => Some((rgb(40, 200, 200), rgb(0, 122, 132))), // not green: the online dot is
            Accent::Violet => Some((rgb(165, 110, 255), rgb(110, 52, 206))),
            Accent::Amber => Some((rgb(255, 170, 30), rgb(176, 104, 0))),
            Accent::Pink => Some((rgb(255, 84, 170), rgb(196, 26, 116))),
            Accent::Mono => None, // each palette's own text, in `pal_of`
            Accent::Custom(_) => None, // made per skin and theme in `custom_pal`
        }
    }
}

/// `#rgb` or `#rrggbb` (the `#` optional) as red, green, blue.
pub fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if !s.chars().all(|c| c.is_ascii_hexdigit()) { return None }
    let d = |at: usize, n: usize| u8::from_str_radix(&s[at..at + n], 16).ok();
    match s.len() {
        6 => Some([d(0, 2)?, d(2, 2)?, d(4, 2)?]),
        3 => Some([d(0, 1)? * 17, d(1, 1)? * 17, d(2, 1)? * 17]),
        _ => None,
    }
}

/// WCAG contrast between two colours, 1 to 21.
fn contrast(a: Color32, b: Color32) -> f32 {
    let lin = |v: u8| { let v = v as f32 / 255.0; if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    let lum = |c: Color32| 0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b());
    let (x, y) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
    (x + 0.05) / (y + 0.05)
}

/// `c` on `ground`: pulled toward black on a light ground or white on a dark one, in 5% steps, until it stands out
/// at 3:1 (every built-in accent does). The shade, and how far it moved in percent.
pub fn readable(c: Color32, ground: Color32, light: bool) -> (Color32, u8) {
    let to = if light { 0.0 } else { 255.0 };
    for step in 0..=20u8 {
        let k = step as f32 / 20.0;
        let mix = |v: u8| (v as f32 + (to - v as f32) * k).round() as u8;
        let m = rgb(mix(c.r()), mix(c.g()), mix(c.b()));
        if contrast(m, ground) >= 3.0 { return (m, step * 5) }
    }
    (if light { Color32::BLACK } else { Color32::WHITE }, 100)
}

#[test]
fn custom_accents_stay_readable_and_round_trip() {
    assert_eq!(hex("#ff8800"), Some([255, 136, 0]));
    assert_eq!(hex("f80"), Some([255, 136, 0]));
    assert_eq!((hex("#ff880"), hex("#gg8800")), (None, None));
    assert_eq!(Accent::parse(&Accent::Custom([255, 136, 0]).key()), Some(Accent::Custom([255, 136, 0])));
    assert_eq!(Accent::parse("Blue"), Some(Accent::Blue));
    // Orange on black is fine as it is; near-white on paper and near-black on black get pulled until they read.
    assert_eq!(readable(rgb(255, 136, 0), WIRED_DARK.panel, false), (rgb(255, 136, 0), 0));
    for (c, ground, light) in [(rgb(245, 245, 245), SWISS_LIGHT.panel, true), (rgb(16, 24, 32), WIRED_DARK.panel, false)] {
        let (shade, moved) = readable(c, ground, light);
        assert!(moved > 0 && contrast(shade, ground) >= 3.0, "{c:?} -> {shade:?}");
    }
}

/// `base` with its accent swapped for `c`, and the text on it black or white, whichever reads.
fn tinted(base: &Pal, c: Color32) -> Pal {
    let lum = (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0;
    let on = if lum > 0.55 { rgb(0, 0, 0) } else { rgb(250, 248, 242) };
    let swap = |f: Color32| if f == base.accent { c } else if f == base.on_accent { on } else { f };
    Pal { accent: c, on_accent: on, selected: swap(base.selected), on_selected: swap(base.on_selected), ..*base }
}

/// The app icon, `n`×`n` RGBA: a pixel speaker with sound waves, in `ink`, on a near-black rounded square.
/// Drawn here once for the Dock (main.rs) and for the bundle's icon file (`resona icon`, used by bundle.sh).
pub fn app_icon(n: usize, ink: Color32) -> Vec<u8> {
    const ART: [&str; 13] = [
        "......X........", ".....XX........", "....XXX...X....", "XXXXXXX....X...", "XXXXXXX.X...X..",
        "XXXXXXX..X..X..", "XXXXXXX..X..X..", "XXXXXXX..X..X..", "XXXXXXX.X...X..", "XXXXXXX....X...",
        "....XXX...X....", ".....XX........", "......X........",
    ];
    let k = n as f32 / 1024.0; // drawn at 1024, scaled
    let (cell, pad, r) = (44.0 * k, 100.0 * k, 180.0 * k);
    let (ox, oy) = ((n as f32 - cell * 15.0) / 2.0 + cell, (n as f32 - cell * 13.0) / 2.0);
    let panel = [11, 10, 11, 255];
    let mut out = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (cx, cy) = (fx.clamp(pad + r, n as f32 - pad - r), fy.clamp(pad + r, n as f32 - pad - r));
            let inside = (pad..n as f32 - pad).contains(&fx) && (pad..n as f32 - pad).contains(&fy) && (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r;
            let lit = fx >= ox && fy >= oy && ART.get(((fy - oy) / cell) as usize).and_then(|row| row.as_bytes().get(((fx - ox) / cell) as usize)) == Some(&b'X');
            out.extend(match (inside, lit) { (false, _) => [0; 4], (true, true) => [ink.r(), ink.g(), ink.b(), 255], (true, false) => panel });
        }
    }
    out
}

#[test]
fn icon_has_the_speaker_in_the_accent() {
    let ink = rgb(70, 140, 255);
    let px = app_icon(64, ink);
    let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..][..4];
    assert_eq!(at(0, 0), [0, 0, 0, 0]); // outside the rounded square
    assert_eq!(at(32, 4), [0, 0, 0, 0]); // above it
    assert_eq!(at(20, 32), [70, 140, 255, 255]); // the speaker's body
    assert_eq!(at(35, 32), [11, 10, 11, 255]); // between speaker and waves
}

#[test]
fn accents_keep_text_readable() {
    let amber = tinted(&WIRED_DARK, rgb(255, 170, 30));
    assert_eq!((amber.accent, amber.selected, amber.on_accent, amber.on_selected), (rgb(255, 170, 30), rgb(255, 170, 30), rgb(0, 0, 0), rgb(0, 0, 0)));
    let blue = tinted(&SWISS_LIGHT, rgb(22, 82, 204)); // Swiss marks the playing row with accent text on paper
    assert_eq!((blue.selected, blue.on_selected, blue.on_accent), (SWISS_LIGHT.selected, rgb(22, 82, 204), rgb(250, 248, 242)));
    let mono = pal_of(Skin::Wired, egui::Theme::Dark, Accent::Mono); // ink on black, black on the ink
    assert_eq!((mono.accent, mono.on_accent), (WIRED_DARK.text, rgb(0, 0, 0)));
}

const WIRED_DARK: Pal = Pal {
    ground: rgb(0, 0, 0),
    panel: rgb(11, 10, 11),
    surface: rgb(26, 21, 23),
    hover: rgb(26, 21, 23),
    line: rgb(59, 44, 48),
    text: rgb(230, 225, 214),
    dim: rgb(148, 137, 131),
    accent: rgb(224, 40, 46),
    on_accent: rgb(0, 0, 0),
    online: rgb(107, 255, 142),
    selected: rgb(224, 40, 46),
    on_selected: rgb(0, 0, 0),
    track: rgb(26, 21, 23),
};

const WIRED_LIGHT: Pal = Pal {
    ground: rgb(184, 179, 165),
    panel: rgb(226, 222, 210),
    surface: rgb(207, 202, 189),
    hover: rgb(207, 202, 189),
    line: rgb(125, 120, 107),
    text: rgb(17, 16, 16),
    dim: rgb(69, 65, 58),
    accent: rgb(176, 18, 26),
    on_accent: rgb(244, 241, 232),
    online: rgb(30, 122, 58),
    selected: rgb(176, 18, 26),
    on_selected: rgb(244, 241, 232),
    track: rgb(207, 202, 189),
};

const SWISS_LIGHT: Pal = Pal {
    ground: rgb(242, 240, 235),
    panel: rgb(242, 240, 235),
    surface: rgb(230, 227, 220),
    hover: rgb(233, 230, 223),
    line: rgb(212, 209, 202),
    text: rgb(17, 17, 17),
    dim: rgb(107, 105, 100),
    accent: rgb(225, 37, 27),
    on_accent: rgb(255, 255, 255),
    online: rgb(30, 122, 58),
    selected: rgb(242, 240, 235),
    on_selected: rgb(225, 37, 27),
    track: rgb(201, 198, 191),
};

const SWISS_DARK: Pal = Pal {
    ground: rgb(18, 18, 18),
    panel: rgb(18, 18, 18),
    surface: rgb(30, 30, 30),
    hover: rgb(26, 26, 26),
    line: rgb(46, 45, 42),
    text: rgb(242, 240, 235),
    dim: rgb(154, 151, 143),
    accent: rgb(255, 59, 47),
    on_accent: rgb(255, 255, 255),
    online: rgb(59, 209, 111),
    selected: rgb(18, 18, 18),
    on_selected: rgb(255, 59, 47),
    track: rgb(58, 57, 54),
};

/// Paper-coloured type on carbon.
const ASCII_DARK: Pal = Pal {
    ground: rgb(14, 14, 14),
    panel: rgb(20, 20, 20),
    surface: rgb(27, 27, 25),
    hover: rgb(34, 33, 31),
    line: rgb(61, 59, 54),
    text: rgb(220, 215, 202),
    dim: rgb(122, 118, 108),
    accent: rgb(229, 72, 77),
    on_accent: rgb(0, 0, 0),
    online: rgb(107, 200, 120),
    selected: rgb(229, 72, 77),
    on_selected: rgb(0, 0, 0),
    track: rgb(61, 59, 54),
};

/// Tractor-feed printout: black and red ribbon on paper.
const ASCII_LIGHT: Pal = Pal {
    ground: rgb(232, 228, 216),
    panel: rgb(243, 240, 230),
    surface: rgb(228, 236, 223),
    hover: rgb(233, 229, 216),
    line: rgb(185, 179, 163),
    text: rgb(29, 29, 27),
    dim: rgb(125, 120, 108),
    accent: rgb(200, 16, 46),
    on_accent: rgb(255, 255, 255),
    online: rgb(30, 122, 58),
    selected: rgb(200, 16, 46),
    on_selected: rgb(255, 255, 255),
    track: rgb(185, 179, 163),
};

/// Sizes that differ per skin.
pub struct Look {
    pub dark: Pal,
    pub light: Pal,
    /// Space between windows, and between windows and the macOS window edge.
    pub gap: i8,
    pub edge: i8,
    pub row_h: f32,
    pub side_row_h: f32,
    /// Sidebar cover size; 0 = a numbered text list instead (Swiss).
    pub side_cover: f32,
    pub header_cover: f32,
    pub title_size: f32,
    pub player_h: f32,
    /// Padding inside the main window.
    pub pad: i8,
}

pub static WIRED: Look = Look {
    dark: WIRED_DARK,
    light: WIRED_LIGHT,
    gap: 8,
    edge: 14,
    row_h: 44.0,
    side_row_h: 52.0,
    side_cover: 40.0,
    header_cover: 132.0,
    title_size: 64.0,
    player_h: 142.0,
    pad: 24,
};

pub static SWISS: Look = Look {
    dark: SWISS_DARK,
    light: SWISS_LIGHT,
    gap: 0,
    edge: 0,
    row_h: 40.0,
    side_row_h: 28.0,
    side_cover: 0.0,
    header_cover: 120.0,
    title_size: 76.0,
    player_h: 104.0,
    pad: 36,
};

pub static ASCII: Look = Look {
    dark: ASCII_DARK,
    light: ASCII_LIGHT,
    gap: 8,
    edge: 14,
    row_h: 30.0,
    side_row_h: 30.0,
    side_cover: 0.0,
    header_cover: 132.0,
    title_size: 52.0,
    player_h: 128.0,
    pad: 28,
};

pub fn skin(ctx: &egui::Context) -> Skin {
    ctx.data(|d| d.get_temp(egui::Id::new("skin"))).unwrap_or_default()
}

pub fn look(ctx: &egui::Context) -> &'static Look {
    look_of(skin(ctx))
}

fn look_of(skin: Skin) -> &'static Look {
    match skin {
        Skin::Wired => &WIRED,
        Skin::Swiss => &SWISS,
        Skin::Ascii => &ASCII,
    }
}

/// The accent on screen: one being tried in the skin menu (`peek_accent`), else the kept one.
pub fn accent(ctx: &egui::Context) -> Accent {
    ctx.data(|d| d.get_temp(egui::Id::new("accent-peek"))).unwrap_or_else(|| kept_accent(ctx))
}

/// The accent that's saved and on the Dock icon.
pub fn kept_accent(ctx: &egui::Context) -> Accent {
    ctx.data(|d| d.get_temp(egui::Id::new("accent"))).unwrap_or_default()
}

/// Switches the accent colour; takes effect next frame.
pub fn set_accent(ctx: &egui::Context, a: Accent) {
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("accent"), a);
        d.remove_temp::<Accent>(egui::Id::new("accent-peek"));
    });
    visuals(ctx, skin(ctx));
}

/// Shows `a` everywhere without keeping it; None goes back to the kept one.
pub fn peek_accent(ctx: &egui::Context, a: Option<Accent>) {
    let id = egui::Id::new("accent-peek");
    if ctx.data(|d| d.get_temp::<Accent>(id)) == a { return }
    ctx.data_mut(|d| match a {
        Some(a) => d.insert_temp(id, a),
        None => { d.remove_temp::<Accent>(id); }
    });
    visuals(ctx, skin(ctx));
}

pub fn pal(ctx: &egui::Context) -> &'static Pal {
    pal_of(skin(ctx), ctx.theme(), accent(ctx))
}

/// Every skin × theme × accent palette, made once.
pub fn pal_of(skin: Skin, theme: egui::Theme, a: Accent) -> &'static Pal {
    if let Accent::Custom(c) = a { return custom_pal(skin, theme, c) }
    static ALL: std::sync::OnceLock<Vec<Pal>> = std::sync::OnceLock::new();
    let all = ALL.get_or_init(|| {
        let mut v = Vec::new();
        for s in Skin::ALL {
            let l = look_of(s);
            for (light, base) in [(false, &l.dark), (true, &l.light)] {
                for a in Accent::ALL {
                    v.push(match a.shades() { None if a == Accent::Mono => tinted(base, base.text), None => *base, Some((d, li)) => tinted(base, if light { li } else { d }) });
                }
            }
        }
        v
    });
    let s = Skin::ALL.iter().position(|x| *x == skin).unwrap_or(0);
    let t = usize::from(theme == egui::Theme::Light);
    &all[(s * 2 + t) * Accent::ALL.len() + Accent::ALL.iter().position(|x| *x == a).unwrap_or(0)]
}

/// A custom accent's palette for one skin and theme, made the first time it's asked for.
/// ponytail: every colour tried stays in memory (~100 bytes each, a few hundred after dragging the hue bar about);
/// evict old ones if that ever adds up.
fn custom_pal(skin: Skin, theme: egui::Theme, c: [u8; 3]) -> &'static Pal {
    type Made = std::collections::HashMap<(usize, bool, [u8; 3]), &'static Pal>;
    static MADE: std::sync::OnceLock<std::sync::Mutex<Made>> = std::sync::OnceLock::new();
    let light = theme == egui::Theme::Light;
    let key = (Skin::ALL.iter().position(|x| *x == skin).unwrap_or(0), light, c);
    let mut made = MADE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    made.entry(key).or_insert_with(|| {
        let l = look_of(skin);
        let base = if light { &l.light } else { &l.dark };
        Box::leak(Box::new(tinted(base, readable(rgb(c[0], c[1], c[2]), base.panel, light).0)))
    })
}

/// Big titles: VT323 / Archivo ExtraBold / Courier Prime Bold.
pub fn title(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("title".into()))
}

/// Numbers, timecodes, small labels: VT323 / Archivo ExtraBold / Courier Prime Bold.
pub fn num(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("num".into()))
}

/// Switches the whole app to `skin`: palettes, square widgets and fonts. Takes effect next frame.
pub fn apply(ctx: &egui::Context, skin: Skin) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("skin"), skin));
    visuals(ctx, skin);
    ctx.set_fonts(fonts(skin));
}

/// egui's own colours and shapes from the palettes (all an accent change needs: fonts stay).
fn visuals(ctx: &egui::Context, skin: Skin) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let p = pal_of(skin, theme, accent(ctx));
        let mut v = theme.default_visuals();
        v.panel_fill = p.ground;
        v.window_fill = p.panel;
        v.extreme_bg_color = p.surface;
        v.selection.bg_fill = p.accent.gamma_multiply(0.5);
        v.override_text_color = Some(p.text);
        v.selection.stroke = Stroke::new(1.0_f32, p.accent);
        v.text_cursor.stroke.color = p.accent;
        // Both skins are square: widgets and right-click menus too.
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = egui::CornerRadius::ZERO;
        }
        v.window_corner_radius = egui::CornerRadius::ZERO;
        v.menu_corner_radius = egui::CornerRadius::ZERO;
        v.window_stroke = Stroke::new(1.0_f32, p.line);
        ctx.set_visuals_of(theme, v);
    }
    ctx.all_styles_mut(|s| {
        // Scrollbars get their own lane, so they never sit on top of rows.
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.interaction.selectable_labels = false; // so clicks reach the rows
        // Ascii letters its own bar over egui's (`char_scroll`): egui's stays, unseen, to take the drag.
        if skin == Skin::Ascii {
            let sc = &mut s.spacing.scroll;
            // Opacity only applies to floating bars; this one keeps the solid bar's own lane.
            (sc.floating, sc.floating_allocated_width) = (true, sc.allocated_width());
            (sc.dormant_background_opacity, sc.active_background_opacity, sc.interact_background_opacity) = (0.0, 0.0, 0.0);
            (sc.dormant_handle_opacity, sc.active_handle_opacity, sc.interact_handle_opacity) = (0.0, 0.0, 0.0);
        }
    });
}

/// Where a scrollbar's thumb sits in `n` cells: (first cell, length), for `seen` of `all` scrolled `offset` in.
fn thumb(n: usize, seen: f32, all: f32, offset: f32) -> (usize, usize) {
    let len = ((seen / all * n as f32).round() as usize).clamp(1, n);
    (((offset / all * n as f32).round() as usize).min(n - len), len)
}

#[test]
fn thumb_fills_its_share_and_stops_at_the_end() {
    assert_eq!(thumb(10, 500.0, 1000.0, 0.0), (0, 5));
    assert_eq!(thumb(10, 500.0, 1000.0, 500.0), (5, 5)); // scrolled to the end: flush with the bottom
    assert_eq!(thumb(10, 10.0, 100_000.0, 99_990.0), (9, 1)); // a long list still shows one cell, last
}

/// Ascii's scrollbars, lettered over the lane of the scroll area `out` drew: a `:` rail with a `#` thumb down
/// the side, a `-` rail with a `#` thumb along the bottom.
pub fn char_scroll<R>(ui: &egui::Ui, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    if skin(ui.ctx()) != Skin::Ascii { return }
    // `inner_rect` is the room the area was offered, not the size it shrank to: a strip of covers would get
    // its bar at the bottom of the window. Shrunk to the content, it sits right under them.
    let inner = Rect::from_min_size(out.inner_rect.min, out.inner_rect.size().min(out.content_size));
    let (p, lane) = (pal(ui.ctx()), ui.spacing().scroll.allocated_width());
    let font = mono(11.0, true);
    let c = cell(ui.painter(), &font);
    // (cells, seen, all, offset, where the rail starts, the rail's character, between cells)
    let bars = [
        ((inner.height() / c.y) as usize, inner.height(), out.content_size.y, out.state.offset.y, pos2(inner.right() + lane / 2.0, inner.top()), ":", "\n"),
        ((inner.width() / c.x) as usize, inner.width(), out.content_size.x, out.state.offset.x, pos2(inner.left(), inner.bottom() + lane / 2.0 - c.y / 2.0), "-", ""),
    ];
    for (n, seen, all, offset, at, rail, gap) in bars {
        if n == 0 || all <= seen + 0.5 { continue }
        let (top, len) = thumb(n, seen, all, offset);
        let line = |on: bool, ch: &str| (0..n).map(|i| if (top..top + len).contains(&i) == on { ch } else { " " }).collect::<Vec<_>>().join(gap);
        let align = if gap.is_empty() { egui::Align2::LEFT_TOP } else { egui::Align2::CENTER_TOP };
        for (text, color) in [(line(false, rail), p.line), (line(true, "#"), p.dim)] {
            ui.painter().text(at, align, text, font.clone(), color);
        }
    }
}

/// All bundled fonts are OFL (licenses in assets/).
fn fonts(skin: Skin) -> egui::FontDefinitions {
    let mut f = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("dot", &include_bytes!("../assets/DotGothic16-Regular.ttf")[..]),
        ("vt", include_bytes!("../assets/VT323-Regular.ttf")),
        ("archivo", include_bytes!("../assets/Archivo-Regular.ttf")),
        ("archivo-heavy", include_bytes!("../assets/Archivo-ExtraBold.ttf")),
        ("courier", include_bytes!("../assets/CourierPrime-Regular.ttf")),
        ("courier-bold", include_bytes!("../assets/CourierPrime-Bold.ttf")),
    ] {
        f.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    let (body, heading) = match skin {
        Skin::Wired => ("dot", "vt"),
        Skin::Swiss => ("archivo", "archivo-heavy"),
        Skin::Ascii => ("courier", "courier-bold"),
    };
    let defaults = f.families[&FontFamily::Proportional].clone();
    // DotGothic16 covers kana; egui's defaults cover the rest (♥, ♫, arrows).
    let chain = |first: &str| -> Vec<String> { [first, "dot"].map(String::from).into_iter().chain(defaults.iter().cloned()).collect() };
    f.families.insert(FontFamily::Proportional, chain(body));
    f.families.insert(FontFamily::Name("title".into()), chain(heading));
    f.families.insert(FontFamily::Name("num".into()), chain(heading));
    f
}

// ---------- characters (Ascii)

/// Ascii's type: Courier Prime, bold for the loud parts.
fn mono(size: f32, bold: bool) -> FontId {
    if bold { title(size) } else { FontId::proportional(size) }
}

/// One character cell of `font`: its advance and line height.
fn cell(painter: &egui::Painter, font: &FontId) -> Vec2 {
    painter.layout_no_wrap("-".into(), font.clone(), Color32::PLACEHOLDER).size()
}

/// A box of characters around `rect`: `+--[ title ]--+` on top, `|` down the sides, `+---+` under. The title
/// sits on `fill` in `ink`.
fn char_box(painter: &egui::Painter, rect: Rect, title: Option<&str>, color: Color32, ink: Color32, fill: Color32) {
    let font = mono(14.0, false);
    let c = cell(painter, &font);
    let n = ((rect.width() / c.x) as usize).saturating_sub(1);
    let rule = format!("+{}", "-".repeat(n.saturating_sub(1)));
    for y in [rect.top(), rect.bottom() - c.y] {
        painter.text(pos2(rect.left(), y), egui::Align2::LEFT_TOP, &rule, font.clone(), color);
        painter.text(pos2(rect.right(), y), egui::Align2::RIGHT_TOP, "+", font.clone(), color); // the exact edge
    }
    let mut y = rect.top() + c.y;
    while y < rect.bottom() - c.y - 1.0 {
        painter.text(pos2(rect.left(), y), egui::Align2::LEFT_TOP, "|", font.clone(), color);
        painter.text(pos2(rect.right(), y), egui::Align2::RIGHT_TOP, "|", font.clone(), color);
        y += c.y;
    }
    if let Some(t) = title {
        let g = painter.layout_no_wrap(format!("[ {t} ]"), font, ink);
        let at = pos2(rect.left() + c.x * 2.0, rect.top());
        painter.rect_filled(Rect::from_min_size(at, g.size()), 0.0, fill);
        painter.galley(at, g, ink);
    }
}

/// `[####------]` across `width`: `frac` of it filled with `full`, the rest `-`.
fn char_bar(painter: &egui::Painter, rect: Rect, frac: f32, full: char, lit: Color32, rest: Color32) {
    let font = mono(15.0, true);
    let n = ((rect.width() / cell(painter, &font).x) as usize).saturating_sub(2).max(1);
    let k = ((frac.clamp(0.0, 1.0) * n as f32).round() as usize).min(n);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
    job.append("[", 0.0, fmt(rest));
    job.append(&full.to_string().repeat(k), 0.0, fmt(lit));
    job.append(&"-".repeat(n - k), 0.0, fmt(rest));
    job.append("]", 0.0, fmt(rest));
    let g = painter.layout_job(job);
    painter.galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, rest);
}

/// The character ramp covers are dithered to, light to dark.
const RAMP: &[u8] = b" .:-=+*#%@";

/// Ascii's cover art: two soft blobs over a gradient in ramp characters, `cols` × `rows`, seeded by `seed` so each
/// list keeps its own.
fn char_art(seed: &str, cols: usize, rows: usize) -> String {
    let mut h = seed.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) | 1;
    let mut rnd = || { h ^= h << 13; h ^= h >> 17; h ^= h << 5; (h % 1000) as f32 / 1000.0 };
    let blobs: Vec<(f32, f32, f32)> = (0..2).map(|_| (rnd(), rnd(), 0.25 + rnd() * 0.3)).collect();
    let line = |j: usize| -> String {
        (0..cols).map(|i| {
            let (u, v) = (i as f32 / cols as f32, j as f32 / rows as f32);
            let mut val = 0.15 + 0.35 * v;
            for (x, y, r) in &blobs { val += (1.0 - ((u - x).powi(2) + ((v - y) * 0.9).powi(2)).sqrt() / r).max(0.0) * 0.6 }
            val += (((i * 7 + j * 13) % 5) as f32 - 2.0) * 0.03; // ordered noise, so flat areas still dither
            RAMP[((val * 9.99).max(0.0) as usize).min(9)] as char
        }).collect()
    };
    (0..rows).map(line).collect::<Vec<_>>().join("\n")
}

#[test]
fn char_art_is_the_lists_own_and_only_ramp_characters() {
    let (a, b) = (char_art("late night drive", 22, 12), char_art("gym 2026", 22, 12));
    assert_eq!(a, char_art("late night drive", 22, 12)); // a list keeps its cover
    assert_ne!(a, b);
    assert_eq!(a.lines().count(), 12);
    assert!(a.lines().all(|l| l.len() == 22) && a.bytes().all(|c| c == b'\n' || RAMP.contains(&c)));
}

/// How sharp a brightness step must be (Sobel, on 0..1) to be drawn as an outline instead of a shade.
const EDGE: f32 = 1.6;

/// A real cover in characters, `cols` × `rows`, the way image-to-ASCII converters do it: each cell's brightness,
/// stretched to the picture's own range and S-curved so mid-tones split, picks a ramp character, and a sharp step
/// is drawn as an outline (`- | / \`) along its direction. `dark`: light ink on black, so bright is dense.
fn image_art(img: &image::GrayImage, cols: usize, rows: usize, dark: bool) -> String {
    let img = image::imageops::resize(img, cols.max(1) as u32, rows.max(1) as u32, image::imageops::FilterType::Triangle);
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (lo, hi) = img.pixels().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
    let span = hi.saturating_sub(lo).max(1) as f32;
    let at = |x: i32, y: i32| (img.get_pixel(x.clamp(0, w - 1) as u32, y.clamp(0, h - 1) as u32)[0] - lo) as f32 / span;
    let grad = |x: i32, y: i32| {
        let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
        // ponytail: a cell taken as twice as tall as wide; pass the font's real cell if diagonals come out skewed
        let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)) * 0.5;
        (gx, gy)
    };
    let mag = |x: i32, y: i32| { let (gx, gy) = grad(x, y); gx.hypot(gy) };
    let ch = |x: i32, y: i32| {
        let (gx, gy) = grad(x, y);
        let m = gx.hypot(gy);
        // A step lights the cells on both sides of it: only the stronger one, across the edge, draws the line.
        let (dx, dy) = if gx.abs() >= gy.abs() { (1, 0) } else { (0, 1) };
        if m > EDGE && m >= mag(x - dx, y - dy) && m > mag(x + dx, y + dy) {
            // The edge runs across the gradient; y points down, so 45° is "\\".
            return match (gy.atan2(gx).to_degrees() + 90.0).rem_euclid(180.0) {
                e if !(22.5..157.5).contains(&e) => '-',
                e if e < 67.5 => '\\',
                e if e < 112.5 => '|',
                _ => '/',
            };
        }
        let t = at(x, y);
        let t = t * t * (3.0 - 2.0 * t);
        RAMP[((if dark { t } else { 1.0 - t }) * 9.99) as usize] as char
    };
    (0..h).map(|y| (0..w).map(|x| ch(x, y)).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// A downloaded cover, decoded once and kept small and grey: all `image_art` needs at any size.
fn cover_luma(bytes: &[u8]) -> Option<std::sync::Arc<image::GrayImage>> {
    Some(std::sync::Arc::new(image::load_from_memory(bytes).ok()?.thumbnail(192, 192).to_luma8()))
}

#[test]
fn image_art_shades_flat_areas_and_outlines_steps() {
    let mut img = image::GrayImage::new(4, 2);
    for x in 2..4 { for y in 0..2 { img.put_pixel(x, y, image::Luma([255])) } } // black left half, white right
    assert_eq!(image_art(&img, 4, 2, true), "  |@\n  |@"); // one line per step, not two
    assert_eq!(image_art(&img, 4, 2, false), "@@| \n@@| ");
    assert!(cover_luma(b"not an image").is_none());
}

#[test]
#[ignore] // RESONA_COVER=some.jpg cargo test cover_preview -- --ignored --nocapture
fn cover_preview() {
    let bytes = std::fs::read(std::env::var("RESONA_COVER").unwrap()).unwrap();
    let img = cover_luma(&bytes).unwrap();
    for (c, r) in [(36, 18), (10, 5)] { println!("{}\n", image_art(&img, c, r, false)) }
}

/// Ascii's cover, filling `rect`: the art at `url` in characters once it's downloaded, the list's own `char_art`
/// until then (and for lists without a picture).
fn char_cover(painter: &egui::Painter, rect: Rect, url: Option<&str>, seed: &str, color: Color32) {
    let font = mono((rect.width() / 36.0).clamp(7.0, 14.0), true); // ~36 columns on big covers; under 7pt small ones blur to grey
    let c = cell(painter, &font);
    let (cols, rows) = ((rect.width() / c.x) as usize, (rect.height() / c.y) as usize);
    let ctx = painter.ctx();
    let dark = ctx.theme() == egui::Theme::Dark;
    // Each cover is decoded once (a broken one is kept as None, like egui's own loader keeps the error), and its
    // characters are kept for the last size and theme drawn: one entry per cover, redone only when those change.
    type Art = (usize, usize, bool, std::sync::Arc<String>);
    let real = url.and_then(|u| {
        let (luma_id, art_id) = (egui::Id::new(("cover-luma", u)), egui::Id::new(("char_cover", u)));
        if let Some((c, r, d, art)) = ctx.data(|m| m.get_temp::<Art>(art_id)) && (c, r, d) == (cols, rows, dark) { return Some(art) }
        let luma = match ctx.data(|m| m.get_temp::<Option<std::sync::Arc<image::GrayImage>>>(luma_id)) {
            Some(luma) => luma,
            None => match ctx.try_load_bytes(u) {
                Ok(egui::load::BytesPoll::Pending { .. }) => return None,
                done => {
                    let luma = done.ok().and_then(|b| match b { egui::load::BytesPoll::Ready { bytes, .. } => cover_luma(&bytes), _ => None });
                    ctx.data_mut(|m| m.insert_temp(luma_id, luma.clone()));
                    luma
                }
            },
        }?;
        let art = std::sync::Arc::new(image_art(&luma, cols, rows, dark));
        ctx.data_mut(|m| m.insert_temp::<Art>(art_id, (cols, rows, dark, art.clone())));
        Some(art)
    });
    let art = real.map_or_else(|| char_art(seed, cols, rows), |a| a.to_string());
    painter.with_clip_rect(rect).text(rect.min, egui::Align2::LEFT_TOP, art, font, color);
}

/// ASCII's text in 7 bits: the few typographic marks the other skins use, spelled out. Other skins get `text` back.
pub fn plain(s: Skin, text: &str) -> String {
    if s != Skin::Ascii { return text.into() }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '·' => out.push('-'),
            '…' => out.push_str("..."),
            '√' => out.push('x'),
            '♪' => out.push('~'),
            '⌘' => out.push_str("CMD"),
            '⇧' => out.push_str("SHIFT"),
            '→' => out.push_str("->"),
            '←' => out.push_str("<-"),
            '↑' => out.push_str("UP"),
            '↓' => out.push_str("DOWN"),
            '▶' => out.push('>'),
            '◀' => out.push('<'),
            c => out.push(c),
        }
    }
    out
}

#[test]
fn plain_spells_marks_out_for_ascii_only() {
    assert_eq!(plain(Skin::Ascii, "26 TRACKS · 51 MIN"), "26 TRACKS - 51 MIN");
    assert_eq!(plain(Skin::Ascii, "Next (⌘→) · Loading…"), "Next (CMD->) - Loading...");
    assert!(plain(Skin::Ascii, "⌘ ⇧ C ↑ ↓ ← → √ ♪ ▶ ◀ · …").is_ascii());
    assert_eq!(plain(Skin::Wired, "a · b…"), "a · b…");
}

/// ASCII cuts `text` to whole characters ending in `...` when it's wider than `width` (egui's own cut mark is
/// the 8-bit `…`); the font is monospaced, so characters are cells. Other skins get `text` back.
pub fn fit(ui: &egui::Ui, text: String, font: &FontId, width: f32) -> String {
    if skin(ui.ctx()) != Skin::Ascii || ui.painter().layout_no_wrap(text.clone(), font.clone(), Color32::PLACEHOLDER).size().x <= width {
        return text;
    }
    let n = ((width / cell(ui.painter(), font).x) as usize).saturating_sub(3);
    format!("{}...", text.chars().take(n).collect::<String>().trim_end())
}

/// The teletype spinner, `| / - \`, turning 8 times a second.
pub fn spinner(t: f64) -> char {
    ['|', '/', '-', '\\'][(t * 8.0) as usize % 4]
}

/// ASCII's loading title, typed out a character every 1/30 s from when this title began loading; all of it otherwise.
pub fn typed(ctx: &egui::Context, text: &str, typing: bool) -> String {
    let id = egui::Id::new("typed");
    if !typing {
        ctx.data_mut(|d| d.remove::<(String, f64)>(id));
        return text.into();
    }
    let now = ctx.input(|i| i.time);
    // Kept with its title, so opening another list mid-load types that one from the start.
    let start = ctx.data_mut(|d| {
        let at = d.get_temp_mut_or_insert_with(id, || (text.to_string(), now));
        if at.0 != text { *at = (text.to_string(), now) }
        at.1
    });
    text.chars().take(((now - start) * 30.0) as usize).collect()
}

// ---------- vector glyphs (Swiss)

/// A smooth heart, `r` wide each side of `c`.
pub fn heart(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    for dx in [-0.5, 0.5] {
        painter.circle_filled(c + vec2(dx * r, -0.2 * r), 0.52 * r, color);
    }
    painter.add(Shape::convex_polygon(vec![c + vec2(-0.98 * r, -0.05 * r), c + vec2(0.98 * r, -0.05 * r), c + vec2(0.0, 0.95 * r)], color, Stroke::NONE));
}

/// Filled triangle pointing right (`dir` = 1) or left (-1).
fn tri(painter: &egui::Painter, c: Pos2, h: f32, dir: f32, color: Color32) {
    let w = h * 0.86;
    painter.add(Shape::convex_polygon(vec![c + vec2(-w / 2.0 * dir, -h / 2.0), c + vec2(w / 2.0 * dir, 0.0), c + vec2(-w / 2.0 * dir, h / 2.0)], color, Stroke::NONE));
}

// ---------- pixel icons (Wired)

/// Width, height, and [x, y, w, h] cells.
pub type Icon = (f32, f32, &'static [[f32; 4]]);
const PREV: Icon = (8.0, 7.0, &[[0.0, 0.0, 1.0, 7.0], [2.0, 3.0, 1.0, 1.0], [3.0, 2.0, 1.0, 3.0], [4.0, 1.0, 1.0, 5.0], [5.0, 0.0, 3.0, 7.0]]);
const NEXT: Icon = (8.0, 7.0, &[[0.0, 0.0, 3.0, 7.0], [3.0, 1.0, 1.0, 5.0], [4.0, 2.0, 1.0, 3.0], [5.0, 3.0, 1.0, 1.0], [7.0, 0.0, 1.0, 7.0]]);
const PAUSE: Icon = (7.0, 8.0, &[[0.0, 0.0, 2.0, 8.0], [5.0, 0.0, 2.0, 8.0]]);
const PLAY: Icon = (7.0, 8.0, &[[0.0, 0.0, 2.0, 8.0], [2.0, 1.0, 2.0, 6.0], [4.0, 2.0, 2.0, 4.0], [6.0, 3.0, 1.0, 2.0]]);
const SHUFFLE: Icon = (9.0, 7.0, &[[0.0, 1.0, 2.0, 1.0], [2.0, 2.0, 1.0, 1.0], [3.0, 3.0, 2.0, 1.0], [5.0, 4.0, 1.0, 1.0], [6.0, 5.0, 2.0, 1.0], [0.0, 5.0, 2.0, 1.0], [2.0, 4.0, 1.0, 1.0], [5.0, 2.0, 1.0, 1.0], [6.0, 1.0, 2.0, 1.0], [7.0, 0.0, 1.0, 3.0], [8.0, 1.0, 1.0, 1.0], [7.0, 4.0, 1.0, 3.0], [8.0, 5.0, 1.0, 1.0]]);
const REPEAT: Icon = (9.0, 7.0, &[[1.0, 1.0, 6.0, 1.0], [6.0, 0.0, 1.0, 3.0], [7.0, 1.0, 1.0, 1.0], [0.0, 1.0, 1.0, 3.0], [8.0, 3.0, 1.0, 3.0], [2.0, 5.0, 6.0, 1.0], [2.0, 4.0, 1.0, 3.0], [1.0, 5.0, 1.0, 1.0]]);
const QUEUE: Icon = (9.0, 7.0, &[[0.0, 0.0, 1.0, 3.0], [1.0, 1.0, 1.0, 1.0], [3.0, 1.0, 6.0, 1.0], [0.0, 4.0, 9.0, 1.0], [0.0, 6.0, 9.0, 1.0]]);
const LOUPE: Icon = (9.0, 9.0, &[[2.0, 0.0, 3.0, 1.0], [1.0, 1.0, 1.0, 1.0], [5.0, 1.0, 1.0, 1.0], [0.0, 2.0, 1.0, 3.0], [6.0, 2.0, 1.0, 3.0], [1.0, 5.0, 1.0, 1.0], [5.0, 5.0, 1.0, 1.0], [2.0, 6.0, 3.0, 1.0], [6.0, 6.0, 1.0, 1.0], [7.0, 7.0, 1.0, 1.0], [8.0, 8.0, 1.0, 1.0], [6.0, 7.0, 1.0, 1.0], [7.0, 8.0, 1.0, 1.0]]);
const NOTE: Icon = (7.0, 8.0, &[[2.0, 0.0, 1.0, 6.0], [3.0, 0.0, 2.0, 1.0], [5.0, 1.0, 1.0, 2.0], [0.0, 5.0, 3.0, 3.0]]);
const MENU: Icon = (7.0, 7.0, &[[0.0, 0.0, 7.0, 1.0], [0.0, 3.0, 7.0, 1.0], [0.0, 6.0, 7.0, 1.0]]);
const SPARKLE: Icon = (9.0, 9.0, &[[4.0, 0.0, 1.0, 2.0], [3.0, 2.0, 3.0, 1.0], [2.0, 3.0, 5.0, 1.0], [0.0, 4.0, 9.0, 1.0], [2.0, 5.0, 5.0, 1.0], [3.0, 6.0, 3.0, 1.0], [4.0, 7.0, 1.0, 2.0]]);
const HEART: Icon = (7.0, 6.0, &[[1.0, 0.0, 2.0, 1.0], [4.0, 0.0, 2.0, 1.0], [0.0, 1.0, 7.0, 2.0], [1.0, 3.0, 5.0, 1.0], [2.0, 4.0, 3.0, 1.0], [3.0, 5.0, 1.0, 1.0]]);

/// Draws `icon` centred on `center`, `scale` px per cell.
fn pixels(painter: &egui::Painter, center: Pos2, (w, h, cells): Icon, scale: f32, color: Color32) {
    let origin = center - vec2(w, h) * scale / 2.0;
    for [x, y, cw, ch] in cells {
        painter.rect_filled(Rect::from_min_size(origin + vec2(*x, *y) * scale, vec2(*cw, *ch) * scale), 0.0, color);
    }
}

/// Checkerboard fill, Wired's stand-in for missing cover art.
fn dither(painter: &egui::Painter, rect: Rect, tone: Color32) {
    // ponytail: one quad per lit 2px cell (~4k at 132px); bake a texture if covers ever get much bigger.
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, tone), StrokeKind::Inside);
    let (cols, rows) = ((rect.width() / 2.0) as usize, (rect.height() / 2.0) as usize);
    for y in 0..rows {
        for x in (y % 2..cols).step_by(2) {
            painter.rect_filled(Rect::from_min_size(rect.min + vec2(x as f32, y as f32) * 2.0, Vec2::splat(2.0)), 0.0, tone);
        }
    }
}

// ---------- toast

/// The toast in the bottom right, above the player bar. Wired: a bordered box with an OK / ERR tag.
/// Swiss: a flat surface under one rule, black when done, red for an error. `left` (1 → 0) drains the bar under it.
pub fn toast(ctx: &egui::Context, text: &str, error: bool, left: f32) -> Response {
    let (p, s, l) = (pal(ctx), skin(ctx), look(ctx));
    if s == Skin::Ascii {
        // A printed line, no box: `*** note ***` in ink, `>> ERR: …` in the accent.
        let text = plain(s, text);
        let line = if error { format!(">> ERR: {text}") } else { format!("*** {text} ***") };
        return egui::Area::new(egui::Id::new("toast")).order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-(l.edge as f32 + 14.0), -(l.player_h + 14.0)))
            .show(ctx, |ui| {
                let rect = egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.set_max_width(520.0);
                    let line = fit(ui, line, &mono(15.0, true), 520.0);
                    ui.add(egui::Label::new(RichText::new(line).font(mono(15.0, true)).color(if error { p.accent } else { p.text })).truncate());
                }).response.rect;
                ui.interact(rect, egui::Id::new("toast-click"), Sense::click())
            })
            .inner;
    }
    let flag = if error { p.accent } else { p.online };
    egui::Area::new(egui::Id::new("toast"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-(l.edge as f32 + 14.0), -(l.player_h + 14.0)))
        .show(ctx, |ui| {
            let frame = match s {
                Skin::Wired => egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0_f32, if error { p.accent } else { p.line })),
                Skin::Swiss | Skin::Ascii => egui::Frame::new().fill(p.surface).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 12 }),
            };
            let rect = frame.show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.horizontal(|ui| match s {
                    Skin::Wired => {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (tag, _) = ui.allocate_exact_size(vec2(38.0, 28.0), Sense::hover());
                        ui.painter().rect_filled(tag, 0.0, flag);
                        ui.painter().text(tag.center(), egui::Align2::CENTER_CENTER, if error { "ERR" } else { "OK" }, num(17.0), p.panel);
                        ui.add(egui::Label::new(RichText::new(text.to_uppercase()).font(num(19.0)).color(p.text)).truncate());
                        ui.add_space(2.0);
                    }
                    Skin::Swiss | Skin::Ascii => { ui.add(egui::Label::new(RichText::new(text).size(14.0).color(p.text)).truncate()); }
                });
            }).response.rect;
            let painter = ui.painter();
            if s == Skin::Swiss {
                painter.hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0_f32, if error { p.accent } else { p.text }));
            }
            let bar = if s == Skin::Wired { flag } else { p.dim };
            painter.rect_filled(Rect::from_min_size(pos2(rect.left(), rect.bottom() - 2.0), vec2(rect.width() * left.clamp(0.0, 1.0), 2.0)), 0.0, bar);
            ui.interact(rect, egui::Id::new("toast-click"), Sense::click())
        })
        .inner
}

// ---------- windows

#[derive(Clone, Copy, PartialEq)]
pub enum Win {
    Library,
    Main,
    Player,
}

/// One of the three windows. Wired: square frame plus a title bar ("■ title", a tag right, an optional
/// status dot). Swiss: a bare area, with a black rule over the player.
pub fn window<R>(ui: &mut egui::Ui, win: Win, title_text: &str, tag: &str, dot: Option<Color32>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let active = win == Win::Main;
    if s == Skin::Ascii {
        // Drawn around the contents afterwards, so the margin leaves a character's room on every side.
        let r = egui::Frame::new().fill(p.panel).inner_margin(egui::Margin { left: 18, right: 18, top: 24, bottom: 20 }).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            add(ui)
        });
        char_box(ui.painter(), r.response.rect, Some(title_text), p.line, if active { p.accent } else { p.dim }, p.panel);
        return r.inner;
    }
    let stroke = match s {
        Skin::Wired => Stroke::new(1.0_f32, if active { p.accent } else { p.line }),
        Skin::Swiss | Skin::Ascii => Stroke::NONE,
    };
    egui::Frame::new().fill(p.panel).stroke(stroke).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        let rect = ui.max_rect();
        match s {
            Skin::Swiss if win == Win::Player => { ui.painter().hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0_f32, p.text)); }
            Skin::Swiss | Skin::Ascii => {}
            Skin::Wired => {
                ui.spacing_mut().item_spacing.y = 0.0;
                let (bar, ink) = if active { (p.accent, p.on_accent) } else { (p.surface, p.dim) };
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(rect, 0.0, bar);
                painter.text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("■ {title_text}"), num(17.0), ink);
                let tag = painter.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, tag, FontId::proportional(12.0), ink);
                if let Some(c) = dot {
                    painter.rect_filled(Rect::from_center_size(pos2(tag.left() - 8.0, rect.center().y), Vec2::splat(6.0)), 0.0, c);
                }
                ui.spacing_mut().item_spacing.y = 8.0;
            }
        }
        add(ui)
    })
    .inner
}

/// Small section label (Swiss). Wired has title bars instead.
pub fn section(ui: &mut egui::Ui, text: &str) {
    if skin(ui.ctx()) == Skin::Swiss {
        ui.label(RichText::new(text.to_uppercase()).font(num(13.0)).color(pal(ui.ctx()).text));
    }
}

// ---------- search

/// A text field (search, new playlist) with each skin's hint. Returns the text edit's response (Enter = `lost_focus` + Enter pressed).
pub fn text_field(ui: &mut egui::Ui, id: &str, wired: &str, swiss: &str, query: &mut String) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let id = egui::Id::new(id);
    let focused = ui.memory(|m| m.has_focus(id));
    let (margin, hint, font, fill, stroke) = match s {
        Skin::Wired => (egui::Margin::symmetric(8, 4), wired, num(20.0), p.surface, Stroke::new(1.0_f32, if focused { p.accent } else { p.line })),
        Skin::Swiss => (egui::Margin::symmetric(0, 6), swiss, FontId::proportional(14.0), Color32::TRANSPARENT, Stroke::NONE),
        Skin::Ascii => (egui::Margin::symmetric(0, 6), wired, mono(15.0, false), Color32::TRANSPARENT, Stroke::NONE),
    };
    let r = egui::Frame::new().fill(fill).stroke(stroke).inner_margin(margin).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if s.blocky() {
                // A fixed ">" prompt, red while typing; typing starts right after it.
                ui.label(RichText::new(">").font(if s == Skin::Ascii { mono(15.0, true) } else { num(20.0) }).color(if focused { p.accent } else { p.dim }));
            }
            let hint = RichText::new(hint).font(font.clone()).color(p.dim);
            ui.add(egui::TextEdit::singleline(query).id(id).frame(false).hint_text(hint).font(font).margin(Vec2::ZERO).desired_width(f32::INFINITY))
        })
        .inner
    });
    let rect = r.response.rect;
    match s {
        // Print-style field: just a rule under the text.
        Skin::Swiss => { ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(2.0_f32, if focused { p.accent } else { p.text })); }
        Skin::Ascii => {
            let font = mono(14.0, false);
            let n = (rect.width() / cell(ui.painter(), &font).x) as usize;
            ui.painter().text(rect.left_bottom(), egui::Align2::LEFT_CENTER, "-".repeat(n), font, if focused { p.accent } else { p.line });
        }
        Skin::Wired => {}
    }
    r.inner
}

/// A dim label `indent` in, with a rule running on to `width`: the divider between groups of rows (New
/// releases' days). ASCII's rule is dashes.
pub fn divider(ui: &mut egui::Ui, text: &str, font: FontId, indent: f32, width: f32) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let left = ui.cursor().left();
    let r = ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new(text).font(font).color(p.dim))
    }).inner;
    let (from, to, y) = (r.rect.right() + 10.0, left + width, r.rect.center().y);
    if from >= to { return }
    if s == Skin::Ascii {
        let font = mono(14.0, false);
        let n = ((to - from) / cell(ui.painter(), &font).x) as usize;
        ui.painter().text(pos2(from, y), egui::Align2::LEFT_CENTER, "-".repeat(n), font, p.line);
    } else {
        ui.painter().hline(from..=to, y, Stroke::new(1.0_f32, p.line));
    }
}

// ---------- covers

/// What to stamp on a cover that has no art.
pub enum Stamp<'a> {
    Liked,
    /// The play queue: a "play + list" glyph.
    Queue,
    /// Search results: a magnifying glass.
    Search,
    /// New releases: a four-point sparkle.
    New,
    Text(&'a str),
    None,
}

/// Album art, or this skin's stand-in. `big` = the playlist header (Wired adds crosshairs).
pub fn cover(ui: &mut egui::Ui, url: Option<&str>, size: f32, stamp: Stamp, big: bool) -> Rect {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if s == Skin::Ascii {
        // The cover in characters; a list without a picture gets its own art with its tag lettered in the corner.
        let tag = match stamp { Stamp::Liked => "<3", Stamp::Queue => "Q", Stamp::Search => "?", Stamp::New => "NR", Stamp::Text(t) => t, Stamp::None => "" };
        let painter = ui.painter();
        char_cover(painter, rect, url, url.unwrap_or(tag), if big { p.text } else { p.dim });
        if url.is_none() && !tag.is_empty() {
            let g = painter.layout_no_wrap(format!("[{tag}]"), mono((size * 0.16).max(13.0), true), p.accent);
            let at = Rect::from_min_size(rect.left_bottom() - vec2(0.0, g.size().y), g.size() + vec2(4.0, 0.0));
            painter.rect_filled(at, 0.0, p.panel);
            painter.galley(at.min + vec2(2.0, 0.0), g, p.accent);
        }
        return rect;
    }
    if let Some(u) = url {
        let img = egui::Image::new(u);
        // A broken link or odd format falls through to the stand-in instead of egui's ⚠.
        if img.load_for_size(ui.ctx(), rect.size()).is_ok() {
            img.paint_at(ui, rect);
            return rect;
        }
    }
    let painter = ui.painter();
    match (s, stamp) {
        // The queue gets its own tile, like Liked Songs, instead of a stand-in cover.
        (Skin::Wired, Stamp::Queue) => {
            painter.rect(rect, 0.0, p.surface, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
            queue_glyph(painter, rect.center(), size * 0.5, p.accent, true);
        }
        (Skin::Swiss, Stamp::Queue) => {
            painter.rect_filled(rect, 0.0, p.text);
            queue_glyph(painter, rect.center(), size * 0.5, p.panel, false);
        }
        (Skin::Wired, Stamp::Search) => {
            painter.rect(rect, 0.0, p.surface, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
            pixels(painter, rect.center(), LOUPE, (size * 0.5 / 9.0).floor().max(2.0), p.accent);
        }
        (Skin::Swiss, Stamp::Search) => {
            painter.rect_filled(rect, 0.0, p.text);
            let (c, r) = (rect.center() - Vec2::splat(size * 0.06), size * 0.17);
            let st = Stroke::new((size / 22.0).max(2.0), p.panel);
            painter.circle_stroke(c, r, st);
            painter.line_segment([c + Vec2::splat(r * 0.72), c + Vec2::splat(r * 1.75)], st);
        }
        (Skin::Wired, Stamp::New) => {
            painter.rect(rect, 0.0, p.surface, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
            pixels(painter, rect.center(), SPARKLE, (size * 0.55 / 9.0).floor().max(2.0), p.accent);
        }
        (Skin::Swiss, Stamp::New) => {
            // Two thin diamonds crossed: a four-point star.
            painter.rect_filled(rect, 0.0, p.text);
            let (c, r, w) = (rect.center(), size * 0.3, size * 0.07);
            for (a, b) in [(vec2(0.0, r), vec2(w, 0.0)), (vec2(r, 0.0), vec2(0.0, w))] {
                painter.add(egui::Shape::convex_polygon(vec![c - a, c + b, c + a, c - b], p.panel, Stroke::NONE));
            }
        }
        (Skin::Wired, Stamp::Liked) => {
            painter.rect_filled(rect, 0.0, p.accent);
            pixels(painter, rect.center(), HEART, (size / 11.0).floor().max(3.0), p.on_accent);
        }
        (Skin::Swiss, Stamp::Liked) => {
            painter.rect_filled(rect, 0.0, p.accent);
            heart(painter, rect.center(), size * 0.24, p.on_accent);
        }
        (Skin::Wired, Stamp::Text(t)) => {
            dither(painter, rect, if big { p.accent } else { p.dim });
            if big {
                painter.vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, p.text));
                painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, p.text));
            }
            let g = painter.layout_no_wrap(t.into(), num((size * 0.36).max(16.0)), p.text);
            let stamp = Rect::from_min_size(rect.left_bottom() - vec2(0.0, g.size().y), g.size() + vec2(6.0, 0.0));
            painter.rect_filled(stamp, 0.0, p.panel);
            painter.galley(stamp.min + vec2(3.0, 0.0), g, p.text);
        }
        (Skin::Swiss, Stamp::Text(t)) => {
            painter.rect_filled(rect, 0.0, p.surface);
            painter.text(rect.left_bottom() + vec2(size * 0.08, -size * 0.06), egui::Align2::LEFT_BOTTOM, t, title(size * 0.3), p.text);
        }
        (Skin::Wired, Stamp::None) => dither(painter, rect, p.accent),
        (Skin::Swiss, Stamp::None) => { painter.rect_filled(rect, 0.0, p.surface); }
        (Skin::Ascii, _) => {} // drawn above
    }
    rect
}

/// "Play + list": a small arrow and three lines, `w` wide. Wired gets the pixel version.
fn queue_glyph(painter: &egui::Painter, c: Pos2, w: f32, color: Color32, pixel: bool) {
    if pixel {
        pixels(painter, c, QUEUE, (w / 9.0).floor().max(2.0), color);
        return;
    }
    let (x0, x1, h) = (c.x - w / 2.0, c.x + w / 2.0, w * 0.75);
    let st = Stroke::new((w / 10.0).max(1.5), color);
    tri(painter, pos2(x0 + w * 0.08, c.y - h * 0.32), h * 0.36, 1.0, color);
    painter.line_segment([pos2(x0 + w * 0.32, c.y - h * 0.32), pos2(x1, c.y - h * 0.32)], st);
    painter.line_segment([pos2(x0, c.y + h * 0.05), pos2(x1, c.y + h * 0.05)], st);
    painter.line_segment([pos2(x0, c.y + h * 0.4), pos2(x1, c.y + h * 0.4)], st);
}

// ---------- rows

/// Whether the playing row and the play buttons are accent blocks: Wired's always are; ASCII prints its marks in
/// the accent instead, except under the Mono accent, which leaves no colour to print in.
fn fills(p: &Pal, s: Skin) -> bool {
    s == Skin::Wired || s == Skin::Ascii && p.accent == p.text
}

/// Background of a track row, and the colours for its text.
pub fn row_bg(ui: &egui::Ui, rect: Rect, current: bool, hovered: bool) -> (Color32, Color32) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let painter = ui.painter();
    match s {
        Skin::Wired => {
            if current { painter.rect_filled(rect, 0.0, p.selected); }
            if hovered {
                // On the playing row's fill the frame is drawn inset, in the fill's ink.
                let (frame, ink) = if current { (rect.shrink(2.0), p.on_selected) } else { (rect, p.accent) };
                painter.rect_stroke(frame, 0.0, Stroke::new(1.0_f32, ink), StrokeKind::Inside);
            }
            painter.extend(Shape::dashed_line(&[rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, p.line), 1.0, 2.0));
        }
        Skin::Swiss => {
            if hovered { painter.rect_filled(rect, 0.0, p.hover); }
            painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, p.line));
        }
        Skin::Ascii => {
            // A teletype can't print a block: the playing row is printed in the accent instead (`fills`).
            let block = current && fills(p, s);
            if block { painter.rect_filled(rect, 0.0, p.selected); }
            if hovered {
                let (font, ink) = (mono(16.0, true), if block { p.on_selected } else { p.accent });
                painter.text(rect.left_center() + vec2(2.0, 0.0), egui::Align2::LEFT_CENTER, "[", font.clone(), ink);
                painter.text(rect.right_center() - vec2(2.0, 0.0), egui::Align2::RIGHT_CENTER, "]", font, ink);
            }
        }
    }
    match (s, current) {
        (Skin::Swiss, true) => (p.on_selected, p.on_selected),
        (Skin::Ascii, true) if !fills(p, s) => (p.accent, p.accent),
        (Skin::Wired | Skin::Ascii, true) => (p.on_selected, p.on_selected.gamma_multiply(0.85)),
        _ => (p.text, p.dim),
    }
}

/// A like heart in `rect`: lit when `on`, dim when off but `show_off` (a hovered row, the player bar).
/// `current` = it sits on the playing row's fill.
pub fn heart_toggle(ui: &mut egui::Ui, rect: Rect, id: egui::Id, on: bool, show_off: bool, current: bool) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let r = ui.interact(rect, id, Sense::click());
    let current = current && fills(p, s); // on the accent-filled row the heart is drawn in the row's ink
    let lit = if current { p.on_selected } else { p.accent };
    let off = if current { p.on_selected.gamma_multiply(0.45) } else { p.dim.gamma_multiply(0.6) };
    let color = if on { Some(lit) } else if r.hovered() { Some(lit.gamma_multiply(0.75)) } else if show_off { Some(off) } else { None };
    if let Some(color) = color {
        match s {
            Skin::Wired => pixels(ui.painter(), rect.center(), HEART, 2.0, color),
            Skin::Swiss => heart(ui.painter(), rect.center(), 6.5, color),
            Skin::Ascii => { ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, if on || r.hovered() { "<3" } else { "--" }, mono(15.0, true), color); }
        }
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if on { "Remove from Liked Songs" } else { "Save to Liked Songs" })
}

/// The "now playing" mark in the number column, left-centred at `at`: three equaliser bars that bounce
/// while `playing` (time `t`) and rest when paused.
pub fn now_mark(painter: &egui::Painter, at: Pos2, color: Color32, s: Skin, playing: bool, t: f64) {
    if s == Skin::Ascii {
        // `>>`, blinking while paused.
        if playing || (t * 2.0) as i64 % 2 == 0 { painter.text(at, egui::Align2::LEFT_CENTER, ">>", mono(18.0, true), color); }
        return;
    }
    for i in 0..3 {
        let h = if playing { 4.0 + 12.0 * ((t * 5.0 + i as f64 * 1.9).sin().abs() as f32) } else { [6.0, 10.0, 8.0][i] };
        let h = if s.blocky() { (h / 2.0).round() * 2.0 } else { h }; // pixel bars move in whole 2px steps
        let x = i as f32 * 6.0;
        painter.rect_filled(Rect::from_min_max(at + vec2(x, 8.0 - h), at + vec2(x + 4.0, 8.0)), 0.0, color);
    }
}

/// While loading: a placeholder row with blank bars. `scan` is the row the highlight is on.
pub fn ghost_row(ui: &mut egui::Ui, w: f32, cols: [f32; 4], scan: bool) {
    let (l, p, s) = (look(ui.ctx()), pal(ui.ctx()), skin(ui.ctx()));
    let (rect, _) = ui.allocate_exact_size(vec2(w, l.row_h), Sense::hover());
    let [c_num, c_title, c_album, c_dur] = cols.map(|c| rect.left() + c);
    let painter = ui.painter();
    let y = rect.center().y;
    if s == Skin::Ascii {
        // `= = = =` where the text will be; the scan line is red.
        let ink = if scan { p.accent } else { p.line };
        for (x, n) in [(c_num, 2), (c_title, 10), (c_album, 7), (c_dur - 36.0, 3)] {
            painter.text(pos2(x, y), egui::Align2::LEFT_CENTER, vec!["="; n].join(" "), mono(14.0, false), ink);
        }
        return;
    }
    let bar = if scan { p.line } else { p.surface };
    if scan {
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
    }
    let two_lines = s == Skin::Wired;
    let title_w = (c_album - c_title - 40.0).clamp(40.0, 220.0);
    let block = |x: f32, dy: f32, bw: f32, h: f32| { painter.rect_filled(Rect::from_min_size(pos2(x, y + dy), vec2(bw, h)), 0.0, bar); };
    block(c_num, -3.0, 14.0, 6.0);
    block(c_title, if two_lines { -9.0 } else { -4.0 }, title_w, 8.0);
    if two_lines { block(c_title, 5.0, title_w * 0.55, 6.0) }
    block(c_album, -3.0, 140.0_f32.min(c_dur - c_album - 80.0).max(0.0), 6.0);
    block(c_dur - 36.0, -4.0, 36.0, 8.0);
    match s {
        Skin::Wired => painter.extend(Shape::dashed_line(&[rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, p.line), 1.0, 2.0)),
        Skin::Swiss | Skin::Ascii => { painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, p.line)); }
    }
}

// ---------- player controls

#[derive(Clone, Copy, PartialEq)]
pub enum Ctl {
    Prev,
    Play,
    Pause,
    Next,
}

/// Size of a transport button; `main` = play/pause.
pub fn ctl_size(s: Skin, main: bool) -> Vec2 {
    match (s, main) {
        (Skin::Wired, true) => vec2(52.0, 44.0),
        (Skin::Wired, false) => vec2(44.0, 44.0),
        (Skin::Swiss, true) => vec2(70.0, 30.0),
        (Skin::Swiss, false) => vec2(56.0, 30.0),
        (Skin::Ascii, true) => vec2(64.0, 34.0),
        (Skin::Ascii, false) => vec2(52.0, 34.0),
    }
}

/// A transport button: Wired square pixel button, Swiss plain words.
pub fn ctl_button(ui: &mut egui::Ui, ctl: Ctl) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let main = matches!(ctl, Ctl::Play | Ctl::Pause);
    let (rect, r) = ui.allocate_exact_size(ctl_size(s, main), Sense::click());
    let hover = r.hovered();
    let c = rect.center();
    let painter = ui.painter();
    match s {
        Skin::Wired => {
            let ink = if main {
                painter.rect_filled(rect, 0.0, p.accent);
                p.on_accent
            } else {
                painter.rect_filled(rect, 0.0, p.surface);
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, if hover { p.accent } else { p.line }), StrokeKind::Inside);
                if hover { p.text } else { p.dim }
            };
            let icon = match ctl { Ctl::Prev => PREV, Ctl::Next => NEXT, Ctl::Play => PLAY, Ctl::Pause => PAUSE };
            pixels(painter, c, icon, 2.0, ink);
        }
        Skin::Swiss => {
            let text = match ctl { Ctl::Prev => "Prev", Ctl::Next => "Next", Ctl::Play => "Play", Ctl::Pause => "Pause" };
            let (font, ink) = if main { (title(16.0), p.accent) } else { (FontId::proportional(15.0), if hover { p.accent } else { p.text }) };
            painter.text(c, egui::Align2::CENTER_CENTER, text, font, ink);
        }
        Skin::Ascii => {
            let text = match ctl { Ctl::Prev => "[<<]", Ctl::Next => "[>>]", Ctl::Play => "[>]", Ctl::Pause => "[||]" };
            let ink = if main && fills(p, s) {
                painter.rect_filled(rect, 0.0, if hover { p.accent.gamma_multiply(0.85) } else { p.accent });
                p.on_accent
            } else if main != hover { p.accent } else { p.text }; // play/pause printed in the accent; hover swaps
            painter.text(c, egui::Align2::CENTER_CENTER, text, mono(17.0, true), ink);
        }
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A slider: Wired block meter (`volume` makes the blocks rise), Swiss thin rail.
/// Returns the response and where along it (0..=1) the pointer is while pressed.
pub fn slider(ui: &mut egui::Ui, width: f32, frac: f32, volume: bool) -> (Response, Option<f32>) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let h = if s == Skin::Wired && !volume { 10.0 } else if s == Skin::Ascii { 20.0 } else { 16.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(width, h), Sense::click_and_drag());
    let frac = frac.clamp(0.0, 1.0);
    match s {
        Skin::Wired => {
            let n = if volume { 10 } else { 40 };
            let bw = (rect.width() - 2.0 * (n - 1) as f32) / n as f32;
            let lit = if volume { p.text } else { p.accent };
            for i in 0..n {
                let bh = if volume { rect.height() * (0.4 + 0.6 * i as f32 / (n - 1) as f32) } else { rect.height() };
                let x = rect.left() + i as f32 * (bw + 2.0);
                ui.painter().rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - bh), pos2(x + bw, rect.bottom())), 0.0, if (i as f32) < frac * n as f32 { lit } else { p.surface });
            }
        }
        Skin::Swiss => {
            let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), 2.0));
            ui.painter().rect_filled(rail, 0.0, p.track);
            let fill = Rect::from_min_size(rail.min - vec2(0.0, 1.0), vec2(rail.width() * frac, 4.0));
            ui.painter().rect_filled(fill, 0.0, if volume { p.text } else { p.accent });
        }
        Skin::Ascii => char_bar(ui.painter(), rect, frac, if volume { '=' } else { '#' }, if volume { p.text } else { p.accent }, p.dim),
    }
    let at = r.interact_pointer_pos().map(|q| ((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    (r.on_hover_cursor(egui::CursorIcon::PointingHand), at)
}

/// Loading progress: `done/total` filled, or a short segment looping along while the total is unknown.
pub fn progress(ui: &mut egui::Ui, width: f32, done: usize, total: Option<usize>) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let t = ui.input(|i| i.time);
    if s == Skin::Ascii {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        match total {
            Some(tot) if tot > 0 => char_bar(ui.painter(), rect, done as f32 / tot as f32, '#', p.accent, p.dim),
            _ => {
                // Unknown total: three `#` walking along.
                let font = mono(15.0, true);
                let n = ((width / cell(ui.painter(), &font).x) as usize).saturating_sub(2).max(4);
                let head = (t * 12.0) as usize % n;
                let inner: String = (0..n).map(|i| if (i + n - head) % n < 3 { '#' } else { '-' }).collect();
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, format!("[{inner}]"), font, p.accent);
            }
        }
        return;
    }
    if s == Skin::Wired {
        let n = (width / 8.0) as usize;
        let (rect, _) = ui.allocate_exact_size(vec2(n as f32 * 8.0 - 2.0, 10.0), Sense::hover());
        let head = (t * 12.0) as usize % n.max(1);
        for i in 0..n {
            let lit = match total { Some(tot) if tot > 0 => i * tot < done * n, _ => (i + n - head) % n < 3 };
            ui.painter().rect_filled(Rect::from_min_size(rect.min + vec2(i as f32 * 8.0, 0.0), vec2(6.0, 10.0)), 0.0, if lit { p.accent } else { p.surface });
        }
        return;
    }
    let h = 3.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.track);
    let seg = match total {
        Some(tot) if tot > 0 => Rect::from_min_size(rect.min, vec2(rect.width() * (done as f32 / tot as f32).min(1.0), h)),
        _ => {
            let x = ((t * 0.8).fract() as f32) * (rect.width() + 60.0) - 60.0;
            Rect::from_min_max(pos2((rect.left() + x).max(rect.left()), rect.top()), pos2((rect.left() + x + 60.0).min(rect.right()), rect.bottom()))
        }
    };
    if seg.width() > 0.5 {
        ui.painter().rect_filled(seg, 0.0, p.accent);
    }
}

/// A decorative spectrum meter, centred: bars that bounce on a rhythm while `playing` (librespot doesn't
/// hand us audio levels, so it's for feel, not measurement). Wired's move in whole steps.
pub fn meter(ui: &mut egui::Ui, width: f32, height: f32, t: f64, playing: bool) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if s == Skin::Ascii {
        // Columns of `|`, a `:` on top, in whole character steps.
        let font = mono(13.0, true);
        let c = cell(ui.painter(), &font);
        let (cols, rows) = ((rect.width() / (c.x * 2.0)) as usize, (rect.height() / c.y).max(1.0) as usize);
        let phase = if playing { t } else { 0.0 };
        for i in 0..cols {
            let x = i as f64;
            let f = 0.18 + 0.8 * ((x * 0.55 + phase * 3.1).sin() * (x * 0.17 + phase * 1.7).cos()).abs();
            let lit = ((f * rows as f64).round() as usize).clamp(1, rows);
            let col: Vec<&str> = (0..rows).map(|r| if r < rows - lit { " " } else if r == rows - lit { ":" } else { "|" }).collect();
            ui.painter().text(pos2(rect.left() + i as f32 * c.x * 2.0, rect.top()), egui::Align2::LEFT_TOP, col.join("\n"), font.clone(), p.accent.gamma_multiply(0.45 + 0.55 * f as f32));
        }
        return;
    }
    let n = 32;
    let bw = (rect.width() - 3.0 * (n - 1) as f32) / n as f32;
    let phase = if playing { t } else { 0.0 };
    for i in 0..n {
        let x = i as f64;
        let mut f = (0.18 + 0.8 * ((x * 0.55 + phase * 3.1).sin() * (x * 0.17 + phase * 1.7).cos()).abs()) as f32;
        if s.blocky() { f = (f * 10.0).round() / 10.0 }
        let left = rect.left() + i as f32 * (bw + 3.0);
        let bar = Rect::from_min_max(pos2(left, rect.bottom() - rect.height() * f), pos2(left + bw, rect.bottom()));
        ui.painter().rect_filled(bar, 0.0, p.accent.gamma_multiply(0.45 + 0.55 * f));
    }
}

/// A text button for the list header ("PLAY" / "SHUFFLE"); `primary` is the loud one.
pub fn action_button(ui: &mut egui::Ui, text: &str, primary: bool) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let font = match s {
        Skin::Wired => num(20.0),
        Skin::Swiss => title(14.0),
        Skin::Ascii => mono(15.0, true),
    };
    let text = if s == Skin::Ascii { format!("[ {} ]", text.trim_start_matches("▶ ")) } else { text.to_string() };
    // PLACEHOLDER: the colour is picked below, per skin and state.
    let g = ui.painter().layout_no_wrap(text, font, Color32::PLACEHOLDER);
    let (rect, r) = ui.allocate_exact_size(g.size() + vec2(28.0, 12.0), Sense::click());
    let hover = r.hovered();
    let painter = ui.painter();
    let ink = match s {
        Skin::Swiss => {
            painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0_f32, if primary || hover { p.accent } else { p.text }));
            if primary { p.accent } else { p.text }
        }
        Skin::Ascii if primary && !fills(p, s) => if hover { p.text } else { p.accent },
        Skin::Wired | Skin::Ascii if primary => {
            painter.rect_filled(rect, 0.0, if hover { p.accent.gamma_multiply(0.85) } else { p.accent });
            p.on_accent
        }
        Skin::Ascii => if hover { p.accent } else { p.text },
        Skin::Wired => {
            painter.rect(rect, 0.0, if hover { p.hover } else { p.surface }, Stroke::new(1.0_f32, p.line), StrokeKind::Inside);
            p.text
        }
    };
    painter.galley(rect.center() - g.size() / 2.0, g, ink);
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
pub enum Glyph {
    /// Lyrics / the focus view.
    Note,
    /// The skin and theme menu.
    Menu,
    Shuffle,
    Repeat,
    /// Repeat the one song.
    RepeatOne,
}

/// A small square icon button in the skin's style; `on` lights it (e.g. the focus view is open).
pub fn icon_button(ui: &mut egui::Ui, glyph: Glyph, on: bool) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let hover = r.hovered();
    let painter = ui.painter();
    let c = rect.center();
    match s {
        Skin::Wired => {
            let fill = if on { p.accent } else if hover { p.hover } else { p.surface };
            painter.rect(rect, 0.0, fill, Stroke::new(1.0_f32, if on || hover { p.accent } else { p.line }), StrokeKind::Inside);
            let ink = if on { p.on_accent } else if hover { p.text } else { p.dim };
            let icon = match glyph { Glyph::Note => NOTE, Glyph::Menu => MENU, Glyph::Shuffle => SHUFFLE, Glyph::Repeat | Glyph::RepeatOne => REPEAT };
            pixels(painter, c, icon, 2.0, ink);
            if matches!(glyph, Glyph::RepeatOne) {
                painter.rect_filled(Rect::from_center_size(c + vec2(10.0, -9.0), vec2(8.0, 8.0)), 0.0, p.accent);
                painter.text(c + vec2(10.0, -9.0), egui::Align2::CENTER_CENTER, "1", num(11.0), p.on_accent);
            }
        }
        Skin::Ascii => {
            let text = match glyph { Glyph::Note => "[*]", Glyph::Menu => "[=]", Glyph::Shuffle => "[~]", Glyph::Repeat => "[o]", Glyph::RepeatOne => "[1]" };
            let ink = if on { p.accent } else if hover { p.text } else { p.dim };
            painter.text(c, egui::Align2::CENTER_CENTER, text, mono(16.0, true), ink);
        }
        Skin::Swiss => {
            let ink = if on || hover { p.accent } else { p.dim };
            let st = Stroke::new(1.6_f32, ink);
            match glyph {
                Glyph::Note => {
                    painter.circle_filled(c + vec2(-3.0, 4.5), 3.5, ink);
                    painter.line_segment([c + vec2(0.3, 4.5), c + vec2(0.3, -7.0)], st);
                    painter.line_segment([c + vec2(0.3, -7.0), c + vec2(5.0, -4.0)], st);
                }
                Glyph::Menu => {
                    for dy in [-5.0, 0.0, 5.0] {
                        painter.line_segment([c + vec2(-7.0, dy), c + vec2(7.0, dy)], Stroke::new(1.8_f32, ink));
                    }
                }
                Glyph::Shuffle => {
                    painter.line_segment([c + vec2(-8.0, -4.0), c + vec2(6.0, 4.0)], st);
                    painter.line_segment([c + vec2(-8.0, 4.0), c + vec2(6.0, -4.0)], st);
                    tri(painter, c + vec2(7.0, -4.0), 6.0, 1.0, ink);
                    tri(painter, c + vec2(7.0, 4.0), 6.0, 1.0, ink);
                }
                Glyph::Repeat | Glyph::RepeatOne => {
                    painter.rect_stroke(Rect::from_center_size(c, vec2(15.0, 9.0)), 0.0, st, StrokeKind::Middle);
                    tri(painter, c + vec2(2.0, -4.5), 6.0, 1.0, ink);
                    if matches!(glyph, Glyph::RepeatOne) {
                        painter.rect_filled(Rect::from_center_size(c + vec2(9.0, -8.0), vec2(9.0, 9.0)), 0.0, p.accent);
                        painter.text(c + vec2(9.0, -8.0), egui::Align2::CENTER_CENTER, "1", FontId::proportional(8.0), p.on_accent);
                    }
                }
            }
        }
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One accent swatch in the skin menu: `color` filled in (None: the empty custom slot, a `+`), ringed when `kept`.
/// ASCII: `##` in the colour, `[##]` when kept.
pub fn swatch(ui: &mut egui::Ui, color: Option<Color32>, kept: bool) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let size = if s == Skin::Ascii { vec2(36.0, 24.0) } else { Vec2::splat(26.0) };
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if s == Skin::Ascii {
        let text = match (color, kept) { (None, _) => " + ", (Some(_), true) => "[##]", (Some(_), false) => " ## " };
        let ink = color.unwrap_or(if r.hovered() { p.text } else { p.dim });
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, mono(16.0, true), ink);
        if r.hovered() && !kept { painter.text(rect.center_bottom(), egui::Align2::CENTER_CENTER, "--", mono(12.0, false), p.dim); }
        return r.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    match color {
        Some(c) => { painter.rect_filled(rect, 0.0, c); }
        None => {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, if r.hovered() { p.text } else { p.dim }), StrokeKind::Inside);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "+", FontId::proportional(18.0), if r.hovered() { p.text } else { p.dim });
        }
    }
    if kept || (r.hovered() && color.is_some()) {
        let ring = if kept { p.text } else { p.dim };
        painter.rect_stroke(rect.expand(3.0), 0.0, Stroke::new(2.0_f32, ring), StrokeKind::Inside);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The custom accent's hue bar: the rainbow is the track, the handle is the skin's (a block in Wired, a thin rule
/// in Swiss). `hue` is 0..1; returns the new one while it's clicked or dragged.
pub fn hue_bar(ui: &mut egui::Ui, width: f32, hue: f32) -> Option<f32> {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let (rect, r) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(width, if s == Skin::Swiss { 10.0 } else { 14.0 }));
    let mut mesh = egui::Mesh::default();
    let n = 36;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let c: Color32 = egui::ecolor::Hsva::new(t, 1.0, 1.0, 1.0).into();
        let x = track.left() + track.width() * t;
        mesh.colored_vertex(pos2(x, track.top()), c);
        mesh.colored_vertex(pos2(x, track.bottom()), c);
        if i > 0 {
            let v = 2 * i as u32;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v, v + 1);
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    let x = track.left() + track.width() * hue.clamp(0.0, 1.0);
    match s {
        Skin::Swiss => { painter.rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(3.0, 24.0)), 0.0, p.text); }
        _ => {
            let handle = Rect::from_center_size(pos2(x, rect.center().y), vec2(8.0, 24.0));
            painter.rect_filled(handle, 0.0, p.panel);
            painter.rect_filled(handle.shrink(2.0), 0.0, p.text);
        }
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).interact_pointer_pos().map(|q| ((q.x - track.left()) / track.width()).clamp(0.0, 1.0))
}

/// Wired's block cursor after a big line of text.
pub fn cursor_after(ui: &egui::Ui, text: Rect, height: f32) {
    let s = skin(ui.ctx());
    if s == Skin::Swiss { return }
    if s == Skin::Ascii {
        let at = pos2(text.right() + 6.0, text.bottom() - 4.0);
        if at.x + height * 0.5 < ui.max_rect().right() { ui.painter().text(at, egui::Align2::LEFT_BOTTOM, "_", mono(height * 0.9, true), pal(ui.ctx()).accent); }
        return;
    }
    let block = Rect::from_min_size(pos2(text.right() + 8.0, text.bottom() - height - 4.0), vec2(height * 0.4, height));
    if block.right() < ui.max_rect().right() {
        ui.painter().rect_filled(block, 0.0, pal(ui.ctx()).accent);
    }
}

#[test]
fn every_skin_lays_out_text_in_every_font() {
    // Font files that fail to parse only blow up when first used, so render a frame per skin.
    for s in Skin::ALL {
        let ctx = egui::Context::default();
        apply(&ctx, s);
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Liked Songs ライブラリ ♥ ♫ 03:05");
                    ui.label(RichText::new("TITLE 12").font(title(20.0)));
                    ui.label(RichText::new("NUM 12").font(num(12.0)));
                });
            });
        }
    }
}
