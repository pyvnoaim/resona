//! The few Spotify Web API calls the library view needs.
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use librespot::{
    core::{SpotifyUri, session::Session},
    metadata::{self, Metadata},
};
use librespot_oauth::{OAuthClient, OAuthClientBuilder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The client ID of your own app from developer.spotify.com, used only for the Web API (its redirect URI must be
/// REDIRECT in engine.rs): `RESONA_CLIENT_ID`, else `~/.resona/client_id`. Each person brings their own, set on the
/// first launch. librespot's shared client ID works for playback but gets day-long rate-limit bans here.
pub fn client_id(dir: &Path) -> Option<String> {
    let valid = |s: String| Some(s.trim().to_string()).filter(|s| valid_client_id(s));
    std::env::var("RESONA_CLIENT_ID").ok().and_then(valid)
        .or_else(|| std::fs::read_to_string(dir.join("client_id")).ok().and_then(valid))
}

/// Spotify client IDs are 32 hex digits.
pub fn valid_client_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Saves the client ID for every later launch.
pub fn save_client_id(dir: &Path, id: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("client_id"), format!("{}\n", id.trim()))
}

#[test]
fn checks_client_ids() {
    assert!(valid_client_id("8abbb289e35c43b6977234abdd3520d3"));
    assert!(!valid_client_id("8abbb289e35c43b6977234abdd3520d")); // one short
    assert!(!valid_client_id("https://developer.spotify.com/x"));
}
const SCOPES: [&str; 8] = [
    "user-follow-read",
    "user-library-read",
    "user-library-modify",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-public",
    "playlist-modify-private",
    "user-read-private",
];

/// Web API access: one browser sign-in, then silent refreshes from a refresh token saved on disk.
pub struct WebAuth {
    client: OAuthClient,
    refresh_file: PathBuf,
    token: Mutex<Option<(String, Instant)>>,
}

impl WebAuth {
    pub fn new(dir: &Path, redirect: &str) -> Result<Self, String> {
        let id = client_id(dir).ok_or("No Spotify app client ID: set it on the first-launch screen, or in ~/.resona/client_id")?;
        let client = OAuthClientBuilder::new(&id, redirect, SCOPES.to_vec()).open_in_browser().build().map_err(|e| e.to_string())?;
        Ok(Self { client, refresh_file: dir.join("web_refresh_token"), token: Mutex::new(None) })
    }

    // ponytail: two callers racing an expiry both refresh; harmless, Spotify accepts either.
    pub async fn token(&self) -> Result<String, String> {
        if let Some((t, exp)) = self.token.lock().unwrap().clone() && exp > Instant::now() + Duration::from_secs(60) {
            return Ok(t);
        }
        let saved = std::fs::read_to_string(&self.refresh_file).ok();
        let refreshed = match saved {
            // A sign-in from before we asked for more scopes refreshes fine but can't do everything: sign in again.
            Some(r) => self.client.refresh_token_async(r.trim()).await.ok().filter(|t| SCOPES.iter().all(|s| t.scopes.iter().any(|g| g == s))),
            None => None,
        };
        let fresh = match refreshed {
            Some(t) => t,
            None => self.client.get_access_token_async().await.map_err(|e| format!("Library sign-in failed: {e}"))?,
        };
        // A refresh may not hand out a new refresh token; then the saved one stays valid.
        if !fresh.refresh_token.is_empty() {
            save_private(&self.refresh_file, &fresh.refresh_token).map_err(|e| format!("Saving sign-in: {e}"))?;
        }
        *self.token.lock().unwrap() = Some((fresh.access_token.clone(), fresh.expires_at));
        Ok(fresh.access_token)
    }
}

/// Writes a secret readable only by you.
fn save_private(path: &Path, data: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new();
    f.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut f, 0o600);
    f.open(path)?.write_all(data.as_bytes())
}

#[derive(Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
    #[serde(default)]
    pub total: Option<usize>,
    /// Where this page starts in the whole list.
    #[serde(default)]
    pub offset: usize,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Image {
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Named {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Clone)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
    pub owner: Option<Owner>,
    #[serde(default)]
    pub collaborative: bool,
}

/// GET /search?type=track: the tracks come wrapped one level deeper than other lists.
#[derive(Deserialize)]
pub struct Search {
    pub tracks: Page<Track>,
}

