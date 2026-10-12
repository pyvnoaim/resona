//! Skins: how the app looks. Each skin has a dark and a light palette, its own fonts and its own
//! widgets (window frames, covers, buttons, sliders, rows). main.rs lays things out; this draws them.
//!
//! - Wired: Lain-style cyber/pixel. Square windows with title bars, red on black, pixel fonts.
//! - Swiss: print-style. No chrome, a strict grid, huge type, one red rule.

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
}

impl Skin {
    pub const ALL: [Skin; 2] = [Skin::Wired, Skin::Swiss];

    /// The terminal look: block meters, pixel icons, block cursors, zero-padded times.
    pub fn blocky(self) -> bool {
        self == Skin::Wired
    }
    pub fn label(self) -> &'static str {
        match self {
            Skin::Wired => "Wired",
            Skin::Swiss => "Swiss",
        }
    }
}

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

pub fn skin(ctx: &egui::Context) -> Skin {
    ctx.data(|d| d.get_temp(egui::Id::new("skin"))).unwrap_or_default()
}

pub fn look(ctx: &egui::Context) -> &'static Look {
    match skin(ctx) {
        Skin::Wired => &WIRED,
        Skin::Swiss => &SWISS,
    }
}

pub fn pal(ctx: &egui::Context) -> &'static Pal {
    let l = look(ctx);
    if ctx.theme() == egui::Theme::Light { &l.light } else { &l.dark }
}

/// Big titles: VT323 / Archivo ExtraBold.
pub fn title(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("title".into()))
}

/// Numbers, timecodes, small labels: VT323 / Archivo ExtraBold.
pub fn num(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("num".into()))
}

/// Switches the whole app to `skin`: palettes, square widgets and fonts. Takes effect next frame.
pub fn apply(ctx: &egui::Context, skin: Skin) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("skin"), skin));
    let l = look(ctx);
    for (theme, p) in [(egui::Theme::Dark, &l.dark), (egui::Theme::Light, &l.light)] {
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
    });
    ctx.set_fonts(fonts(skin));
}

/// All bundled fonts are OFL (licenses in assets/).
fn fonts(skin: Skin) -> egui::FontDefinitions {
    let mut f = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("dot", &include_bytes!("../assets/DotGothic16-Regular.ttf")[..]),
        ("vt", include_bytes!("../assets/VT323-Regular.ttf")),
        ("archivo", include_bytes!("../assets/Archivo-Regular.ttf")),
        ("archivo-heavy", include_bytes!("../assets/Archivo-ExtraBold.ttf")),
    ] {
        f.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    let (body, heading) = match skin {
        Skin::Wired => ("dot", "vt"),
        Skin::Swiss => ("archivo", "archivo-heavy"),
    };
    let defaults = f.families[&FontFamily::Proportional].clone();
    // DotGothic16 covers kana; egui's defaults cover the rest (♥, ♫, arrows).
    let chain = |first: &str| -> Vec<String> { [first, "dot"].map(String::from).into_iter().chain(defaults.iter().cloned()).collect() };
    f.families.insert(FontFamily::Proportional, chain(body));
    f.families.insert(FontFamily::Name("title".into()), chain(heading));
    f.families.insert(FontFamily::Name("num".into()), chain(heading));
    f
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
    let stroke = match s {
        Skin::Wired => Stroke::new(1.0_f32, if active { p.accent } else { p.line }),
        Skin::Swiss => Stroke::NONE,
    };
    egui::Frame::new().fill(p.panel).stroke(stroke).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        let rect = ui.max_rect();
        match s {
            Skin::Swiss if win == Win::Player => { ui.painter().hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0_f32, p.text)); }
            Skin::Swiss => {}
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

/// The search field. Returns the text edit's response (Enter = `lost_focus` + Enter pressed).
pub fn search_box(ui: &mut egui::Ui, query: &mut String) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let id = egui::Id::new("search");
    let focused = ui.memory(|m| m.has_focus(id));
    let (margin, hint, font, fill, stroke) = match s {
        Skin::Wired => (egui::Margin::symmetric(8, 4), "search spotify_", num(20.0), p.surface, Stroke::new(1.0_f32, if focused { p.accent } else { p.line })),
        Skin::Swiss => (egui::Margin::symmetric(0, 6), "Search", FontId::proportional(14.0), Color32::TRANSPARENT, Stroke::NONE),
    };
    let r = egui::Frame::new().fill(fill).stroke(stroke).inner_margin(margin).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if s == Skin::Wired {
                // A fixed red ">" prompt; typing starts right after it.
                ui.label(RichText::new(">").font(num(20.0)).color(p.accent));
            }
            ui.add(egui::TextEdit::singleline(query).id(id).frame(false).hint_text(hint).font(font).margin(Vec2::ZERO).desired_width(f32::INFINITY))
        })
        .inner
    });
    if s == Skin::Swiss {
        // Print-style field: just a rule under the text.
        let rect = r.response.rect;
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(2.0_f32, if focused { p.accent } else { p.text }));
    }
    r.inner
}

