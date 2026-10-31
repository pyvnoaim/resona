//! librespot: sign-in, the session and player, and the background loaders that fill the library.

use std::{
    sync::{Arc, Mutex, OnceLock, RwLock},
    time::Instant,
};

use eframe::egui;
use librespot::{
    core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session},
    playback::{
        audio_backend,
        config::{AudioFormat, PlayerConfig},
        mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
        player::{Player, PlayerEvent, PlayerEventChannel},
    },
};
use librespot_oauth::OAuthClientBuilder;

use crate::{api, settings::*, state::*};

// ponytail: playback borrows librespot's public client ID (same as the official desktop app). The Web API
// uses your own app (`api::client_id`) because this shared ID gets day-long rate-limit bans there.
pub const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

pub const REDIRECT: &str = "http://127.0.0.1:8898/login";

pub struct Engine {
    // Swapped by `reconnect`, so always read through `player()` / `session()`.
    player: RwLock<Arc<Player>>,
    session: RwLock<Session>,
    pub web: Arc<api::WebAuth>,
    pub mixer: Arc<SoftMixer>,
    pub http: reqwest::Client,
}

pub type Shared = Arc<Mutex<State>>;

impl Engine {
    pub fn player(&self) -> Arc<Player> { self.player.read().unwrap().clone() }
    pub fn session(&self) -> Session { self.session.read().unwrap().clone() }

    /// librespot never reconnects a session that lost its connection (a network drop, a missed keepalive, sleep):
    /// every load after that fails. A fresh session and player take over; the caller listens to the new events.
    async fn reconnect(&self) -> Result<PlayerEventChannel, String> {
        let cache = cache()?;
        let creds = cache.credentials().ok_or("No saved sign-in")?;
        let (session, player) = start(cache, creds, &self.mixer).await?;
        let events = player.get_player_event_channel();
        *self.session.write().unwrap() = session;
        *self.player.write().unwrap() = player; // the old one stops when its last clone drops
        Ok(events)
    }
}

/// The saved sign-in, and the songs played lately (~/.resona/audio, up to 1 GB, oldest dropped first), so a
/// replay or a step back starts at once without downloading the song again.
fn cache() -> Result<Cache, String> {
    let dir = crate::settings::resona_dir();
    Cache::new(Some(dir.clone()), None, Some(dir.join("audio")), Some(1 << 30)).map_err(|e| e.to_string())
}

/// A signed-in session and a player on it.
async fn start(cache: Cache, creds: Credentials, mixer: &SoftMixer) -> Result<(Session, Arc<Player>), String> {
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await.map_err(|e| format!("Connect failed: {e}"))?;
    let backend = audio_backend::find(None).unwrap();
    let player = Player::new(PlayerConfig::default(), session.clone(), mixer.get_soft_volume(), move || backend(None, AudioFormat::default()));
    Ok((session, player))
}