/// Small pages (10): newer developer apps get a lower cap on search page size.
pub fn search_url(query: &str) -> String {
    reqwest::Url::parse_with_params("https://api.spotify.com/v1/search", [("q", query), ("type", "track"), ("limit", "10")]).unwrap().into()
}

impl Playlist {
    /// The Web API only lists tracks of playlists you own or collaborate on (403 otherwise, for developer apps).
    pub fn web_readable(&self, me: &str) -> bool {
        self.collaborative || self.owner.as_ref().and_then(|o| o.id.as_deref()) == Some(me)
    }
}

#[derive(Deserialize, Clone)]
pub struct Owner {
    pub id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Album {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub images: Vec<Image>,
}


/// Lenient on purpose: playlists also hold episodes and local files.
#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Track {
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: Vec<Named>,
    #[serde(default)]
    pub album: Album,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(rename = "type", default)]
    pub kind: String,
    /// When it was added to the list (ms since 1970; 0 when the list has no dates). Only for sorting.
    #[serde(skip)]
    pub added: i64,
    /// Its place in the list as Spotify keeps it, so "list order" can come back after a sort.
    #[serde(skip)]
    pub pos: usize,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }
    /// Smallest cover, for list rows and the player bar.
    pub fn thumb(&self) -> Option<&str> {
        self.album.images.last().map(|i| i.url.as_str())
    }
}

/// A playlist entry or saved track. Spotify names the field `track` or, on newer responses, `item`.
#[derive(Deserialize)]
pub struct Entry {
    track: Option<Track>,
    item: Option<Track>,
    added_at: Option<String>,
}

impl Entry {
    pub fn playable(self) -> Option<Track> {
        let added = self.added_at.as_deref().and_then(|a| librespot::core::date::Date::from_iso8601(a).ok()).map_or(0, |d| d.as_timestamp_ms());
        self.track.or(self.item).filter(|t| t.kind == "track" && t.id.is_some()).map(|t| Track { added, ..t })
    }
}

pub async fn get<T: DeserializeOwned>(auth: &WebAuth, http: &reqwest::Client, url: &str) -> Result<T, String> {
    send(auth, http, reqwest::Method::GET, url, None).await?.json().await.map_err(|e| e.to_string())
}

