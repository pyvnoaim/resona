//! librespot: sign-in, the session and player, and the background loaders that fill the library.

use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use eframe::egui;
use librespot::{
    core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session},
    playback::{
        audio_backend,
        config::{AudioFormat, PlayerConfig},
        mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
        player::{Player, PlayerEvent},
    },
};
use librespot_oauth::OAuthClientBuilder;

use crate::{api, settings::*, state::*};

// ponytail: playback borrows librespot's public client ID (same as the official desktop app). The Web API
// uses your own app (WEB_CLIENT_ID in api.rs) because this shared ID gets day-long rate-limit bans there.
pub const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

pub const REDIRECT: &str = "http://127.0.0.1:8898/login";

pub struct Engine {
    pub player: Arc<Player>,
    pub session: Session,
    pub web: Arc<api::WebAuth>,
    pub mixer: Arc<SoftMixer>,
    pub http: reqwest::Client,
}

pub type Shared = Arc<Mutex<State>>;

pub async fn connect(state: Shared, slot: Arc<OnceLock<Engine>>, ctx: egui::Context) -> Result<(), String> {
    let set = |s: &str| { state.lock().unwrap().status = s.into(); ctx.request_repaint(); };
    let dir = std::env::home_dir().unwrap().join(".resona");
    // Saved reusable credentials, so the browser login only happens once.
    let cache = Cache::new(Some(&dir), None, None, None).map_err(|e| e.to_string())?;
    let creds = match cache.credentials() {
        Some(c) => c,
        None => {
            set("Sign in in your browser…");
            let token = OAuthClientBuilder::new(CLIENT_ID, REDIRECT, vec!["streaming"])
                .open_in_browser().build().map_err(|e| e.to_string())?
                .get_access_token_async().await.map_err(|e| e.to_string())?;
            Credentials::with_access_token(token.access_token)
        }
    };
    set("Connecting…");
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await.map_err(|e| format!("Connect failed: {e}"))?;

    let mixer = Arc::new(SoftMixer::open(MixerConfig::default()).map_err(|e| e.to_string())?);
    mixer.set_volume((0.7 * u16::MAX as f32) as u16);
    let backend = audio_backend::find(None).unwrap();
    let player = Player::new(PlayerConfig::default(), session.clone(), mixer.get_soft_volume(), move || backend(None, AudioFormat::default()));
    let mut events = player.get_player_event_channel();
    let web = Arc::new(api::WebAuth::new(&dir, REDIRECT)?);
    let engine = Engine { player: player.clone(), session, web, mixer, http: reqwest::Client::new() };

    // Sidebar: every playlist, page by page.
    set("Loading your library…");
    let mut url = Some("me/playlists?limit=50".to_string());
    while let Some(u) = url {
        // A failed load (a rate-limit ban, say) is shown but not fatal: playback works without the Web API.
        let page = match api::get::<api::Page<api::Playlist>>(&engine.web, &engine.http, &u).await {
            Ok(page) => page,
            Err(e) => { state.lock().unwrap().error = Some(e); break }
        };
        state.lock().unwrap().playlists.extend(page.items);
        ctx.request_repaint();
        url = page.next;
    }
    tokio::spawn(load_liked(state.clone(), engine.web.clone(), engine.http.clone(), ctx.clone()));
    let _ = slot.set(engine);
    set("");

    while let Some(ev) = events.recv().await {
        let mut s = state.lock().unwrap();
        match ev {
            PlayerEvent::Playing { position_ms, .. } => (s.pos_ms, s.since) = (position_ms, Some(Instant::now())),
            PlayerEvent::Paused { position_ms, .. } => (s.pos_ms, s.since) = (position_ms, None),
            PlayerEvent::Seeked { position_ms, .. } | PlayerEvent::PositionCorrection { position_ms, .. } => {
                s.pos_ms = position_ms;
                s.since = s.since.map(|_| Instant::now());
            }
            PlayerEvent::EndOfTrack { .. } => {
                let from = s.qi;
                let again = s.repeat == Repeat::One && s.play_at(&player, from);
                if !again && !s.step(&player, from, 1) { (s.pos_ms, s.since) = (0, None) }
            }
            PlayerEvent::Unavailable { track_id, .. } => {
                // librespot also reports failed *preloads* of other songs here while this one plays on, so
                // only a failure of the song we're on counts.
                let failed = track_id.to_id().ok();
                if failed.is_none() || failed != s.now.as_ref().and_then(|t| t.id.clone()) { continue }
                // Spotify won't play this one here (region, licensing). Remember it, say so, and keep going
                // the way we were going, so "back" doesn't bounce off it forever.
                let (from, dir) = (s.qi, if s.dir == 0 { 1 } else { s.dir });
                if let Some(t) = s.now.clone() {
                    s.notice = Some(format!("Spotify can't play \u{201c}{}\u{201d} here, skipped", t.name));
                    if let Some(id) = t.id {
                        remember_unplayable(&id);
                        s.unplayable.insert(id);
                    }
                }
                if !s.step(&player, from, dir) { (s.pos_ms, s.since) = (0, None) }
            }
            _ => continue,
        }
        ctx.request_repaint();
    }
    Ok(())
}

/// Every liked track id, page by page, so lists can show hearts. Developer apps get 403 from Spotify's
/// "which of these are liked?" lookup, so we read the whole Liked Songs list once instead.
pub async fn load_liked(state: Shared, web: Arc<api::WebAuth>, http: reqwest::Client, ctx: egui::Context) {
    let mut url = Some("me/tracks?limit=50".to_string());
    while let Some(u) = url {
        // ponytail: on failure the hearts just stay empty; liking still works.
        let Ok(page) = api::get::<api::Page<api::Entry>>(&web, &http, &u).await else { return };
        state.lock().unwrap().liked.extend(page.items.into_iter().filter_map(api::Entry::playable).filter_map(|t| t.id));
        ctx.request_repaint();
        url = page.next;
    }
}

/// What's already in each of your own playlists, so the menu can tick them. Ids only, to keep it light.
/// Loaded the first time the menu opens: reading every playlist at startup ran into Spotify's rate limits.
pub async fn load_members(state: Shared, web: Arc<api::WebAuth>, http: reqwest::Client, me: String) {
    let own: Vec<String> = state.lock().unwrap().playlists.iter().filter(|p| p.web_readable(&me)).map(|p| p.id.clone()).collect();
    for id in own {
        let mut url = Some(format!("playlists/{id}/items?limit=100&fields=next,items(track(id,type),item(id,type))"));
        while let Some(u) = url {
            // ponytail: a playlist that fails just shows no ticks; a rate limit stops the lot, until restart.
            let page = match api::get::<api::Page<api::Entry>>(&web, &http, &u).await {
                Ok(page) => page,
                Err(e) if e.contains("rate limit") => return,
                Err(_) => break,
            };
            let ids = page.items.into_iter().filter_map(api::Entry::playable).filter_map(|t| t.id);
            state.lock().unwrap().members.entry(id.clone()).or_default().extend(ids);
            url = page.next;
        }
    }
}

pub fn set_like(state: &Shared, id: &str, on: bool) {
    let mut s = state.lock().unwrap();
    if on { s.liked.insert(id.to_string()); } else { s.liked.remove(id); }
}
