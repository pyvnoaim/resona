//! The views: sidebar, track list, player bar, focus view, help overlay and the sign-in screen.


use eframe::egui::{self, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::skin::{self, Ctl, Skin, Stamp, Win};
use crate::{App, api, settings::*, state::*};

/// "3:05", or "03:05" for Wired's fixed-width terminal look.
pub fn mmss(ms: u32, pad: bool) -> String {
    let (m, s) = (ms / 60_000, ms / 1000 % 60);
    if pad { format!("{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// "Late Night Piano" → "late_night_piano", for the window title.
pub fn slug(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

/// "Late Night Piano" → "LN", stamped on covers that have no art.
pub fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

/// The readable head of an error: "token: Service unavailable { …debug dump… }" → "token: Service unavailable".
pub fn short_err(e: &str) -> String {
    let head = e.split_whitespace().collect::<Vec<_>>().join(" ");
    let head = head.split(" {").next().unwrap_or_default().trim();
    if head.chars().count() > 90 { format!("{}…", head.chars().take(90).collect::<String>()) } else { head.to_string() }
}

/// "4 h 12 min" / "33 min" for a list's total length.
pub fn total_length(ms: u64) -> String {
    let min = ms / 60_000;
    if min >= 60 { format!("{} h {} min", min / 60, min % 60) } else { format!("{min} min") }
}

/// Small text in this skin's voice (counts, timecodes).
pub fn small(s: Skin) -> FontId {
    match s {
        Skin::Wired => skin::num(20.0),
        Skin::Swiss => FontId::proportional(13.0),
    }
}

/// What the song menu asks for after drawing.
#[derive(Default)]
pub struct SongMenu {
    pub play: bool,
    pub enqueue: Option<(api::Track, bool)>,
    pub like: Option<String>,
    pub add: Option<(String, String, String)>,
    pub create: Option<(String, String)>,
    pub members: bool,
    pub radio: Option<Source>,
}

/// A song's right-click menu, shared by the track list and the player bar. `play` offers "Play" (not for
/// the song already playing); `own` is the playlists it can be added to.
pub fn song_menu(ui: &mut egui::Ui, t: &api::Track, s: &State, me: &str, play: bool, out: &mut SongMenu) {
    if play && ui.button("Play").clicked() {
        out.play = true;
        ui.close();
    }
    if ui.button("Play next").clicked() {
        out.enqueue = Some((t.clone(), true));
        ui.close();
    }
    if ui.button("Add to queue").clicked() {
        out.enqueue = Some((t.clone(), false));
        ui.close();
    }
    let Some(id) = &t.id else { return };
    let liked = s.liked.contains(id);
    if ui.button(if liked { "Remove from Liked Songs" } else { "Save to Liked Songs" }).clicked() {
        out.like = Some(id.clone());
        ui.close();
    }
    ui.menu_button("Add to playlist", |ui| {
        out.members = true;
        // Named after the song, as Spotify does.
        if ui.button("+ New playlist").clicked() {
            out.create = Some((t.name.clone(), id.clone()));
            ui.close();
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (pid, name) in own_lists(s, me) {
                if s.members.get(&pid).is_some_and(|m| m.contains(id)) {
                    ui.add_enabled(false, egui::Button::new(format!("√ {name}"))).on_disabled_hover_text("Already in this playlist");
                } else if ui.button(&name).clicked() {
                    out.add = Some((pid, name, id.clone()));
                    ui.close();
                }
            }
        });
    });
    if ui.button("Go to song radio").clicked() {
        out.radio = Some(Source::Radio { track: id.clone(), name: t.name.clone() });
        ui.close();
    }
    ui.separator();
    if ui.button("Copy song link").clicked() {
        ui.ctx().copy_text(format!("https://open.spotify.com/track/{id}"));
        ui.close();
    }
}

/// The pages a song's names lead to.
fn artist_of(t: &api::Track) -> Option<Source> {
    t.artists.iter().find_map(|a| Some(Source::Artist { id: a.id.clone()?, name: a.name.clone() }))
}
fn album_of(t: &api::Track) -> Option<Source> {
    t.album.id.clone().map(|id| Source::Album { id, name: t.album.name.clone() })
}

/// A name that leads to a page: underlined on hover, the hand over it.
fn link(ui: &egui::Ui, at: Rect, id: egui::Id, color: egui::Color32) -> egui::Response {
    let r = ui.interact(at, id, Sense::click());
    if r.hovered() { ui.painter().hline(at.x_range(), at.bottom() - 1.0, Stroke::new(1.0_f32, color)); }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Playlists a track can be added to: yours and the collaborative ones, the ones you last added to first.
fn own_lists(s: &State, me: &str) -> Vec<(String, String)> {
    let mut own: Vec<_> = s.playlists.iter().filter(|pl| pl.web_readable(me)).map(|pl| (pl.id.clone(), pl.name.clone())).collect();
    own.sort_by_key(|(id, _)| s.added_to.iter().position(|p| p == id).unwrap_or(usize::MAX)); // stable: the rest keep sidebar order
    own
}

/// What the focus view asks for after drawing.
#[derive(Default)]
pub struct FocusOut {
    pub play: Option<usize>,
    pub like: Option<String>,
    pub back: bool,
    pub goto: Option<Source>,
}

/// The focus view: the sidebar steps aside and the song takes the screen with its lyrics; no lyrics: a signal
/// meter and the next three songs. Wired (design C): the cover's own pixels fill the background, the song sits
/// in the middle. Swiss (design A): plain paper, the cover on the left, the rest beside it under one black rule.
pub fn focus(ui: &mut egui::Ui, s: &State) -> FocusOut {
    let (l, p, sk) = (skin::look(ui.ctx()), skin::pal(ui.ctx()), skin::skin(ui.ctx()));
    let mut out = FocusOut::default();
    let full = ui.max_rect();
    let caps = !matches!(sk, Skin::Swiss);
    let Some(t) = &s.now else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Nothing playing yet.").font(small(sk)).color(p.dim)));
        return out;
    };
    let swiss = sk == Skin::Swiss;
    // Backdrop: the smallest cover, blown up without smoothing, under a dark veil.
    if let (false, Some(thumb)) = (swiss, t.thumb()) {
        let side = full.width().max(full.height());
        let uv_w = full.width() / side;
        let uv_h = full.height() / side;
        let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uv_w, uv_h));
        egui::Image::new(thumb).texture_options(egui::TextureOptions::NEAREST).uv(uv).paint_at(ui, full);
        ui.painter().rect_filled(full, 0.0, p.ground.gamma_multiply(0.68));
    }

    // Corners: the way back, and where we are.
    let hint = skin::num(if sk.blocky() { 20.0 } else { 13.0 });
    let back = if caps { "[ ESC ] ◀ LIBRARY" } else { "◀ Library" };
    let back_rect = Rect::from_min_size(full.min + vec2(22.0, 16.0), vec2(220.0, 24.0));
    let r = ui.interact(back_rect, egui::Id::new("focus-back"), Sense::click());
    ui.painter().text(back_rect.left_center(), egui::Align2::LEFT_CENTER, back, hint.clone(), if r.hovered() { p.accent } else { p.dim });
    out.back = r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    let here = if sk == Skin::Wired { "now.exe · 再生中" } else if caps { "NOW PLAYING" } else { "Now Playing" };
    ui.painter().text(full.right_top() + vec2(-22.0, 28.0), egui::Align2::RIGHT_CENTER, here, hint, p.dim);

    let lyrics = s.lyrics.as_ref().filter(|(id, _)| Some(id) == t.id.as_ref()).map(|(_, l)| l);
    let has_lyrics = matches!(lyrics, Some(Lyrics::Lines { .. }));
    let art = (full.height() * if has_lyrics { 0.44 } else { 0.38 }).clamp(140.0, 360.0);
    // Centre the whole stack vertically: cover plus roughly what goes under it.
    let below = if has_lyrics { 210.0 } else { 300.0 };
    let cover_url = t.album.images.first().map(|i| i.url.as_str());
    // Title, names, heart, then lyrics (or meter and up next): centred under the cover, or flush left beside it.
    let mut body = |ui: &mut egui::Ui, centred: bool, title: f32| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let name = RichText::new(t.name.clone()).font(skin::title(title));
        if centred { ui.add(egui::Label::new(name).truncate()); } else { ui.add(egui::Label::new(name).wrap()); }
        // Artist · album, then the heart; each name opens its page.
        // Cut to the column (less room for the heart): the artist gets up to 60%, the album the rest.
        let font = FontId::proportional(15.0);
        let room = (ui.available_width() - 40.0).max(60.0);
        let cut = |text: String, width: f32| {
            let mut job = egui::text::LayoutJob::simple_singleline(text, font.clone(), p.dim);
            job.wrap = egui::text::TextWrapping::truncate_at_width(width);
            ui.painter().layout_job(job)
        };
        let dot = cut(" · ".into(), room);
        let artist = cut(t.artist_line(), (room * 0.6).max(room - dot.size().x - ui.painter().layout_no_wrap(t.album.name.clone(), font.clone(), p.dim).size().x));
        let album = cut(t.album.name.clone(), (room - artist.size().x - dot.size().x).max(0.0));
        let size = vec2(artist.size().x + dot.size().x + album.size().x, artist.size().y);
        let (line, _) = ui.allocate_exact_size(size, Sense::hover());
        let artist_at = Rect::from_min_size(line.min, artist.size());
        let album_at = Rect::from_min_size(line.min + vec2(artist.size().x + dot.size().x, 0.0), album.size());
        ui.painter().galley(line.min + vec2(artist.size().x, 0.0), dot, p.dim);
        ui.painter().galley(artist_at.min, artist, p.dim);
        ui.painter().galley(album_at.min, album, p.dim);
        if link(ui, artist_at, egui::Id::new("focus-artist"), p.dim).clicked() { out.goto = artist_of(t) }
        if link(ui, album_at, egui::Id::new("focus-album"), p.dim).clicked() { out.goto = album_of(t) }
        let line = ui.interact(line, egui::Id::new("focus-line"), Sense::hover());
        if let Some(id) = &t.id {
            let heart = Rect::from_center_size(pos2(line.rect.right() + 18.0, line.rect.center().y), vec2(26.0, 26.0));
            if skin::heart_toggle(ui, heart, egui::Id::new("focus-like"), s.liked.contains(id), true, false).clicked() {
                out.like = Some(id.clone());
            }
        }
        ui.add_space(10.0);
        let (big, now_ink) = match sk {
            Skin::Wired => (skin::num(40.0), p.accent),
            Skin::Swiss => (skin::title(30.0), p.accent),
        };
        let small_line = FontId::new(big.size * 0.6, big.family.clone());
        let pos = s.position();
        match lyrics {
            Some(Lyrics::Lines { synced: true, lines }) => {
                // The lines roll up: the sung line big in the middle, the one just sung above and the next below,
                // smaller and fainter. A turning line crossfades between the two fixed sizes (in-between sizes
                // would fill the glyph atlas). Line -1 is the ♪ before the first.
                let cur = lines.iter().rposition(|(at, _)| *at <= pos).map_or(-1, |c| c as isize);
                let k = ui.ctx().animate_value_with_time(egui::Id::new("lyric-roll"), cur as f32, 0.5);
                let d = (cur as f32 - k).clamp(0.0, 1.0).powi(3); // 1 just moved on .. 0 settled, eased out
                let fonts = [big.clone(), small_line.clone()];
                let wrap = (ui.available_width() - 40.0).max(100.0);
                let height = (full.bottom() - ui.cursor().top() - 16.0).clamp(120.0, 200.0);
                let rect = ui.allocate_space(vec2(wrap, height)).1;
                let painter = ui.painter().with_clip_rect(rect);
                let text = |j: isize| match usize::try_from(j).ok().and_then(|j| lines.get(j)) {
                    Some((_, w)) if !w.is_empty() => w.as_str(),
                    _ if (-1..lines.len() as isize).contains(&j) => "♪",
                    _ => "",
                };
                let lay = |j: isize, f: usize| {
                    let mut job = egui::text::LayoutJob::simple(text(j).to_owned(), fonts[f].clone(), p.text, wrap);
                    job.halign = if centred { egui::Align::Center } else { egui::Align::LEFT };
                    painter.layout_job(job)
                };
                // Line j's height while line c is sung, and where its middle sits: stacked out from c in the middle.
                let h = |j: isize, c: isize| lay(j, (j - c).unsigned_abs().min(1)).size().y;
                let gap = 8.0;
                let mid = |j: isize, c: isize| {
                    let (mut y, mut i, step) = (rect.center().y, c, if j < c { -1 } else { 1 });
                    while i != j {
                        y += step as f32 * (h(i, c) / 2.0 + gap + h(i + step, c) / 2.0);
                        i += step;
                    }
                    y
                };
                for j in cur - 2..=cur + 2 {
                    let slot = (j - cur) as f32 + d; // 0 is the sung line, 1 the next, -1 the one just sung
                    let dist = slot.abs();
                    let y = mid(j, cur) * (1.0 - d) + mid(j, cur - 1) * d;
                    // Lines still to come show a little brighter than the ones already sung.
                    let fade = if slot < 0.0 { [1.0, 0.5, 0.0] } else { [1.0, 0.75, 0.0] };
                    let i = (dist as usize).min(1);
                    let alpha = fade[i] + (fade[i + 1] - fade[i]) * (dist - i as f32).min(1.0);
                    let ink = p.dim.lerp_to_gamma(now_ink, (1.0 - dist).max(0.0));
                    let t = if i == 1 { 0.0 } else { dist.fract() };
                    for (f, share) in [(i, 1.0 - t), (1, t)] {
                        if alpha * share < 0.01 { continue }
                        let g = lay(j, f);
                        let at = pos2(if centred { rect.center().x } else { rect.left() }, y - g.size().y / 2.0);
                        painter.galley_with_override_text_color(at, g, ink.gamma_multiply(alpha * share));
                    }
                }
            }
            Some(Lyrics::Lines { lines, .. }) => {
                // Not synced: the whole text, quietly, to scroll through.
                egui::ScrollArea::vertical().max_height(full.bottom() - ui.cursor().top() - 20.0).show(ui, |ui| {
                    for (_, words) in lines {
                        ui.label(RichText::new(words).font(small_line.clone()).color(p.text));
                    }
                });
            }
            Some(Lyrics::Loading) => { ui.label(RichText::new(if caps { "LOADING LYRICS…" } else { "Loading lyrics…" }).font(small(sk)).color(p.dim)); }
            Some(Lyrics::Missing) | None => {
                skin::meter(ui, (full.width() * 0.5).min(560.0), 64.0, ui.input(|i| i.time), s.since.is_some());
                let label = match sk { Skin::Wired => "NO LYRICS · 歌詞なし · UP NEXT >", Skin::Swiss => "No lyrics · Up next" };
                ui.label(RichText::new(label).font(small(sk)).color(p.dim));
                out.play = up_next(ui, s, pos, centred);
            }
        }
    };
    if swiss {
        // Five twelfths for the cover, the other seven for the words, tops aligned on the rule.
        let area = Rect::from_min_max(full.min + vec2(56.0, 72.0), full.max - vec2(56.0, 40.0));
        let gap = 40.0;
        let side = ((area.width() - gap) * 5.0 / 12.0).min(area.height()).min(560.0);
        let cover = Rect::from_min_size(pos2(area.left(), area.center().y - side / 2.0), vec2(side, side));
        skin::cover(&mut ui.new_child(egui::UiBuilder::new().max_rect(cover)), cover_url, side, Stamp::None, true);
        let words = Rect::from_min_max(pos2(cover.right() + gap, cover.top()), area.max);
        ui.painter().hline(words.x_range(), words.top(), Stroke::new(2.0_f32, p.text));
        let mut col = ui.new_child(egui::UiBuilder::new().max_rect(words.with_min_y(words.top() + 18.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        body(&mut col, false, (side * 0.16).clamp(36.0, 88.0));
    } else {
        ui.add_space(((full.height() - art - below) / 2.0).max(52.0));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            skin::cover(ui, cover_url, art, Stamp::None, true);
            ui.add_space(6.0);
            body(ui, true, (l.title_size * 0.8).clamp(26.0, 60.0));
        });
    }
    out
}

/// The next three songs in the queue as cards (as many as fit); the first says when it starts. Returns a clicked one.
pub fn up_next(ui: &mut egui::Ui, s: &State, pos: u32, centred: bool) -> Option<usize> {
    let p = skin::pal(ui.ctx());
    let left = s.now.as_ref().map_or(0, |t| t.duration_ms.saturating_sub(pos));
    // The songs playback would actually reach: refused ones are skipped there, so they're left out here too.
    let cards: Vec<(usize, &api::Track)> = (s.qi + 1..)
        .zip(s.queue.iter().skip(s.qi + 1))
        .filter(|(_, t)| !t.id.as_ref().is_some_and(|id| s.unplayable.contains(id)))
        .take(3)
        .collect();
    let (w, gap) = (230.0, 12.0);
    let fit = (((ui.available_width() + gap) / (w + gap)) as usize).max(1);
    let cards = &cards[..cards.len().min(fit)];
    if cards.is_empty() { return None }
    let total = cards.len() as f32 * w + (cards.len() - 1) as f32 * gap;
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if centred { ui.add_space((ui.available_width() - total) / 2.0) }
        let sk = skin::skin(ui.ctx());
        for (k, &(i, t)) in cards.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(w, 66.0), Sense::click());
            let first = k == 0;
            let (fill, ink, dim) = (p.panel.gamma_multiply(0.85), p.text, p.dim);
            {
                let border = if first || r.hovered() { p.accent } else { p.line };
                ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0_f32, border), StrokeKind::Inside);
            }
            let art = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 20.0), vec2(40.0, 40.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(art));
            skin::cover(&mut child, t.thumb(), 40.0, Stamp::None, false);
            // Three measured lines, centred in the card (fonts differ a lot in line height).
            let x = art.right() + 12.0;
            let width = rect.right() - x - 10.0;
            let line = |text: String, font: FontId, color| {
                let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
                job.wrap = egui::text::TextWrapping::truncate_at_width(width);
                ui.painter().layout_job(job)
            };
            let when = if first { format!("IN {}", mmss(left, true)) } else { "THEN".to_string() };
            let small_font = skin::num(if sk.blocky() { 17.0 } else { 12.0 }); // VT323 runs small
            let rows = [line(when, small_font, dim), line(t.name.clone(), FontId::proportional(13.0), ink), line(t.artist_line(), FontId::proportional(11.0), dim)];
            let mut y = rect.center().y - rows.iter().map(|g| g.size().y).sum::<f32>() / 2.0;
            for (g, color) in rows.into_iter().zip([dim, ink, dim]) {
                let h = g.size().y;
                ui.painter().galley(pos2(x, y), g, color);
                y += h;
            }
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { picked = Some(i) }
        }
    });
    picked
}