/// Appends a track to one of your playlists.
pub async fn add_to_playlist(auth: &WebAuth, http: &reqwest::Client, playlist: &str, track_id: &str) -> Result<(), String> {
    let body = format!(r#"{{"uris":["spotify:track:{track_id}"]}}"#);
    send(auth, http, reqwest::Method::POST, &format!("playlists/{playlist}/items"), Some(body)).await.map(drop)
}

/// Takes every copy of a track out of one of your playlists. The body says `items` since the 2026 changes (it was `tracks`).
pub async fn remove_from_playlist(auth: &WebAuth, http: &reqwest::Client, playlist: &str, track_id: &str) -> Result<(), String> {
    let body = format!(r#"{{"items":[{{"uri":"spotify:track:{track_id}"}}]}}"#);
    send(auth, http, reqwest::Method::DELETE, &format!("playlists/{playlist}/items"), Some(body)).await.map(drop)
}

/// Makes a new, private, empty playlist of yours. `POST /me/playlists` since the 2026 changes (it was under `/users/{id}`).
pub async fn create_playlist(auth: &WebAuth, http: &reqwest::Client, name: &str) -> Result<Playlist, String> {
    let body = serde_json::json!({ "name": name, "public": false }).to_string();
    send(auth, http, reqwest::Method::POST, "me/playlists", Some(body)).await?.json().await.map_err(|e| e.to_string())
}

/// Adds a track to Liked Songs, or removes it. Spotify moved saving to `/me/library` (by URI) in 2026;
/// the old `/me/tracks?ids=` answers 403.
pub async fn set_liked(auth: &WebAuth, http: &reqwest::Client, id: &str, on: bool) -> Result<(), String> {
    let method = if on { reqwest::Method::PUT } else { reqwest::Method::DELETE };
    send(auth, http, method, &format!("me/library?uris=spotify:track:{id}"), None).await.map(drop)
}

/// One Web API call (with an optional JSON body), auth and rate-limit handling; errors for non-2xx answers.
async fn send(auth: &WebAuth, http: &reqwest::Client, method: reqwest::Method, url: &str, json: Option<String>) -> Result<reqwest::Response, String> {
    let url = if url.starts_with("http") { url.to_string() } else { format!("https://api.spotify.com/v1/{url}") };
    loop {
        let token = auth.token().await?;
        let req = http.request(method.clone(), &url).bearer_auth(&token);
        let req = match &json {
            Some(body) => req.header("content-type", "application/json").body(body.clone()),
            None => req.header("content-length", "0"),
        };
        let r = req.send().await.map_err(|e| e.to_string())?;
        if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let wait: u64 = r.headers().get("retry-after").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(2);
            // Short waits are retried quietly; long ones (the shared client ID gets day-long bans) become an error.
            if wait > 60 {
                let left = if wait >= 3600 { format!("{}h", wait.div_ceil(3600)) } else { format!("{}m", wait.div_ceil(60)) };
                return Err(format!("Spotify rate limit, try again in {left}"));
            }
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }
        return r.error_for_status().map_err(|e| e.to_string());
    }
}

#[test]
fn entry_accepts_both_shapes_and_skips_episodes() {
    let page: Page<Entry> = serde_json::from_str(r#"{"next":null,"items":[
        {"track":{"id":"a","name":"A","type":"track","duration_ms":1000,"artists":[{"name":"X"},{"name":"Y"}],"album":{"name":"Al","images":[{"url":"big"},{"url":"small"}]}}},
        {"item":{"id":"b","name":"B","type":"track"}},
        {"track":{"id":"c","name":"Pod","type":"episode"}},
        {"track":{"id":null,"name":"Local","type":"track"}},
        {"track":null}
    ]}"#).unwrap();
    let tracks: Vec<Track> = page.items.into_iter().filter_map(Entry::playable).collect();
    assert_eq!(tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(tracks[0].artist_line(), "X, Y");
    assert_eq!(tracks[0].thumb(), Some("small"));
}

/// Your playlists as the official client lists them (the rootlist), through the session: the sidebar's
/// fallback when the Web API refuses (a rate-limit ban). No owner id, so `web_readable` is false and they
/// open through the session too.
pub async fn session_playlists(session: &Session) -> Result<Vec<Playlist>, String> {
    use librespot::protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    // ponytail: one page of 500 entries (folder markers count); page with `from` if a library ever outgrows it.
    let bytes = session.spclient().get_rootlist(0, Some(500)).await.map_err(|e| format!("Your playlists: {e}"))?;
    let list = SelectedListContent::parse_from_bytes(&bytes).map_err(|e| format!("Your playlists: {e}"))?;
    rootlist_playlists(&list).ok_or_else(|| "Your playlists: names and lists don't line up".to_string())
}

/// The playlists in a decorated rootlist: each item's name sits in the meta item at the same position, so a
/// mismatch in count is refused rather than putting names on the wrong lists.
fn rootlist_playlists(list: &librespot::protocol::playlist4_external::SelectedListContent) -> Option<Vec<Playlist>> {
    let contents = list.contents.get_or_default();
    if contents.items.len() != contents.meta_items.len() { return None }
    Some(contents.items.iter().zip(&contents.meta_items).filter_map(|(item, meta)| {
        let id = item.uri().strip_prefix("spotify:playlist:")?; // folders are start-group / end-group items
        // Covers: the sized links the official client uses, else the picture id as a CDN link.
        let a = &meta.attributes;
        // Largest first, smallest last, like the Web API (the sidebar takes the last, the header the second).
        let rank = |name: &str| ["xlarge", "large", "default", "small"].iter().position(|n| *n == name).unwrap_or(2);
        let mut sizes: Vec<_> = a.picture_size.iter().filter(|p| !p.url().is_empty()).collect();
        sizes.sort_by_key(|p| rank(p.target_name()));
        let mut images: Vec<Image> = sizes.into_iter().map(|p| Image { url: p.url().into() }).collect();
        if images.is_empty() && !a.picture().is_empty() {
            let hex: String = a.picture().iter().map(|b| format!("{b:02x}")).collect();
            images.push(Image { url: format!("https://i.scdn.co/image/{hex}") });
        }
        // No owner: the rootlist only has account ids, not names.
        Some(Playlist { id: id.into(), name: a.name().into(), images: Some(images).filter(|i| !i.is_empty()), owner: None, collaborative: false })
    }).collect())
}

/// Every liked song id through the session: Liked Songs resolved as a playback context, works through a Web API ban.
pub async fn session_liked(session: &Session) -> Result<std::collections::HashSet<String>, String> {
    let uri = format!("spotify:user:{}:collection", session.username());
    let ctx = patient(|| session.spclient().get_context(&uri)).await.map_err(|e| format!("Liked Songs: {e}"))?;
    // ponytail: one page held all 4400 songs; follow `next_page_url` if Spotify ever splits it.
    Ok(ctx.pages.iter().flat_map(|p| &p.tracks).filter_map(|t| t.uri().strip_prefix("spotify:track:")).map(String::from).collect())
}

/// Where a list stands now, asked cheaply, so a copy kept from last time can be trusted: a playlist's revision
/// (one session request, for one song), Liked Songs' ids (one request, the same as the hearts). None for lists
/// that aren't kept, or when Spotify can't say.
pub async fn list_version(session: &Session, src: &crate::state::Source) -> Option<String> {
    use crate::state::Source;
    use librespot::protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    use std::hash::{DefaultHasher, Hash, Hasher};
    match src {
        Source::Playlist(id) => {
            let path = format!("/playlist/v2/playlist/{id}?from=0&length=1");
            let bytes = patient(|| session.spclient().request(&reqwest::Method::GET, &path, None, None)).await.ok()?;
            let list = SelectedListContent::parse_from_bytes(&bytes).ok()?;
            Some(list.revision().iter().map(|b| format!("{b:02x}")).collect::<String>()).filter(|v| !v.is_empty())
        }
        Source::Liked => {
            let mut ids: Vec<String> = session_liked(session).await.ok()?.into_iter().collect();
            ids.sort();
            let mut h = DefaultHasher::new();
            ids.hash(&mut h);
            Some(format!("{:016x}", h.finish()))
        }
        _ => None,
    }
}

/// The cover Spotify draws for a playlist without its own picture: its first four albums as a mosaic, or the
/// first album's cover when it has fewer. Read through the session, so it works through a Web API ban.
pub async fn session_mosaic(session: &Session, id: &str) -> Option<String> {
    use librespot::protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    let path = format!("/playlist/v2/playlist/{id}?from=0&length=20");
    let bytes = patient(|| session.spclient().request(&reqwest::Method::GET, &path, None, None)).await.ok()?;
    let list = SelectedListContent::parse_from_bytes(&bytes).ok()?;
    let mut covers: Vec<String> = Vec::new();
    for it in &list.contents.get_or_default().items {
        let Ok(u @ SpotifyUri::Track { .. }) = SpotifyUri::from_uri(it.uri()) else { continue };
        // One at a time: usually four songs do it, and the player shares this limiter.
        let Ok(t) = patient(|| metadata::Track::get(session, &u)).await else { continue };
        if let Some(c) = t.album.covers.first().and_then(|c| c.id.to_base16().ok()) && !covers.contains(&c) {
            covers.push(c);
            if covers.len() == 4 { break }
        }
    }
    covers.sort(); // Spotify's own mosaics have their tiles in this order
    mosaic_url(&covers)
}

fn mosaic_url(covers: &[String]) -> Option<String> {
    match covers {
        [] => None,
        [a, b, c, d, ..] => Some(format!("https://mosaic.scdn.co/300/{a}{b}{c}{d}")),
        [a, ..] => Some(format!("https://i.scdn.co/image/{a}")),
    }
}

#[test]
fn mosaic_needs_four_albums_else_the_first_cover() {
    let c = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>();
    assert_eq!(mosaic_url(&c(0)), None);
    assert_eq!(mosaic_url(&c(3)).unwrap(), "https://i.scdn.co/image/c0");
    assert_eq!(mosaic_url(&c(4)).unwrap(), "https://mosaic.scdn.co/300/c0c1c2c3");
}

#[test]
fn rootlist_keeps_playlists_skips_folders_and_never_reads_as_web_readable() {
    use librespot::protocol::playlist4_external::{Item, MetaItem, SelectedListContent};
    let mut list = SelectedListContent::new();
    let contents = list.contents.mut_or_insert_default();
    for (uri, name, owner) in [("spotify:start-group:abc:Folder", "", ""), ("spotify:playlist:p1", "lamp", "pyvno"), ("spotify:end-group:abc", "", "")] {
        let mut item = Item::new();
        item.set_uri(uri.into());
        contents.items.push(item);
        let mut meta = MetaItem::new();
        meta.attributes.mut_or_insert_default().set_name(name.into());
        meta.set_owner_username(owner.into());
        contents.meta_items.push(meta);
    }
    let got = rootlist_playlists(&list).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].id.as_str(), got[0].name.as_str()), ("p1", "lamp"));
    assert!(!got[0].web_readable("pyvno")); // opens through the session, which bans don't touch
    list.contents.mut_or_insert_default().meta_items.pop();
    assert!(rootlist_playlists(&list).is_none()); // names that don't line up are refused, not shifted
}