// ---------- covers

/// What to stamp on a cover that has no art.
pub enum Stamp<'a> {
    Liked,
    /// The play queue: a "play + list" glyph.
    Queue,
    /// Search results: a magnifying glass.
    Search,
    Text(&'a str),
    None,
}

/// Album art, or this skin's stand-in. `big` = the playlist header (Wired adds crosshairs).
pub fn cover(ui: &mut egui::Ui, url: Option<&str>, size: f32, stamp: Stamp, big: bool) -> Rect {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
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
        (Skin::Wired, Stamp::Liked) => {
            painter.rect_filled(rect, 0.0, p.accent);
            pixels(painter, rect.center(), HEART, (size / 11.0).floor().max(3.0), p.on_accent);
        }
        (Skin::Swiss, Stamp::Liked) => {
            painter.rect_filled(rect, 0.0, p.accent);
            heart(painter, rect.center(), size * 0.24, Color32::WHITE);
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
    }
    match (s, current) {
        (Skin::Swiss, true) => (p.on_selected, p.on_selected),
        (Skin::Wired, true) => (p.on_selected, p.on_selected.gamma_multiply(0.85)),
        _ => (p.text, p.dim),
    }
}

/// A like heart in `rect`: lit when `on`, dim when off but `show_off` (a hovered row, the player bar).
/// `current` = it sits on the playing row's fill.
pub fn heart_toggle(ui: &mut egui::Ui, rect: Rect, id: egui::Id, on: bool, show_off: bool, current: bool) -> Response {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let r = ui.interact(rect, id, Sense::click());
    let lit = if current && s == Skin::Wired { p.on_selected } else { p.accent };
    let off = if current { p.on_selected.gamma_multiply(0.45) } else { p.dim.gamma_multiply(0.6) };
    let color = if on { Some(lit) } else if r.hovered() { Some(lit.gamma_multiply(0.75)) } else if show_off { Some(off) } else { None };
    if let Some(color) = color {
        if s.blocky() { pixels(ui.painter(), rect.center(), HEART, 2.0, color) } else { heart(ui.painter(), rect.center(), 6.5, color) }
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if on { "Remove from Liked Songs" } else { "Save to Liked Songs" })
}

/// The "now playing" mark in the number column, left-centred at `at`: three equaliser bars that bounce
/// while `playing` (time `t`) and rest when paused.
pub fn now_mark(painter: &egui::Painter, at: Pos2, color: Color32, s: Skin, playing: bool, t: f64) {
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
        Skin::Swiss => { painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, p.line)); }
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
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A slider: Wired block meter (`volume` makes the blocks rise), Swiss thin rail.
/// Returns the response and where along it (0..=1) the pointer is while pressed.
pub fn slider(ui: &mut egui::Ui, width: f32, frac: f32, volume: bool) -> (Response, Option<f32>) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let h = if s == Skin::Wired && !volume { 10.0 } else { 16.0 };
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
    }
    let at = r.interact_pointer_pos().map(|q| ((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    (r.on_hover_cursor(egui::CursorIcon::PointingHand), at)
}

/// Loading progress: `done/total` filled, or a short segment looping along while the total is unknown.
pub fn progress(ui: &mut egui::Ui, width: f32, done: usize, total: Option<usize>) {
    let (p, s) = (pal(ui.ctx()), skin(ui.ctx()));
    let t = ui.input(|i| i.time);
    if s.blocky() {
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
    };
    // PLACEHOLDER: the colour is picked below, per skin and state.
    let g = ui.painter().layout_no_wrap(text.into(), font, Color32::PLACEHOLDER);
    let (rect, r) = ui.allocate_exact_size(g.size() + vec2(28.0, 12.0), Sense::click());
    let hover = r.hovered();
    let painter = ui.painter();
    let ink = match s {
        Skin::Swiss => {
            painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0_f32, if primary || hover { p.accent } else { p.text }));
            if primary { p.accent } else { p.text }
        }
        Skin::Wired if primary => {
            painter.rect_filled(rect, 0.0, if hover { p.accent.gamma_multiply(0.85) } else { p.accent });
            p.on_accent
        }
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

/// Wired's block cursor after a big line of text.
pub fn cursor_after(ui: &egui::Ui, text: Rect, height: f32) {
    if !skin(ui.ctx()).blocky() { return }
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