/// The "?" overlay: every shortcut, grouped, in the skin's own window style.
pub fn help_overlay(ctx: &egui::Context) {
    let (p, sk) = (skin::pal(ctx), skin::skin(ctx));
    let groups: [(&str, &[(&str, &str)]); 5] = [
        ("Playback", &[("Space", "Play / pause"), ("⌘ →", "Next song"), ("⌘ ←", "Previous song"), ("→  ←", "Forward / back 10 s"), ("S", "Shuffle on / off"), ("R", "Repeat: off / all / one")]),
        ("Move", &[("↑  ↓", "Up / down (J K too)"), ("Tab", "Sidebar / songs"), ("Enter", "Open the list / play the song"), ("PgUp PgDn", "Ten rows"), ("Home End", "Top / bottom")]),
        ("Volume", &[("+  -", "Louder / quieter"), ("M", "Mute / unmute")]),
        ("Find & go", &[("⌘ F", "Search Spotify"), ("⌘ J", "Jump to the playing song"), ("⌘ E", "Cover + lyrics view"), ("Esc", "Back / close")]),
        ("This song", &[("⌘ L", "Like / unlike"), ("L", "Like / unlike the highlighted song"), ("⌘ ⇧ C", "Copy song link"), ("Q", "Add the highlighted song to the queue"), ("⌘ ,", "Skin & theme menu"), ("?", "This list")]),
    ];
    egui::Area::new(egui::Id::new("help")).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        let stroke = Stroke::new(1.0_f32, if sk == Skin::Wired { p.accent } else { p.line });
        egui::Frame::new().fill(p.panel).stroke(stroke).inner_margin(egui::Margin::same(22)).show(ui, |ui| {
            ui.label(RichText::new(if sk.blocky() { "KEYS // キー" } else { "Keyboard shortcuts" }).font(small(sk)).color(p.dim));
            ui.add_space(8.0);
            egui::Grid::new("keys").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for (group, keys) in groups {
                    ui.label(RichText::new(group.to_uppercase()).font(skin::num(13.0)).color(p.accent));
                    ui.end_row();
                    for (key, what) in keys {
                        ui.label(RichText::new(*key).font(skin::num(15.0)).color(p.text));
                        ui.label(RichText::new(*what).size(13.0).color(p.dim));
                        ui.end_row();
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("? or Esc closes this").size(11.0).color(p.dim));
        });
    });
}