/// Tracks of a playlist someone else owns, read through the session like the official client does.
/// The song list (ids and dates) comes first, so songs are fetched in `sort` order: the top of the list fills
/// in first. Hands them over `CHUNK` at a time with the playlist's track count; stops early when `each` returns
/// false (the user moved on).
pub async fn session_playlist(session: &Session, id: &str, sort: crate::state::Sort, mut each: impl FnMut(Vec<Track>, usize) -> bool) -> Result<(), String> {
    use librespot::protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;
    const CHUNK: usize = 25;
    const PAGE: usize = 100;
    // librespot allows 300 requests per 30 s (then 10/s), shared with playback. Past the first 200 songs, go
    // at ~8/s so the player keeps room: when it's starved, a playable song gets reported as refused.
    const BURST: usize = 200;
    // Page through the list ourselves: librespot's `Playlist::get` asks once, for the first part only.
    let (mut from, mut songs) = (0, Vec::new());
    let mut total;
    loop {
        let path = format!("/playlist/v2/playlist/{id}?from={from}&length={PAGE}");
        let bytes = patient(|| session.spclient().request(&reqwest::Method::GET, &path, None, None)).await.map_err(|e| format!("Playlist: {e}"))?;
        let list = SelectedListContent::parse_from_bytes(&bytes).map_err(|e| format!("Playlist: {e}"))?;
        total = list.length().max(0) as usize;
        if !each(Vec::new(), total) { return Ok(()) }
        let items = &list.contents.get_or_default().items;
        songs.extend(items.iter().enumerate()
            .filter_map(|(i, it)| Some((SpotifyUri::from_uri(it.uri()).ok()?, it.attributes.timestamp(), from + i)))
            .filter(|(u, _, _)| matches!(u, SpotifyUri::Track { .. })));
        from += items.len();
        if items.is_empty() || (total > 0 && from >= total) { break }
    }
    sort.order(&mut songs, |(_, added, pos)| (*added, *pos));
    for (n, chunk) in songs.chunks(CHUNK).enumerate() {
        if !each(Vec::new(), total) { return Ok(()) } // moved on: stop before spending more of the budget
        if n * CHUNK >= BURST { tokio::time::sleep(Duration::from_secs(3)).await }
        let mut set = tokio::task::JoinSet::new();
        for (i, (u, added, pos)) in chunk.iter().cloned().enumerate() {
            let session = session.clone();
            set.spawn(async move { (i, patient(|| metadata::Track::get(&session, &u)).await.map(|t| Track { added, pos, ..Track::from(t) })) });
        }
        let mut got: Vec<Option<Track>> = (0..chunk.len()).map(|_| None).collect();
        while let Some(Ok((i, t))) = set.join_next().await {
            got[i] = t.ok(); // unavailable tracks just drop out
        }
        if !each(got.into_iter().flatten().collect(), total) { return Ok(()) }
    }
    Ok(())
}