pub async fn connect(state: Shared, slot: Arc<OnceLock<Engine>>, ctx: egui::Context) -> Result<(), String> {
    let set = |s: &str| { state.lock().unwrap().status = s.into(); ctx.request_repaint(); };
    let dir = crate::settings::resona_dir();
    // Saved reusable credentials, so the browser login only happens once.
    let cache = cache()?;
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
    let mixer = Arc::new(SoftMixer::open(MixerConfig::default()).map_err(|e| e.to_string())?);
    mixer.set_volume((0.7 * u16::MAX as f32) as u16);
    let (session, player) = start(cache, creds, &mixer).await?;
    let mut events = player.get_player_event_channel();
    let web = Arc::new(api::WebAuth::new(&dir, REDIRECT)?);
    let engine = Engine { player: RwLock::new(player), session: RwLock::new(session), web, mixer, http: reqwest::Client::new() };

    // Sidebar: every playlist, page by page.
    set("Loading your library…");
    let mut url = Some("me/playlists?limit=50".to_string());
    while let Some(u) = url {
        // A failed load (a rate-limit ban, say) is shown but not fatal: playback works without the Web API.
        let page = match api::get::<api::Page<api::Playlist>>(&engine.web, &engine.http, &u).await {
            Ok(page) => page,
            Err(e) => {
                // The same list through the session, which bans don't touch; the error still says why covers are missing.
                let e = match api::session_playlists(&engine.session()).await {
                    Ok(all) => {
                        // The rootlist only has covers you uploaded; Spotify's mosaics are drawn from the songs.
                        let bare: Vec<String> = all.iter().filter(|p| p.images.is_none()).map(|p| p.id.clone()).collect();
                        state.lock().unwrap().playlists = all;
                        let (session, state, ctx) = (engine.session(), state.clone(), ctx.clone());
                        tokio::spawn(async move {
                            for id in bare {
                                let Some(url) = api::session_mosaic(&session, &id).await else { continue };
                                if let Some(p) = state.lock().unwrap().playlists.iter_mut().find(|p| p.id == id) {
                                    p.images = Some(vec![api::Image { url }]);
                                }
                                ctx.request_repaint();
                            }
                        });
                        e
                    }
                    Err(also) => format!("{e}; {also}"),
                };
                state.lock().unwrap().error = Some(e);
                break;
            }
        };
        state.lock().unwrap().playlists.extend(page.items);
        ctx.request_repaint();
        url = page.next;
    }
    // Hearts: ~/.resona/liked at once, then the whole set fresh through the session (one request).
    if let Some(ids) = load_liked_ids() { let mut s = state.lock().unwrap(); (s.liked, s.liked_complete) = (ids, true) }
    tokio::spawn(load_liked(state.clone(), engine.session(), ctx.clone()));
    let _ = slot.set(engine);
    let e = slot.get().unwrap();
    set("");

    let mut retried: Option<String> = None; // the song that already got its second try
    while let Some(ev) = events.recv().await {
        // Any failed load comes as Unavailable (a lost audio key, a CDN hiccup), so for the song we're on, ask
        // Spotify's metadata whether it really refuses it. Asked before the lock, so the UI never waits on it.
        let refused = match &ev {
            PlayerEvent::Unavailable { track_id, .. } => {
                let id = track_id.to_id().ok();
                let ours = id.is_some() && { let s = state.lock().unwrap(); !s.loading && s.now.as_ref().and_then(|t| t.id.clone()) == id };
                // A dead session fails every song: reconnect and play this one again, instead of skipping them all.
                if ours && e.session().is_invalid() {
                    let back = e.reconnect().await;
                    let mut s = state.lock().unwrap();
                    match back {
                        Ok(new) => { events = new; let from = s.qi; s.play_at(&e.player(), from); }
                        Err(err) => { (s.pos_ms, s.since) = (0, None); s.error = Some(format!("Lost the connection to Spotify: {err}")) }
                    }
                    ctx.request_repaint();
                    continue;
                }
                match id { Some(id) if ours => api::refused(&e.session(), &id).await, _ => false }
            }
            _ => false,
        };
        let player = e.player();
        let mut s = state.lock().unwrap();
        match ev {
            PlayerEvent::Playing { position_ms, .. } => {
                (s.pos_ms, s.since) = (position_ms, Some(Instant::now()));
                let _ = crate::settings::save_resume(&s);
            }
            PlayerEvent::Paused { position_ms, .. } => {
                (s.pos_ms, s.since) = (position_ms, None);
                let _ = crate::settings::save_resume(&s);
            }
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
                // A list loading through the session starves librespot's request limiter, and the player then
                // reports playable songs as Unavailable: stop instead of marking (and skipping) the whole list.
                // ponytail: only covers list loads; a streak guard if other session bursts starve it too.
                if s.loading {
                    (s.pos_ms, s.since) = (0, None);
                    s.error = Some("Spotify is busy while this list loads; play again in a moment".into());
                    ctx.request_repaint();
                    continue;
                }
                let Some(id) = failed else { continue };
                let (from, dir) = (s.qi, if s.dir == 0 { 1 } else { s.dir });
                if refused {
                    // No message: the song just fades in its list, like every refused one.
                    remember_unplayable(&id);
                    s.unplayable.insert(id);
                } else if retried.as_ref() != Some(&id) { // a passing failure: one more try
                    retried = Some(id);
                    s.play_at(&player, from);
                    ctx.request_repaint();
                    continue;
                } else {
                    // Twice, yet its metadata says it plays: most likely Spotify withholding audio keys (in bursts
                    // on some accounts, on every song on some newer ones: librespot #1649). Skipping on would only
                    // ask for more keys and fail the whole queue silently: stop and say so.
                    retried = None; // pressing play again gets its second try again
                    (s.pos_ms, s.since) = (0, None);
                    s.error = Some("Spotify won't send this song right now (likely its audio-key limit); try again in a while".into());
                    ctx.request_repaint();
                    continue;
                }
                // Keep going the way we were going, so "back" doesn't bounce off it forever.
                if !s.step(&player, from, dir) { (s.pos_ms, s.since) = (0, None) }
            }
            _ => continue,
        }
        ctx.request_repaint();
    }
    Ok(())
}

/// Every liked track id, so lists can show hearts: Liked Songs as a playback context, all of it in one session
/// request (the Web API needs ~90 requests for 4000 songs, and its "are these liked?" lookup is 403).
pub async fn load_liked(state: Shared, session: Session, ctx: egui::Context) {
    let edits = state.lock().unwrap().liked_edits;
    // ponytail: on failure the hearts stay as saved; liking still works.
    let Ok(ids) = api::session_liked(&session).await else { return };
    let mut s = state.lock().unwrap();
    if s.liked_edits != edits { return } // a like landed meanwhile: this set may predate it
    (s.liked, s.liked_complete) = (ids, true);
    save_liked_ids(&s.liked);
    ctx.request_repaint();
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
    s.liked_edits += 1;
    // A partial set (the first full read still going, or stopped) is never saved: it would pass for complete.
    if s.liked_complete { save_liked_ids(&s.liked) }
}

#[test]
#[ignore = "live: needs ~/.resona/credentials.json, the network and an audio device (cargo test -- --ignored)"]
fn reconnect_brings_a_working_player() {
    use librespot::core::SpotifyUri;
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let dir = std::env::home_dir().unwrap().join(".resona");
        let cache = Cache::new(Some(&dir), None, None, None).unwrap();
        let creds = cache.credentials().expect("signed in");
        let mixer = Arc::new(SoftMixer::open(MixerConfig::default()).unwrap());
        let (session, player) = start(cache, creds, &mixer).await.unwrap();
        let e = Engine { player: RwLock::new(player), session: RwLock::new(session), web: Arc::new(api::WebAuth::new(&dir, REDIRECT).unwrap()), mixer, http: reqwest::Client::new() };
        let song = SpotifyUri::from_uri("spotify:track:35JnXQJ4coVhXXjvTHoalE").unwrap();
        let first = |mut ev: PlayerEventChannel| async move {
            loop { match ev.recv().await.unwrap() { PlayerEvent::Unavailable { .. } => return false, PlayerEvent::Playing { .. } => return true, _ => {} } }
        };
        e.session().shutdown(); // what a lost connection does (minus the dropped socket)
        assert!(e.session().is_invalid());
        let ev = e.reconnect().await.unwrap();
        assert!(!e.session().is_invalid());
        e.player().load(song, true, 0);
        assert!(first(ev).await);
        e.player().stop();
    });
}