/// The ≡ button: a menu with every skin and theme, the current ones marked. Opens upwards from the
/// player bar.
pub fn switches(ui: &mut egui::Ui, ctx: &egui::Context) {
    use egui::ThemePreference::{Dark, Light, System};
    let r = skin::icon_button(ui, skin::Glyph::Menu, false).on_hover_text("Skin and theme (⌘,)");
    // ⌘, asks for the menu through a flag, since the shortcut isn't a click on this button.
    let asked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("open-menu"))).unwrap_or(false);
    if asked { egui::Popup::toggle_id(ctx, egui::Popup::default_response_id(&r)) }
    egui::Popup::menu(&r).align(egui::RectAlign::TOP_END).show(|ui| {
        let p = skin::pal(ui.ctx());
        let head = |ui: &mut egui::Ui, text: &str| { ui.label(RichText::new(text).size(11.0).color(p.dim)); };
        head(ui, "Skin");
        let current = skin::skin(ctx);
        for s in Skin::ALL {
            if ui.selectable_label(s == current, s.label()).clicked() {
                skin::apply(ctx, s);
                save_settings(ctx);
            }
        }
        ui.separator();
        head(ui, "Theme");
        let theme = ctx.options(|o| o.theme_preference);
        for (pref, name) in [(System, "Follow macOS"), (Dark, "Dark"), (Light, "Light")] {
            if ui.selectable_label(theme == pref, name).clicked() {
                ctx.set_theme(pref);
                save_settings(ctx);
            }
        }
    });
}

/// First launch: how to make your own Spotify app, and a field for its client ID. True when Save is pressed
/// with a valid one.
pub fn setup(ui: &mut egui::Ui, typed: &mut String, redirect: &str, error: Option<&str>) -> bool {
    let (p, s) = (skin::pal(ui.ctx()), skin::skin(ui.ctx()));
    let caps = s.blocky();
    let size = match s { Skin::Wired => 52.0, Skin::Swiss => 44.0 };
    let mut save = false;
    ui.add_space((ui.available_height() / 2.0 - 220.0).max(24.0));
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() / 2.0 - 300.0).max(24.0));
        ui.vertical(|ui| {
            ui.set_max_width(600.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(RichText::new("resona").font(skin::num(if caps { 22.0 } else { 14.0 })).color(p.dim));
            ui.add(egui::Label::new(RichText::new("Connect your Spotify app").font(skin::title(size)).color(p.text)).wrap());
            ui.add(egui::Label::new(RichText::new("Resona plays through your Spotify Premium account, and reads your library through a small app of your own on Spotify's developer site. It's free and takes two minutes.").size(14.0).color(p.dim)).wrap());
            ui.add_space(6.0);
            let step = |ui: &mut egui::Ui, n: u32, text: &str| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{n}")).font(skin::title(16.0)).color(p.accent));
                    ui.label(RichText::new(text).size(14.0).color(p.text));
                });
            };
            step(ui, 1, "Open the dashboard and create an app (any name; tick Web API).");
            ui.hyperlink_to(RichText::new("developer.spotify.com/dashboard").size(14.0).color(p.accent), "https://developer.spotify.com/dashboard");
            step(ui, 2, "Add this Redirect URI to the app, exactly:");
            ui.horizontal(|ui| {
                ui.label(RichText::new(redirect).font(FontId::monospace(14.0)).color(p.text));
                if skin::action_button(ui, if caps { "COPY" } else { "Copy" }, false).clicked() { ui.ctx().copy_text(redirect.into()) }
            });
            step(ui, 3, "Paste the app's Client ID here (no secret needed):");
            let ok = api::valid_client_id(typed.trim());
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(typed).hint_text("32 letters and digits").desired_width(340.0).font(FontId::monospace(14.0)));
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let button = ui.add_enabled_ui(ok, |ui| skin::action_button(ui, if caps { "CONNECT" } else { "Connect" }, true)).inner;
                save = ok && (button.clicked() || enter);
            });
            if !typed.trim().is_empty() && !ok {
                ui.label(RichText::new("That doesn't look like a client ID: it's 32 letters and digits.").size(12.0).color(p.accent));
            }
            if let Some(e) = error {
                ui.add(egui::Label::new(RichText::new(short_err(e)).size(12.0).color(p.accent)).wrap()).on_hover_text(e);
            }
            ui.add(egui::Label::new(RichText::new("Your browser then opens twice to sign in: once for playback, once for your app. The ID is kept in ~/.resona/client_id.").size(12.0).color(p.dim)).wrap());
        });
    });
    save
}