/// Runs a session request, waiting out librespot's own rate limiter ("Resource has been exhausted") instead of
/// failing: it refills at 10 requests a second, so a short wait always gets through.
// ponytail: spots the limiter by its message text; if librespot rewords it, requests fail again as before
// (songs drop out). Match its error kind if it ever exposes one.
async fn patient<T, E: std::fmt::Display, F: std::future::Future<Output = Result<T, E>>>(mut call: impl FnMut() -> F) -> Result<T, E> {
    for wait in [1, 1, 2, 2, 4, 4, 8] {
        match call().await {
            Err(e) if e.to_string().contains("exhausted") => tokio::time::sleep(Duration::from_secs(wait)).await,
            done => return done,
        }
    }
    call().await
}

/// Whether Spotify itself refuses this song here (region, licensing, no files and no stand-in), as opposed to
/// a load that just failed: librespot reports a lost audio key or CDN hiccup as `Unavailable` too.
pub async fn refused(session: &Session, id: &str) -> bool {
    let Ok(uri) = SpotifyUri::from_uri(&format!("spotify:track:{id}")) else { return true };
    match patient(|| metadata::audio::AudioItem::get_file(session, uri.clone())).await {
        Ok(a) => a.availability.is_err() || (a.files.is_empty() && a.alternatives.as_ref().is_none_or(|t| t.is_empty())),
        Err(_) => false, // can't tell: don't fade a song for good on a guess
    }
}