/// Before a playlist is open: the connect steps (with the failed one, if any), then a hint once we're in.
pub fn boot(ui: &mut egui::Ui, status: &str, error: Option<&str>) {
    let (p, s) = (skin::pal(ui.ctx()), skin::skin(ui.ctx()));
    let stage = match status {
        st if st.starts_with("Sign in") => Some(0),
        "Connecting…" => Some(1),
        "Loading your library…" => Some(2),
        _ => None,
    };
    let msg = match (error, stage) {
        (Some(_), Some(0)) => "Sign-in failed",
        (Some(_), Some(1)) => "Couldn't connect",
        (Some(_), Some(2)) => "Couldn't load your library",
        (Some(_), _) => "Something broke",
        (None, _) if status.is_empty() => "Pick a playlist or search on the left.",
        (None, _) => status,
    };
    let steps = ["sign in", "connect", "load library"];
    ui.add_space((ui.available_height() / 2.0 - 120.0).max(0.0));
    let body = |ui: &mut egui::Ui| {
        let (brand, size) = match s {
            Skin::Wired => (skin::num(22.0), 52.0),
            Skin::Swiss => (skin::num(14.0), 44.0),
        };
        ui.label(RichText::new("resona").font(brand).color(p.dim));
        let r = ui.add(egui::Label::new(RichText::new(msg).font(skin::title(size)).color(if error.is_some() { p.accent } else { p.text })).wrap());
        skin::cursor_after(ui, r.rect, 34.0);
        if let Some(stage) = stage {
            if s.blocky() {
                for (i, step) in steps.into_iter().enumerate() {
                    let (mark, c) = match i.cmp(&stage) {
                        std::cmp::Ordering::Less => ("OK", p.text),
                        std::cmp::Ordering::Equal if error.is_some() => ("!!", p.accent),
                        std::cmp::Ordering::Equal => ("..", p.text),
                        std::cmp::Ordering::Greater => ("  ", p.dim),
                    };
                    ui.label(RichText::new(format!("> [ {mark} ] {step}")).font(skin::num(22.0)).color(c));
                }
            } else {
                ui.add_space(8.0);
                skin::progress(ui, 240.0, stage + usize::from(error.is_none()), Some(3));
                let text = format!("STEP {}/3 · {}", stage + 1, steps[stage].to_uppercase());
                ui.label(RichText::new(text).font(small(s)).color(if error.is_some() { p.accent } else { p.dim }));
            }
        }
        if let Some(e) = error {
            ui.add_space(12.0);
            ui.add(egui::Label::new(RichText::new(format!("ERR: {}", short_err(e))).size(12.0).color(p.dim)).wrap()).on_hover_text(e);
            ui.label(RichText::new("Restart resona to try again.").size(12.0).color(p.dim));
        }
    };
    body(ui);
}

/// The sidebar's rows, top to bottom. Queue only shows up while you've queued songs by hand (or are looking at it).
pub fn side_lists(s: &State) -> Vec<Source> {
    let queue = (s.queued > 0 || s.view == Some(Source::Queue)).then_some(Source::Queue);
    [Source::Liked, Source::NewReleases].into_iter().chain(queue).chain(s.playlists.iter().map(|p| Source::Playlist(p.id.clone()))).collect()
}

/// The pointer moved (or the wheel moved the rows under it): the row it is over takes the cursor.
fn moused(ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.delta() != Vec2::ZERO || i.smooth_scroll_delta != Vec2::ZERO)
}

impl App {
    pub fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (l, p, sk) = (skin::look(ctx), skin::pal(ctx), skin::skin(ctx));
        let (mut pick, mut create) = (None, None);
        let (cursor, reveal, moused) = ((self.pane == Pane::Lists).then_some(self.side), self.reveal, moused(ui));
        let (mut at, mut point) = (0, None);
        let s = self.state.lock().unwrap();
        skin::window(ui, Win::Library, "library.lst", "ライブラリ", None, |ui| {
            let margin = match sk {
                Skin::Wired => egui::Margin::same(8),
                Skin::Swiss => egui::Margin { left: 30, right: 16, top: 30, bottom: 8 },
            };
            egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                if sk == Skin::Swiss {
                    skin::section(ui, "Resona");
                    ui.add_space(4.0);
                }
                let r = skin::text_field(ui, "search", "search spotify_", "Search", &mut self.query);
                let query = self.query.trim();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !query.is_empty() {
                    pick = Some(Source::Search(query.to_string()));
                }
                ui.add_space(match sk { Skin::Swiss => 12.0, _ => 4.0 });
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = match sk { Skin::Swiss => 0.0, _ => 4.0 };
                    let mut row = |ui: &mut egui::Ui, n: usize, src: Source, img: Option<&str>, name: &str, owner: &str| {
                        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), l.side_row_h), Sense::click());
                        let selected = s.view.as_ref() == Some(&src);
                        let on = cursor == Some(at);
                        if on && reveal { r.scroll_to_me(None) }
                        if moused && r.hovered() { point = Some(at) }
                        at += 1;
                        match sk {
                            Skin::Wired if selected || on => {
                                ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
                            }
                            _ => {}
                        }
                        let inset = if sk == Skin::Swiss { Vec2::ZERO } else { vec2(6.0, 6.0) };
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(inset)).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        if l.side_cover > 0.0 {
                            let initials = initials(name);
                            let stamp = match &src { Source::Liked => Stamp::Liked, Source::Queue => Stamp::Queue, _ => Stamp::Text(&initials) };
                            skin::cover(&mut child, img, l.side_cover, stamp, false);
                            child.add_space(10.0);
                            child.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add_space(3.0);
                                let ink = if selected && sk == Skin::Wired { p.accent } else { p.text };
                                ui.add(egui::Label::new(RichText::new(name).size(14.0).color(ink)).truncate());
                                let sub = match (&src, owner.is_empty()) {
                                    (Source::Queue, _) => "up next".to_string(),
                                    (Source::NewReleases, _) => "followed artists · 2 weeks".to_string(),
                                    (_, true) => "playlist".to_string(),
                                    _ => format!("playlist · {owner}"),
                                };
                                ui.add(egui::Label::new(RichText::new(sub).size(11.0).color(p.dim)).truncate());
                            });
                        } else {
                            match sk {
                                Skin::Swiss => {
                                    let ink = if selected || on { p.accent } else { p.text };
                                    child.add_sized(vec2(30.0, 20.0), egui::Label::new(RichText::new(format!("{:02}", n + 1)).size(13.0).color(if selected { p.accent } else { p.dim })));
                                    let font = if selected { skin::title(14.0) } else { FontId::proportional(14.0) };
                                    child.add(egui::Label::new(RichText::new(name).font(font).color(ink)).truncate());
                                }
                                _ => {
                                    let ink = if selected { p.on_selected } else { p.text };
                                    child.add(egui::Label::new(RichText::new(name.to_uppercase()).font(skin::num(13.0)).color(ink)).truncate());
                                }
                            }
                        }
                        if let Source::Playlist(id) = &src {
                            r.context_menu(|ui| {
                                if ui.button("Copy playlist link").clicked() {
                                    ui.ctx().copy_text(format!("https://open.spotify.com/playlist/{id}"));
                                    ui.close();
                                }
                            });
                        }
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { pick = Some(src) }
                    };
                    let liked_name = "Liked Songs";
                    row(ui, 0, Source::Liked, None, liked_name, "");
                    // The same rows, in the same order, as `side_lists`.
                    row(ui, 1, Source::NewReleases, None, "New releases", "");
                    if s.queued > 0 || s.view == Some(Source::Queue) {
                        row(ui, 2, Source::Queue, None, "Queue", "");
                    }
                    for (i, pl) in s.playlists.iter().enumerate() {
                        let img = pl.images.as_ref().and_then(|v| v.last()).map(|i| i.url.as_str());
                        let owner = pl.owner.as_ref().and_then(|o| o.display_name.as_deref()).unwrap_or("");
                        row(ui, i + 3, Source::Playlist(pl.id.clone()), img, &pl.name, owner);
                    }
                    ui.add_space(8.0);
                    let r = skin::text_field(ui, "new-list", "new playlist_", "New playlist", &mut self.new_list);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.new_list.trim().is_empty() {
                        create = Some(std::mem::take(&mut self.new_list).trim().to_string());
                    }
                });
            });
        });
        drop(s);
        if let Some(i) = point.filter(|i| cursor != Some(*i)) {
            (self.pane, self.side) = (Pane::Lists, i);
            ctx.request_repaint(); // draw the highlight where it is now
        }
        if let Some(name) = create { self.new_playlist(name, None, ctx) }
        if let Some(src) = pick {
            (self.big, self.pane) = (false, Pane::Songs); // the songs are next: arrows and Enter go there
            self.open(src, ctx)
        }
    }

    pub fn tracklist(&mut self, ui: &mut egui::Ui) {
        let (l, p, sk) = (skin::look(ui.ctx()), skin::pal(ui.ctx()), skin::skin(ui.ctx()));
        let mut s = self.state.lock().unwrap();
        if self.big {
            self.fetch_lyrics(&mut s);
            let out = focus(ui, &s);
            if let (Some(i), Some(e)) = (out.play, self.engine.get()) {
                s.queued = s.queued.saturating_sub(i - s.qi);
                s.dir = 1;
                s.play_at(&e.player(), i);
            }
            if s.since.is_some() { self.bars_shown.set(true) } // the meter and lyric lines move
            drop(s);
            if let Some(id) = out.like { self.toggle_like(id) }
            if out.back { self.big = false }
            if let Some(src) = out.goto { self.big = false; self.open(src, &ui.ctx().clone()) }
            return;
        }
        if s.view == Some(Source::Queue) {
            s.tracks = s.queue.iter().skip(s.qi).cloned().collect(); // the playing song, then what's next
        }
        let (title, img) = match &s.view {
            None => (String::new(), None),
            Some(Source::Liked) => ("Liked Songs".to_string(), None),
            Some(Source::NewReleases) => ("New releases".to_string(), None),
            Some(Source::Search(q)) => (q.clone(), None),
            Some(Source::Queue) => ("Queue".to_string(), None),
            Some(Source::Artist { name, .. }) => (name.clone(), s.artist.as_ref().and_then(|a| a.portrait.clone())),
            Some(Source::Album { name, .. }) => (name.clone(), s.tracks.first().and_then(|t| t.album.images.first()).map(|i| i.url.clone())),
            Some(Source::Radio { name, .. }) => (format!("{name} Radio"), s.tracks.first().and_then(|t| t.album.images.first()).map(|i| i.url.clone())),
            Some(Source::Playlist(id)) => s.playlists.iter().find(|pl| &pl.id == id)
                .map(|pl| (pl.name.clone(), pl.images.as_ref().and_then(|v| v.first()).map(|i| i.url.clone())))
                .unwrap_or_default(),
        };
        // What this list is called (window file, Japanese tag, header label), in each skin's voice.
        let (what, ext, jp) = match &s.view {
            None => ("", "exe", "接続中"),
            Some(Source::Search(_)) => ("search", "exe", "検索"),
            Some(Source::Queue) => ("queue", "lst", "キュー"),
            Some(Source::Artist { .. }) => ("artist", "art", "アーティスト"),
            Some(Source::Album { .. }) => ("album", "alb", "アルバム"),
            Some(Source::Radio { .. }) => ("radio", "rad", "ラジオ"),
            Some(Source::NewReleases) => ("new releases", "new", "新着"),
            Some(_) => ("playlist", "pls", "プレイリスト"),
        };
        let file = match &s.view {
            None => "connect.exe".to_string(),
            Some(Source::Search(_)) => "search.exe".to_string(),
            Some(_) => format!("{}.{ext}", slug(&title)),
        };
        let tag = jp;
        let results = what == "search";
        let (kind, unit) = match sk {
            Skin::Wired => (format!("{} // {jp}", what.to_uppercase()), if results { "RESULTS" } else { "TRACKS" }),
            _ => (what.to_uppercase(), if results { "RESULTS" } else { "TRACKS" }),
        };
        let kind = kind.as_str();
        let (mut clicked, mut like) = (self.enter.take(), None); // Enter plays like a click
        let (mut goto, mut play_all, mut point): (Option<Source>, Option<bool>, Option<usize>) = (None, None, None);
        let mut resort = false;
        // "Jump to the playing song": once its row is in the list, scroll there (and stop asking).
        let jump_row = self.jump.as_ref().and_then(|id| s.tracks.iter().position(|t| t.id.as_ref() == Some(id)));
        if jump_row.is_some() || !s.loading && s.view.is_some() && self.jump.is_some() && !s.tracks.is_empty() { self.jump = None }
        if let Some(row) = jump_row { (self.pane, self.row) = (Pane::Songs, row) }
        let (cursor, reveal, moused) = ((self.pane == Pane::Songs).then_some(self.row), self.reveal, moused(ui));
        let ctx = ui.ctx().clone();
        let me = self.engine.get().map(|e| e.session().username()).unwrap_or_default();
        let mut menu = SongMenu::default();

        skin::window(ui, Win::Main, &file, tag, None, |ui| {
            let pad = l.pad;
            egui::Frame::new().inner_margin(egui::Margin { left: pad, right: pad, top: pad, bottom: 8 }).show(ui, |ui| {
                let Some(view) = s.view.clone() else { return boot(ui, &s.status, s.error.as_deref()) };

                // Header.
                ui.horizontal(|ui| {
                    let initials = initials(&title);
                    let stamp = match view {
                        Source::Liked => Stamp::Liked,
                        Source::Search(_) => Stamp::Search,
                        Source::Queue => Stamp::Queue,
                        _ => Stamp::Text(&initials),
                    };
                    skin::cover(ui, img.as_deref(), l.header_cover, stamp, true);
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.add_space(match sk { Skin::Wired => 22.0, Skin::Swiss => 4.0, });
                        match sk {
                            Skin::Wired => { ui.label(RichText::new(kind).size(12.0).color(p.dim)); }
                            Skin::Swiss => { ui.label(RichText::new(kind).font(skin::num(13.0)).color(p.text)); }
                        }
                        let r = ui.add(egui::Label::new(RichText::new(&title).font(skin::title(l.title_size))).truncate());
                        skin::cursor_after(ui, r.rect, 40.0);
                        if sk == Skin::Swiss {
                            // The one red rule.
                            let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                            ui.painter().rect_filled(rule, 0.0, p.accent);
                        }
                        let n = s.tracks.len();
                        let count = match sk {
                            Skin::Wired => format!("{n:02} {unit}"),
                            Skin::Swiss => format!("{n} {unit}"),
                        };
                        match &s.error {
                            Some(e) => { ui.label(RichText::new(format!("{count} · ERR: {}", short_err(e))).font(small(sk)).color(p.accent)).on_hover_text(e); }
                            None if s.loading => {
                                ui.horizontal(|ui| {
                                    let of = s.expected.map_or("??".to_string(), |t| if sk.blocky() { format!("{t:02}") } else { t.to_string() });
                                    let done = if sk.blocky() { format!("{n:02}") } else { n.to_string() };
                                    ui.label(RichText::new(format!("{done}/{of} {unit}")).font(small(sk)).color(p.dim));
                                    ui.add_space(6.0);
                                    skin::progress(ui, 190.0, n, s.expected);
                                });
                            }
                            None => {
                                let total: u64 = s.tracks.iter().map(|t| t.duration_ms as u64).sum();
                                let length = if matches!(sk, Skin::Swiss) { total_length(total) } else { total_length(total).to_uppercase() };
                                let count = if n > 0 { format!("{count} · {length}") } else { count };
                                let count = match s.sort_label() {
                                    Some(l) if matches!(sk, Skin::Swiss) => format!("{count} · {l}"),
                                    Some(l) => format!("{count} · {}", l.to_uppercase()),
                                    None => count,
                                };
                                ui.label(RichText::new(count).font(small(sk)).color(p.dim));
                            }
                        }
                        if n > 0 && view != Source::Queue {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                let caps = !matches!(sk, Skin::Swiss);
                                if skin::action_button(ui, if caps { "▶ PLAY" } else { "Play" }, true).clicked() { play_all = Some(false) }
                                if skin::action_button(ui, if caps { "SHUFFLE" } else { "Shuffle" }, false).clicked() { play_all = Some(true) }
                                // Only where it does something: lists with dates (not search, albums, artists).
                                if s.tracks.iter().any(|t| t.added > 0) {
                                    let order = s.sort.label().unwrap_or("list order");
                                    let text = if caps { format!("SORT: {}", order.to_uppercase()) } else { format!("Sort: {order}") };
                                    if skin::action_button(ui, &text, false).on_hover_text("List order → newest added → oldest added (⌘O)").clicked() { resort = true }
                                }
                            });
                        }
                    });
                });
                ui.add_space(14.0);

                let artist_page = s.artist.is_some();
                // Everything under the header; on artist pages it all scrolls together.
                let mut rest = |ui: &mut egui::Ui| {
                    // Artist pages: their albums / EPs / singles as a strip of covers, then the top songs.
                    if let Some(a) = s.artist.as_ref().filter(|a| !a.releases.is_empty()) {
                        let wired = sk == Skin::Wired;
                        ui.label(RichText::new(if wired { "ALBUMS & EPS // ディスコグラフィ" } else { "ALBUMS & EPS" }).font(small(sk)).color(p.dim));
                        egui::ScrollArea::horizontal().id_salt("releases").show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 16.0;
                                for r in &a.releases {
                                    let size = 132.0;
                                    let (rect, resp) = ui.allocate_exact_size(vec2(size, size + 46.0), Sense::click());
                                    let art = Rect::from_min_size(rect.min, Vec2::splat(size));
                                    let initials = initials(&r.name);
                                    skin::cover(&mut ui.new_child(egui::UiBuilder::new().max_rect(art)), r.cover.as_deref(), size, Stamp::Text(&initials), false);
                                    if resp.hovered() { ui.painter().rect_stroke(art, 0.0, Stroke::new(2.0_f32, p.accent), StrokeKind::Outside); }
                                    let line = |text: String, font: FontId, color| {
                                        let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
                                        job.wrap = egui::text::TextWrapping::truncate_at_width(size);
                                        ui.painter().layout_job(job)
                                    };
                                    let name = line(r.name.clone(), FontId::proportional(13.0), if resp.hovered() { p.accent } else { p.text });
                                    let meta = line(format!("{} · {}", r.year, r.kind), skin::num(if wired { 16.0 } else { 11.0 }), p.dim);
                                    let y = art.bottom() + 6.0;
                                    let below = y + name.size().y + 2.0;
                                    ui.painter().galley(pos2(art.left(), y), name, p.text);
                                    ui.painter().galley(pos2(art.left(), below), meta, p.dim);
                                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&r.name).clicked() {
                                        goto = Some(Source::Album { id: r.id.clone(), name: r.name.clone() });
                                    }
                                }
                            });
                        });
                        ui.add_space(12.0);
                        ui.label(RichText::new(if wired { "TOP SONGS // 人気" } else { "TOP SONGS" }).font(small(sk)).color(p.dim));
                    }

                    // Columns, as offsets from the row's left edge. Swiss gives the artist its own column;
                    // Wired stacks it under the title.
                    // Leave the scrollbar its lane, or the rows push the window wider than its panel.
                    let w = ui.available_width() - ui.spacing().scroll.allocated_width();
                    let one_line = sk == Skin::Swiss;
                    let [c_num, c_title, c_artist, c_album, c_dur] = if one_line {
                        [10.0, 62.0, w * 0.42, w * 0.66, w - 10.0]
                    } else {
                        [10.0, 70.0, 0.0, w * 0.58, w - 10.0]
                    };
                    let (rect, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                    let y = rect.center().y;
                    let head_font = match sk {
                        Skin::Wired => skin::num(17.0),
                        Skin::Swiss => FontId::proportional(11.0),
                    };
                    let labels: &[(f32, &str)] = match sk {
                        Skin::Wired => &[(c_num, "NO."), (c_title, "TITLE"), (c_album, "ALBUM")],
                        _ => &[(c_num, "NO"), (c_title, "TITLE"), (c_artist, "ARTIST"), (c_album, "ALBUM")],
                    };
                    let painter = ui.painter();
                    for (x, label) in labels {
                        painter.text(pos2(rect.left() + x, y), egui::Align2::LEFT_CENTER, *label, head_font.clone(), p.dim);
                    }
                    painter.text(pos2(rect.left() + c_dur, y), egui::Align2::RIGHT_CENTER, "TIME", head_font, p.dim);
                    match sk {
                        Skin::Wired => painter.extend(egui::Shape::dashed_line(&[rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, p.line), 4.0, 3.0)),
                        Skin::Swiss => { painter.hline(rect.x_range(), rect.bottom(), Stroke::new(2.0_f32, p.text)); }
                    }
                    ui.add_space(-6.0);

                    // Rows: only the visible ones are laid out, so long lists stay fast.
                    let now_id = s.now.as_ref().and_then(|t| t.id.clone());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // While loading, placeholder rows trail the list with a highlight scanning down them.
                    let ghosts = match (s.loading, s.tracks.is_empty()) { (false, _) => 0, (true, true) => 8, (true, false) => 3 };
                    let scan = (ui.input(|i| i.time) * 8.0) as usize % ghosts.max(1);
                    let num_font = match sk {
                        Skin::Wired => skin::num(22.0),
                        Skin::Swiss => skin::title(22.0),
                    };
                    let time_font = match sk {
                        Skin::Wired => skin::num(20.0),
                        Skin::Swiss => FontId::proportional(13.0),
                    };
                    let cell_size = match sk { Skin::Swiss => 15.0, _ => 13.0 };
                    let mut area = egui::ScrollArea::vertical().auto_shrink(false);
                    if let Some(row) = jump_row {
                        let pitch = l.row_h + ui.spacing().item_spacing.y;
                        area = area.vertical_scroll_offset((row as f32 * pitch - 3.0 * pitch).max(0.0));
                    }
                    let total = s.tracks.len() + ghosts;
                    let mut draw = |ui: &mut egui::Ui, range: std::ops::Range<usize>| {
                        if let (true, Some(row)) = (reveal, cursor) {
                            // The cursor's row may be outside the rows laid out: place it from the first of them.
                            let top = ui.cursor().top() + (row as f32 - range.start as f32) * l.row_h;
                            ui.scroll_to_rect(Rect::from_min_size(pos2(ui.cursor().left(), top), vec2(w, l.row_h)), None);
                        }
                        for i in range {
                            let Some(t) = s.tracks.get(i) else {
                                skin::ghost_row(ui, w, [c_num, c_title, if one_line { c_artist } else { c_album }, c_dur], i - s.tracks.len() == scan);
                                continue;
                            };
                            let (rect, r) = ui.allocate_exact_size(vec2(w, l.row_h), Sense::click());
                            let current = t.id == now_id;
                            let on = cursor == Some(i);
                            if moused && r.hovered() { point = Some(i) }
                            let (ink, sub) = skin::row_bg(ui, rect, current, on);
                            // Songs Spotify refused this session fade out.
                            let (ink, sub) = if t.id.as_ref().is_some_and(|id| s.unplayable.contains(id)) { (p.dim.gamma_multiply(0.5), p.dim.gamma_multiply(0.4)) } else { (ink, sub) };
                            let (x, y) = (rect.left(), rect.center().y);
                            if current {
                                skin::now_mark(ui.painter(), pos2(x + c_num, y), ink, sk, s.since.is_some(), ui.input(|i| i.time));
                                self.bars_shown.set(true);
                            } else {
                                let n = if sk.blocky() || sk == Skin::Swiss { format!("{:02}", i + 1) } else { (i + 1).to_string() };
                                ui.painter().text(pos2(x + c_num, y), egui::Align2::LEFT_CENTER, n, num_font.clone(), p.dim);
                            }
                            // One left-aligned, truncated label per cell; returns where its text landed.
                            let cell = |ui: &mut egui::Ui, from: f32, to: f32, text: String, size: f32, color| -> Rect {
                                let cell = Rect::from_min_max(pos2(x + from, rect.top()), pos2(x + to, rect.bottom()));
                                ui.put(cell, |ui: &mut egui::Ui| ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).truncate())
                                }).inner).rect
                            };
                            // Artist and album names lead to their pages: underline on hover, click to open.
                            let mut link = |ui: &mut egui::Ui, at: Rect, key: &str, to: Option<Source>, color| {
                                if let Some(to) = to && link(ui, at, egui::Id::new((key, i)), color).clicked() { goto = Some(to) }
                            };
                            let (artist_page, album_page) = (artist_of(t), album_of(t));
                            if one_line {
                                let name = t.name.clone();
                                cell(ui, c_title, c_artist - 12.0, name, cell_size, ink);
                                let at = cell(ui, c_artist, c_album - 12.0, t.artist_line(), cell_size, sub);
                                link(ui, at, "artist", artist_page.clone(), sub);
                            } else {
                                // Title over artist, measured and centred in the row so neither touches its edges.
                                let big = 14.0;
                                let width = (c_album - 16.0 - c_title).max(0.0);
                                let line = |text: String, size: f32, color| {
                                    let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
                                    ui.painter().layout_job(job)
                                };
                                let (name, artist) = (line(t.name.clone(), big, ink), line(t.artist_line(), big - 2.5, sub));
                                let top = y - (name.size().y + 1.0 + artist.size().y) / 2.0;
                                let below = top + name.size().y + 1.0;
                                ui.painter().galley(pos2(x + c_title, top), name, ink);
                                let at = Rect::from_min_size(pos2(x + c_title, below), artist.size());
                                ui.painter().galley(pos2(x + c_title, below), artist, sub);
                                link(ui, at, "artist", artist_page.clone(), sub);
                            }
                            let at = cell(ui, c_album, c_dur - 70.0, t.album.name.clone(), if one_line { cell_size } else { 12.0 }, sub);
                            link(ui, at, "album", album_page, sub);
                            ui.painter().text(pos2(x + c_dur, y), egui::Align2::RIGHT_CENTER, mmss(t.duration_ms, sk.blocky()), time_font.clone(), sub);
                            if let Some(id) = &t.id {
                                let heart = Rect::from_center_size(pos2(x + c_dur - 56.0, y), vec2(26.0, 26.0));
                                if skin::heart_toggle(ui, heart, egui::Id::new(("like", i)), s.liked.contains(id), on, current).clicked() {
                                    like = Some(id.clone());
                                }
                            }

                            r.context_menu(|ui| song_menu(ui, t, &s, &me, true, &mut menu));
                            if std::mem::take(&mut menu.play) { clicked = Some(i) }
                            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { clicked = Some(i) }
                        }
                    };
                    // Artist pages scroll as a whole (strip + a short list); long lists only lay out visible rows.
                    if artist_page { draw(ui, 0..total) } else { area.show_rows(ui, l.row_h, total, |ui, range| draw(ui, range)); }
                };
                if artist_page {
                    egui::ScrollArea::vertical().id_salt("artist-page").auto_shrink(false).show(ui, rest);
                } else {
                    rest(ui);
                }
            });
        });

        // Enter can name a row the list no longer has (it emptied to load, the queue moved on).
        if let (Some(i), Some(e)) = (clicked.filter(|i| *i < s.tracks.len()), self.engine.get()) {
            if let Some(id) = s.tracks.get(i).and_then(|t| t.id.clone()) {
                forget_unplayable(&mut s.unplayable, &id); // a click gives a faded song another try
            }
            if s.view == Some(Source::Queue) {
                // Picking from the queue jumps ahead in it, using up hand-queued songs on the way.
                let at = s.qi + i;
                s.queued = s.queued.saturating_sub(i);
                s.dir = 1;
                s.play_at(&e.player(), at);
            } else {
                let (list, src) = (s.tracks.clone(), s.view.clone());
                s.start(&e.player(), list, Some(i), src);
            }
        }
        if resort { s.cycle_sort() }
        if let (Some(shuffled), Some(e)) = (play_all, self.engine.get()) {
            if shuffled { s.shuffle = true }
            let (list, src) = (s.tracks.clone(), s.view.clone());
            s.start(&e.player(), list, None, src);
        }
        drop(s); // toggle_like locks the state itself
        if let Some(i) = point.filter(|i| cursor != Some(*i)) {
            (self.pane, self.row) = (Pane::Songs, i);
            ctx.request_repaint(); // draw the highlight where it is now
        }
        if let Some(id) = like { self.toggle_like(id) }
        if let Some(src) = goto { self.open(src, &ctx) }
        self.apply_menu(menu, &ctx);
    }

    /// Does what a song menu asked for.
    fn apply_menu(&mut self, menu: SongMenu, ctx: &egui::Context) {
        if let (Some((t, next)), Some(e)) = (menu.enqueue, self.engine.get()) {
            self.state.lock().unwrap().enqueue(&e.player(), t, next);
        }
        if let Some(id) = menu.like { self.toggle_like(id) }
        if let Some((playlist, name, track)) = menu.add { self.add_to(playlist, name, track, ctx) }
        if let Some((name, track)) = menu.create { self.new_playlist(name, Some(track), ctx) }
        if menu.members { self.load_members() }
        if let Some(src) = menu.radio { self.big = false; self.open(src, ctx) }
    }

    pub fn player_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (p, sk) = (skin::pal(ctx), skin::skin(ctx));
        let engine = self.engine.clone();
        let playing = self.state.lock().unwrap().since.is_some();
        let (tag, dot) = match (engine.get().is_some(), playing) {
            (false, _) => ("OFFLINE", p.dim),
            (true, true) => ("ONLINE · 再生中", p.online),
            (true, false) => ("ONLINE", p.online),
        };
        skin::window(ui, Win::Player, "player.exe", tag, Some(dot), |ui| {
            let margin = match sk {
                Skin::Wired => egui::Margin::symmetric(14, 10),
                Skin::Swiss => egui::Margin { left: 30, right: 30, top: 14, bottom: 10 },
            };
            egui::Frame::new().inner_margin(margin).show(ui, |ui| match engine.get() {
                Some(_) => self.transport(ui, ctx),
                None => { ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| switches(ui, ctx)); } // restyle while signing in
            });
        });
    }

    pub fn transport(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (p, sk) = (skin::pal(ctx), skin::skin(ctx));
        let s = self.state.lock().unwrap();
        let (pos, playing) = (s.position(), s.since.is_some());
        let dur = s.now.as_ref().map_or(0, |t| t.duration_ms);
        let full = ui.available_rect_before_wrap();
        let side = (full.width() * 0.3).min(360.0);
        let pad = sk.blocky();
        let (shuffle, repeat) = (s.shuffle, s.repeat);

        let (mut like_now, mut open_big, mut jump, mut goto) = (None, false, false, None);
        let (me, mut menu) = (self.engine.get().map(|e| e.session().username()).unwrap_or_default(), SongMenu::default());
        let mut cmds: Vec<Cmd> = vec![];

        // Left: now playing. The cover opens the focus view, the name jumps to the song in its list.
        let left = Rect::from_min_size(full.min, vec2(side, full.height()));
        ui.put(left, |ui: &mut egui::Ui| ui.horizontal_centered(|ui| {
            if let Some(t) = &s.now {
                let size = match sk { Skin::Wired => 52.0, Skin::Swiss => 44.0, };
                let art = skin::cover(ui, t.thumb(), size, Stamp::None, false);
                let r = ui.interact(art, egui::Id::new("now-cover"), Sense::click());
                r.context_menu(|ui| song_menu(ui, t, &s, &me, false, &mut menu));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Cover and lyrics (⌘E)").clicked() {
                    open_big = true;
                }
                // Name over artist, measured and centred on the cover (a nested vertical would sit at the top).
                let width = (left.right() - art.right() - 60.0).max(0.0); // leaves room for the heart
                let line = |text: String, size: f32, color| {
                    let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
                    ui.painter().layout_job(job)
                };
                let name = line(t.name.clone(), 14.0, p.text);
                let artist = line(t.artist_line(), 11.5, p.dim);
                let top = art.center().y - (name.size().y + 2.0 + artist.size().y) / 2.0;
                let x = art.right() + 12.0;
                let below = top + name.size().y + 2.0;
                let text_w = name.size().x.max(artist.size().x);
                // The name jumps to the song in its list, the artist opens the artist's page.
                let name_rect = Rect::from_min_size(pos2(x, top), name.size());
                let artist_rect = Rect::from_min_size(pos2(x, below), artist.size());
                ui.painter().galley(pos2(x, top), name, p.text);
                ui.painter().galley(pos2(x, below), artist, p.dim);
                let r = ui.interact(name_rect, egui::Id::new("now-text"), Sense::click());
                r.context_menu(|ui| song_menu(ui, t, &s, &me, false, &mut menu));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Show in its list (⌘J)").clicked() {
                    jump = true;
                }
                let r = link(ui, artist_rect, egui::Id::new("now-artist"), p.dim);
                r.context_menu(|ui| song_menu(ui, t, &s, &me, false, &mut menu));
                if r.clicked() { goto = artist_of(t) }
                if let Some(id) = &t.id {
                    let heart = Rect::from_center_size(pos2(x + text_w + 22.0, art.center().y), vec2(28.0, 28.0));
                    if skin::heart_toggle(ui, heart, egui::Id::new("like-now"), s.liked.contains(id), true, false).on_hover_text("⌘L").clicked() {
                        like_now = Some(id.clone());
                    }
                }
            }
        }).response);

        // Center: shuffle, transport, repeat; seek below.
        let mid_w = (full.width() - 2.0 * side).max(240.0);
        let mid = Rect::from_center_size(full.center(), vec2(mid_w, full.height()));
        let (mut toggle_shuffle, mut cycle_repeat) = (false, false);
        ui.put(mid, |ui: &mut egui::Ui| ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            // Fixed row height, so buttons of different sizes all centre on the same line.
            let (small_b, main_b) = (skin::ctl_size(sk, false), skin::ctl_size(sk, true));
            let gap = match sk { Skin::Swiss => 18.0, _ => 8.0 };
            let widths = 2.0 * small_b.x + main_b.x + 2.0 * 32.0;
            let height = small_b.y.max(main_b.y) + 0.0;
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_space((ui.available_width() - widths - 4.0 * gap) / 2.0);
                toggle_shuffle = skin::icon_button(ui, skin::Glyph::Shuffle, shuffle).on_hover_text("Shuffle (S)").clicked();
                if skin::ctl_button(ui, Ctl::Prev).on_hover_text("Previous (⌘←)").clicked() { cmds.push(Cmd::Prev) }
                if skin::ctl_button(ui, if playing { Ctl::Pause } else { Ctl::Play }).on_hover_text("Play / pause (space)").clicked() { cmds.push(Cmd::Toggle) }
                if skin::ctl_button(ui, Ctl::Next).on_hover_text("Next (⌘→)").clicked() { cmds.push(Cmd::Next) }
                let glyph = if repeat == Repeat::One { skin::Glyph::RepeatOne } else { skin::Glyph::Repeat };
                let hint = match repeat { Repeat::Off => "Repeat: off (R)", Repeat::All => "Repeat: all (R)", Repeat::One => "Repeat: this song (R)" };
                cycle_repeat = skin::icon_button(ui, glyph, repeat != Repeat::Off).on_hover_text(hint).clicked();
            });
            ui.horizontal(|ui| {
                let shown = self.scrub.unwrap_or(pos as f32);
                let time_font = small(sk).size.min(18.0);
                let font = FontId::new(time_font, small(sk).family);
                let left_t = mmss(shown as u32, pad);
                let right_t = mmss(dur, pad);
                ui.add_sized(vec2(40.0, 16.0), egui::Label::new(RichText::new(left_t).font(font.clone()).color(p.dim)));
                let w = ui.available_width() - 48.0;
                let (r, at) = skin::slider(ui, w, shown / dur.max(1) as f32, false);
                let at = at.or(r.hover_pos().map(|q| ((q.x - r.rect.left()) / r.rect.width()).clamp(0.0, 1.0))).map(|f| f * dur as f32);
                if r.dragged() { self.scrub = at }
                if r.drag_stopped() {
                    if let Some(v) = self.scrub.take() { cmds.push(Cmd::SeekTo(v as u32)) }
                } else if r.clicked() && let Some(v) = at {
                    cmds.push(Cmd::SeekTo(v as u32));
                }
                ui.add_sized(vec2(40.0, 16.0), egui::Label::new(RichText::new(right_t).font(font).color(p.dim)));
            });
        }).response);

        // Right: volume, then the focus-view and settings buttons.
        let right = Rect::from_min_size(pos2(full.right() - side, full.top()), vec2(side, full.height()));
        let mut new_volume = None;
        ui.put(right, |ui: &mut egui::Ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let width = match sk { Skin::Wired => 78.0, _ => 90.0 };
            let (r, at) = skin::slider(ui, width, self.volume, true);
            if let Some(v) = at { new_volume = Some(v) }
            if r.drag_stopped() || r.clicked() { set_setting("volume", &format!("{:.2}", at.unwrap_or(self.volume))) }
            match sk {
                Skin::Wired => { ui.label(RichText::new("VOL").font(small(sk)).color(p.dim)); }
                _ => {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    let c = r.center();
                    ui.painter().rect_filled(Rect::from_center_size(c - vec2(3.5, 0.0), vec2(4.0, 6.0)), 1.0, p.dim);
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-2.0, -3.0), c + vec2(3.0, -7.0), c + vec2(3.0, 7.0), c + vec2(-2.0, 3.0)], p.dim, Stroke::NONE));
                }
            }
            ui.add_space(18.0);
            if skin::icon_button(ui, skin::Glyph::Note, self.big).on_hover_text("Cover and lyrics (⌘E, Esc goes back)").clicked() {
                self.big = !self.big;
            }
            ui.add_space(6.0);
            switches(ui, ctx);
        }).response);
        drop(s);

        if let Some(v) = new_volume { self.set_volume(v) }
        if toggle_shuffle { self.toggle_shuffle() }
        if cycle_repeat { self.cycle_repeat() }
        for c in cmds { self.command(c) }
        if let Some(id) = like_now { self.toggle_like(id) }
        if open_big { self.big = true }
        if jump { self.jump_to_playing(ctx) }
        if let Some(src) = goto { self.big = false; self.open(src, ctx) }
        self.apply_menu(menu, ctx);
    }
}