impl From<metadata::Track> for Track {
    fn from(t: metadata::Track) -> Self {
        // Largest first, smallest last, like the Web API (`thumb()` takes the last one).
        let rank = |s: &metadata::image::ImageSize| {
            use metadata::image::ImageSize::*;
            match s { SMALL => 0, DEFAULT => 1, LARGE => 2, XLARGE => 3 }
        };
        let mut covers: Vec<_> = t.album.covers.iter().collect();
        covers.sort_by_key(|c| std::cmp::Reverse(rank(&c.size)));
        let images = covers.into_iter()
            .filter_map(|c| c.id.to_base16().ok())
            .map(|hex| Image { url: format!("https://i.scdn.co/image/{hex}") })
            .collect();
        Track {
            id: t.id.to_id().ok(),
            name: t.name,
            artists: t.artists.iter().map(|a| Named { id: a.id.to_id().ok(), name: a.name.clone() }).collect(),
            album: Album { id: t.album.id.to_id().ok(), name: t.album.name, images },
            duration_ms: t.duration.max(0) as u32,
            kind: "track".into(),
            added: 0,
            pos: 0,
        }
    }
}

#[test]
fn only_own_or_collaborative_playlists_go_through_the_web_api() {
    let p = |owner: &str, collaborative: bool| -> Playlist {
        serde_json::from_value(serde_json::json!({"id": "x", "name": "n", "owner": {"id": owner}, "collaborative": collaborative})).unwrap()
    };
    assert!(p("me", false).web_readable("me"));
    assert!(p("them", true).web_readable("me"));
    assert!(!p("them", false).web_readable("me"));
}

#[test]
fn search_url_encodes_the_query() {
    assert_eq!(search_url("sigur rós & co"), "https://api.spotify.com/v1/search?q=sigur+r%C3%B3s+%26+co&type=track&limit=10");
}

/// Tracks by id through the session, 25 at a time in parallel, in the order given. Unplayable ones drop out.
async fn session_tracks(session: &Session, uris: &[SpotifyUri]) -> Vec<Track> {
    let mut out = Vec::with_capacity(uris.len());
    for chunk in uris.chunks(25) {
        let mut set = tokio::task::JoinSet::new();
        for (i, u) in chunk.iter().cloned().enumerate() {
            let session = session.clone();
            set.spawn(async move { (i, patient(|| metadata::Track::get(&session, &u)).await) });
        }
        let mut got: Vec<Option<Track>> = (0..chunk.len()).map(|_| None).collect();
        while let Some(Ok((i, t))) = set.join_next().await {
            got[i] = t.ok().map(Track::from);
        }
        out.extend(got.into_iter().flatten());
    }
    out
}

/// The ~300px cover (Spotify's DEFAULT size) of a metadata album or artist, as a URL.
fn cover_url(images: &metadata::image::Images) -> Option<String> {
    use metadata::image::ImageSize::*;
    let pick = images.iter().find(|i| i.size == DEFAULT).or_else(|| images.first())?;
    Some(format!("https://i.scdn.co/image/{}", pick.id.to_base16().ok()?))
}

/// GET /artists/{id}: only its pictures are used (as the artist page's portrait).
#[derive(Deserialize)]
pub struct ArtistImages {
    #[serde(default)]
    pub images: Vec<Image>,
}

/// One album, EP or single on an artist page.
#[derive(Clone)]
pub struct Release {
    pub id: String,
    pub name: String,
    pub year: i32,
    pub kind: &'static str,
    pub cover: Option<String>,
}

/// The playlist Spotify makes as a song's radio (similar songs), as the official client asks for it.
pub async fn radio_playlist(session: &Session, track: &str) -> Result<String, String> {
    let path = format!("/inspiredby-mix/v2/seed_to_playlist/spotify:track:{track}?response-format=json");
    let bytes = patient(|| session.spclient().request(&reqwest::Method::GET, &path, None, None)).await.map_err(|e| format!("Song radio: {e}"))?;
    radio_id(&bytes).ok_or_else(|| "Song radio: Spotify has no radio for this song".to_string())
}

/// `{"mediaItems": [{"uri": "spotify:playlist:<id>"}]}` → the id.
fn radio_id(json: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Mix { media_items: Vec<Item> }
    #[derive(Deserialize)]
    struct Item { uri: String }
    let mix: Mix = serde_json::from_slice(json).ok()?;
    mix.media_items.into_iter().find_map(|i| i.uri.strip_prefix("spotify:playlist:").map(String::from))
}

#[test]
fn reads_the_radio_playlist() {
    assert_eq!(radio_id(br#"{"total": 1,"mediaItems": [{"uri": "spotify:playlist:37i9dQZF1E8NrW9Kj6hiiV"}]}"#).as_deref(), Some("37i9dQZF1E8NrW9Kj6hiiV"));
    assert_eq!(radio_id(br#"{"total": 0,"mediaItems": []}"#), None);
}

/// An artist page: portrait, top songs (for the account's country), and every album / EP / single,
/// newest first. Through the session: the Web API's artist endpoints are hit and miss for developer apps.
pub async fn session_artist(session: &Session, id: &str) -> Result<(Option<String>, Vec<Track>, Vec<Release>), String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:artist:{id}")).map_err(|e| e.to_string())?;
    let artist = metadata::Artist::get(session, &uri).await.map_err(|e| format!("Artist: {e}"))?;
    // Spotify fills `portrait_group` more often than `portraits`; main.rs falls back to the Web API.
    let portrait = cover_url(&artist.portrait_group).or_else(|| cover_url(&artist.portraits));
    let top = session_tracks(session, &artist.top_tracks.for_country(&session.country()).0).await;
    let ids: Vec<SpotifyUri> = artist.albums_current().chain(artist.singles_current()).chain(artist.compilations_current()).cloned().collect();
    let mut releases = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(25) {
        let mut set = tokio::task::JoinSet::new();
        for u in chunk {
            let (session, u) = (session.clone(), u.clone());
            set.spawn(async move { metadata::Album::get(&session, &u).await });
        }
        while let Some(Ok(Ok(a))) = set.join_next().await {
            use metadata::album::AlbumType::*;
            releases.push(Release {
                id: a.id.to_id().unwrap_or_default(),
                year: a.date.year(),
                kind: match a.album_type { SINGLE => "SINGLE", EP => "EP", COMPILATION => "COMPILATION", _ => "ALBUM" },
                cover: cover_url(&a.covers),
                name: a.name,
            });
        }
    }
    releases.sort_by(|a, b| b.year.cmp(&a.year).then(a.name.cmp(&b.name)));
    releases.dedup_by(|a, b| a.id == b.id);
    Ok((portrait, top, releases))
}

/// The artists you follow (ids), through the Web API: the session has no such list.
pub async fn followed_artists(auth: &WebAuth, http: &reqwest::Client) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Following { artists: Page }
    #[derive(Deserialize)]
    struct Page { items: Vec<Item>, cursors: Option<Cursors> }
    #[derive(Deserialize)]
    struct Item { id: String }
    #[derive(Deserialize)]
    struct Cursors { after: Option<String> }
    let (mut ids, mut after) = (Vec::new(), None::<String>);
    loop {
        let url = format!("me/following?type=artist&limit=50{}", after.map(|a| format!("&after={a}")).unwrap_or_default());
        let page = get::<Following>(auth, http, &url).await.map_err(|e| format!("Followed artists: {e}"))?.artists;
        let done = page.items.len() < 50;
        ids.extend(page.items.into_iter().map(|i| i.id));
        after = page.cursors.and_then(|c| c.after);
        if done || after.is_none() { return Ok(ids) }
    }
}

/// New releases by these artists (albums, EPs and singles out since `since`, ms since 1970), through the
/// session. An artist's releases come newest first, so each list is read only up to the first older one:
/// a few requests per artist. Songs go to `each` one release at a time, `added` set to the release day.
pub async fn session_new_releases(session: &Session, artists: &[String], since: i64, mut each: impl FnMut(Vec<Track>) -> bool) {
    let mut seen = std::collections::HashSet::new(); // a release shared by two followed artists comes once
    for chunk in artists.chunks(8) {
        let mut set = tokio::task::JoinSet::new();
        for id in chunk {
            let (session, id) = (session.clone(), id.clone());
            set.spawn(async move { fresh_releases(&session, &id, since).await });
        }
        while let Some(done) = set.join_next().await {
            let Ok(albums) = done else { continue }; // one artist failing keeps the others
            for a in albums {
                if !seen.insert(a.id.clone()) { continue }
                let day = a.date.as_timestamp_ms();
                let uris: Vec<SpotifyUri> = a.tracks().cloned().collect();
                let songs = session_tracks(session, &uris).await.into_iter().map(|t| Track { added: day, ..t }).collect();
                if !each(songs) { return }
            }
        }
    }
}

/// One artist's albums and singles out since `since`.
async fn fresh_releases(session: &Session, id: &str, since: i64) -> Vec<metadata::Album> {
    let Ok(uri) = SpotifyUri::from_uri(&format!("spotify:artist:{id}")) else { return vec![] };
    let Ok(artist) = patient(|| metadata::Artist::get(session, &uri)).await else { return vec![] };
    let mut out = vec![];
    // ponytail: trusts Spotify's newest-first order of each list (true in every artist checked); if it ever
    // isn't, read every release and filter, at a few times the requests.
    for list in [artist.albums_current().cloned().collect::<Vec<_>>(), artist.singles_current().cloned().collect()] {
        for u in list {
            match patient(|| metadata::Album::get(session, &u)).await {
                Ok(a) if a.date.as_timestamp_ms() >= since => out.push(a),
                Ok(_) => break, // older than `since`: the rest of this list is older still
                Err(_) => continue, // unreadable just now: skip it, not the artist's other new ones
            }
        }
    }
    out
}

/// An album's tracks, in order, through the session.
pub async fn session_album(session: &Session, id: &str) -> Result<Vec<Track>, String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:album:{id}")).map_err(|e| e.to_string())?;
    let album = metadata::Album::get(session, &uri).await.map_err(|e| format!("Album: {e}"))?;
    let uris: Vec<SpotifyUri> = album.tracks().cloned().collect();
    Ok(session_tracks(session, &uris).await)
}

#[test]
fn patient_waits_out_the_limiter_and_passes_other_errors_straight_through() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let calls = std::cell::Cell::new(0);
    let limited = rt.block_on(patient(|| { calls.set(calls.get() + 1); let n = calls.get(); async move { if n == 1 { Err("Resource has been exhausted") } else { Ok(n) } } }));
    assert_eq!(limited, Ok(2)); // waited once, then got through
    calls.set(0);
    let other = rt.block_on(patient(|| { calls.set(calls.get() + 1); async { Err::<(), _>("not found") } }));
    assert_eq!((other, calls.get()), (Err("not found"), 1)); // no retry for real errors
}