#[test]
fn sidebar_lists_the_queue_only_while_it_shows() {
    let mut s = State::default();
    assert!(side_lists(&s) == [Source::Liked, Source::NewReleases]);
    s.queued = 1;
    assert!(side_lists(&s) == [Source::Liked, Source::NewReleases, Source::Queue]);
}

#[test]
pub fn parses_ids() {
    let id = "4uLU6hMCjMI75M1A2tKUQC";
    assert_eq!(track_id(id), id);
    assert_eq!(track_id(&format!("spotify:track:{id}")), id);
    assert_eq!(track_id(&format!(" https://open.spotify.com/track/{id}?si=abc\n")), id);
    assert_eq!(mmss(185_000, true), "03:05");
    assert_eq!(mmss(185_000, false), "3:05");
    assert_eq!(slug("Late Night Piano!"), "late_night_piano_");
    assert_eq!(initials("late night piano"), "LN");
    assert_eq!(initials(""), "");
    assert_eq!(short_err("token: Service unavailable { error handling Mercury response: MercuryResponse { uri: \"hm://x\" } }"), "token: Service unavailable");
    assert_eq!(short_err("Connect failed: timed out"), "Connect failed: timed out");
    assert_eq!(short_err(&"x".repeat(200)).chars().count(), 91);
}