#[test]
#[ignore = "live: needs ~/.resona/credentials.json and the network (cargo test -- --ignored)"]
fn refused_tells_a_refused_song_from_a_playable_one() {
    use librespot::core::{cache::Cache, config::SessionConfig};
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cache = Cache::new(Some(std::env::home_dir().unwrap().join(".resona")), None, None, None).unwrap();
        let creds = cache.credentials().expect("signed in");
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(creds, true).await.unwrap();
        assert!(refused(&session, "6BJXuY0lY0EUtSDCgkYnqE").await); // no files, no stand-in
        assert!(!refused(&session, "35JnXQJ4coVhXXjvTHoalE").await); // plays
        assert!(!refused(&session, "7GN6EEENu7xnl2lEipjN4X").await); // plays through its stand-in
    });
}


#[test]
#[ignore = "live: needs ~/.resona/credentials.json and the network (cargo test -- --ignored)"]
fn list_version_is_steady_for_an_unchanged_list() {
    use crate::state::Source;
    use librespot::core::{cache::Cache, config::SessionConfig};
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cache = Cache::new(Some(std::env::home_dir().unwrap().join(".resona")), None, None, None).unwrap();
        let creds = cache.credentials().expect("signed in");
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(creds, true).await.unwrap();
        let first = session_playlists(&session).await.unwrap().remove(0);
        for src in [Source::Liked, Source::Playlist(first.id)] {
            let v = list_version(&session, &src).await;
            assert!(v.is_some());
            assert_eq!(v, list_version(&session, &src).await);
        }
        assert_eq!(list_version(&session, &Source::Search("x".into())).await, None);
    });
}
