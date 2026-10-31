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
const SCOPES: [&str; 9] = [
    "user-follow-read",
    "user-follow-modify",
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
    /// Release day and the album's own artists: filled only on album pages (`session_album`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub released: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by: Vec<Named>,
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

/// Saves someone else's playlist to your library (it shows in the sidebar), through `/me/library` by URI.
pub async fn save_playlist(auth: &WebAuth, http: &reqwest::Client, id: &str) -> Result<(), String> {
    send(auth, http, reqwest::Method::PUT, &format!("me/library?uris=spotify:playlist:{id}"), None).await.map(drop)
}

/// Follows an artist, or unfollows. Through `/me/library` by URI, like liking since the 2026 changes.
pub async fn set_followed(auth: &WebAuth, http: &reqwest::Client, id: &str, on: bool) -> Result<(), String> {
    let method = if on { reqwest::Method::PUT } else { reqwest::Method::DELETE };
    send(auth, http, method, &format!("me/library?uris=spotify:artist:{id}"), None).await.map(drop)
}

/// Whether you follow an artist, as Spotify says now (`GET /me/library/contains`, by URI).
pub async fn is_followed(auth: &WebAuth, http: &reqwest::Client, id: &str) -> Result<bool, String> {
    let answer: Vec<bool> = get(auth, http, &format!("me/library/contains?uris=spotify:artist:{id}")).await?;
    answer.first().copied().ok_or_else(|| "Follow check: empty answer".to_string())
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
    match src {
        Source::Playlist(id) => {
            let path = format!("/playlist/v2/playlist/{id}?from=0&length=1");
            let bytes = patient(|| session.spclient().request(&reqwest::Method::GET, &path, None, None)).await.ok()?;
            let list = SelectedListContent::parse_from_bytes(&bytes).ok()?;
            Some(list.revision().iter().map(|b| format!("{b:02x}")).collect::<String>()).filter(|v| !v.is_empty())
        }
        Source::Liked => Some(liked_version(&session_liked(session).await.ok()?)),
        _ => None,
    }
}

/// Liked Songs' version: a hash of its ids, in any order.
pub fn liked_version(ids: &std::collections::HashSet<String>) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut ids: Vec<&String> = ids.iter().collect();
    ids.sort();
    let mut h = DefaultHasher::new();
    ids.hash(&mut h);
    format!("{:016x}", h.finish())
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
            album: Album { id: t.album.id.to_id().ok(), name: t.album.name, images, ..Default::default() },
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
    /// The release day, ms since 1970 (midnight UTC).
    pub day: i64,
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

/// An artist page: portrait, popularity (Spotify's 0–100 score), top songs (for the account's country), every album / EP / single newest
/// first, and the latest songs (the newest releases' first ten). Through the session: the Web API's artist endpoints are hit and miss for developer apps.
/// The releases come in one web-player query when it answers, else one session read per release.
pub async fn session_artist(session: &Session, http: &reqwest::Client, id: &str) -> Result<(Option<String>, u8, Vec<Track>, Vec<Release>, Vec<Track>), String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:artist:{id}")).map_err(|e| e.to_string())?;
    let artist = metadata::Artist::get(session, &uri).await.map_err(|e| format!("Artist: {e}"))?;
    // Spotify fills `portrait_group` more often than `portraits`; main.rs falls back to the Web API.
    let portrait = cover_url(&artist.portrait_group).or_else(|| cover_url(&artist.portraits));
    let top = session_tracks(session, &artist.top_tracks.for_country(&session.country()).0).await;
    let (releases, mut fresh) = match web_releases(session, http, id, i64::MIN).await {
        Some(releases) => {
            // Latest songs: read only the newest releases, enough of them for ten songs.
            let mut fresh = vec![];
            for r in &releases {
                if fresh.len() >= 10 { break }
                let Ok(u) = SpotifyUri::from_uri(&format!("spotify:album:{}", r.id)) else { continue };
                if let Ok(a) = patient(|| metadata::Album::get(session, &u)).await { fresh.extend(a.tracks().cloned()) }
            }
            (releases, fresh)
        }
        None => session_releases(session, &artist).await,
    };
    fresh.truncate(10); // as many as the top songs list holds
    let latest = session_tracks(session, &fresh).await;
    Ok((portrait, artist.popularity.clamp(0, 100) as u8, top, releases, latest))
}

/// Every release read one by one through the session, newest first, and the newest ones' songs.
async fn session_releases(session: &Session, artist: &metadata::Artist) -> (Vec<Release>, Vec<SpotifyUri>) {
    let ids: Vec<SpotifyUri> = artist.albums_current().chain(artist.singles_current()).chain(artist.compilations_current()).cloned().collect();
    let mut albums = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(25) {
        let mut set = tokio::task::JoinSet::new();
        for u in chunk {
            let (session, u) = (session.clone(), u.clone());
            set.spawn(async move { patient(|| metadata::Album::get(&session, &u)).await });
        }
        while let Some(done) = set.join_next().await {
            if let Ok(Ok(a)) = done { albums.push(a) } // one unreadable release keeps the others
        }
    }
    albums.sort_by_key(|a| std::cmp::Reverse(a.date.as_timestamp_ms()));
    albums.dedup_by(|a, b| a.id == b.id);
    let fresh = albums.iter().flat_map(|a| a.tracks().cloned()).take(10).collect();
    let releases = albums.into_iter().map(|a| {
        use metadata::album::AlbumType::*;
        Release {
            id: a.id.to_id().unwrap_or_default(),
            year: a.date.year(),
            day: a.date.as_timestamp_ms(),
            kind: match a.album_type { SINGLE => "SINGLE", EP => "EP", COMPILATION => "COMPILATION", _ => "ALBUM" },
            cover: cover_url(&a.covers),
            name: a.name,
        }
    }).collect();
    (releases, fresh)
}

/// One of the web player's own GraphQL queries, signed in with the session's tokens (so no developer-app call).
/// ponytail: a private endpoint and fixed persisted-query hashes; callers fall back or leave things out when it
/// doesn't answer. Upgrade: take the current hashes from the web player.
async fn web_query(session: &Session, http: &reqwest::Client, op: &str, hash: &str, variables: serde_json::Value) -> Option<Vec<u8>> {
    let token = session.login5().auth_token().await.ok()?.access_token;
    let client = session.spclient().client_token().await.ok()?;
    let body = serde_json::json!({ "operationName": op, "variables": variables, "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } } });
    // A bounded wait: every caller has a fallback, and a stalled answer would hold its page on the spinner.
    let r = http.post("https://api-partner.spotify.com/pathfinder/v2/query").bearer_auth(token).header("client-token", client)
        .header("app-platform", "WebPlayer").timeout(std::time::Duration::from_secs(10)).json(&body).send().await.ok()?;
    if !r.status().is_success() { return None }
    Some(r.bytes().await.ok()?.to_vec())
}

#[derive(Deserialize)]
struct Answer<T> { data: Data<T> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Data<T> { artist_union: T }
/// A picture in the sizes the web player's answers list.
#[derive(Deserialize, Default)]
struct Pics { sources: Vec<Pic> }
#[derive(Deserialize)]
struct Pic { url: String, width: Option<u32> }

/// The smallest picture still 200 px or wider (covers and portraits come in 64, ~300 and 640).
fn pick(p: &Pics) -> Option<String> {
    p.sources.iter().filter(|i| i.width.unwrap_or(0) >= 200).min_by_key(|i| i.width).or(p.sources.first()).map(|i| i.url.clone())
}

/// An artist's albums, EPs, singles and compilations out since `since` (ms since 1970; `i64::MIN` for all),
/// newest first, from the web player's discography query: pages of 100 for all, of 10 for the latest.
async fn web_releases(session: &Session, http: &reqwest::Client, id: &str, since: i64) -> Option<Vec<Release>> {
    let (mut out, mut offset, limit) = (vec![], 0, if since == i64::MIN { 100 } else { 10 });
    loop {
        let vars = serde_json::json!({ "uri": format!("spotify:artist:{id}"), "offset": offset, "limit": limit, "order": "DATE_DESC" });
        let json = web_query(session, http, "queryArtistDiscographyAll", "9380995a9d4663cbcb5113fef3c6aabf70ae6d407ba61793fd01e2a1dd6929b0", vars).await?;
        let (page, read, total) = discography(&json)?;
        let older = page.iter().any(|r| r.day < since); // newest first: the rest are older still
        offset += read; // entries, counting any empty one `discography` dropped, so the next page starts after them
        out.extend(page.into_iter().filter(|r| r.day >= since));
        if read == 0 || older || offset >= total { return Some(out) }
    }
}

/// Midnight UTC of a day ("2026-10-23…"), ms since 1970; a year alone counts as its 1 January.
fn day_ms(iso: &str) -> Option<i64> {
    let part = |r: std::ops::Range<usize>| iso.get(r).and_then(|p| p.parse::<i64>().ok());
    let (y, m, d) = (part(0..4)?, part(5..7).unwrap_or(1), part(8..10).unwrap_or(1));
    // Days from 1970-01-01 to y-m-d (proleptic Gregorian), after Howard Hinnant's `days_from_civil`.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400_000)
}

#[test]
fn counts_days_from_1970() {
    assert_eq!(day_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(day_ms("2000-03-01T00:00:00Z"), Some(951_868_800_000));
    assert_eq!(day_ms("2026-10-23T00:00:00Z"), Some(1_792_713_600_000));
    assert_eq!(day_ms("2019"), day_ms("2019-01-01"));
    assert_eq!(day_ms("soon"), None);
}

/// A page of releases, how many entries the page held (an empty one gives no release), and how many there are
/// in all, out of a `queryArtistDiscographyAll` answer.
fn discography(json: &[u8]) -> Option<(Vec<Release>, usize, usize)> {
    #[derive(Deserialize)]
    struct Union { discography: Disc }
    #[derive(Deserialize)]
    struct Disc { all: All }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct All { total_count: usize, items: Vec<Group> }
    #[derive(Deserialize)]
    struct Group { releases: Items }
    #[derive(Deserialize)]
    struct Items { items: Vec<Item> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item { id: String, name: String, r#type: String, date: Date, #[serde(default)] cover_art: Pics }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Date { year: i32, iso_string: Option<String> }
    let all = serde_json::from_slice::<Answer<Union>>(json).ok()?.data.artist_union.discography.all;
    let read = all.items.len();
    let releases = all.items.into_iter().filter_map(|g| g.releases.items.into_iter().next()).map(|r| Release {
        kind: match r.r#type.as_str() { "SINGLE" => "SINGLE", "EP" => "EP", "COMPILATION" => "COMPILATION", _ => "ALBUM" },
        cover: pick(&r.cover_art),
        day: r.date.iso_string.as_deref().and_then(day_ms).or_else(|| day_ms(&r.date.year.to_string())).unwrap_or(0),
        id: r.id, name: r.name, year: r.date.year,
    }).collect();
    Some((releases, read, all.total_count))
}

#[test]
fn reads_a_discography_page() {
    let json = br#"{"data":{"artistUnion":{"discography":{"all":{"totalCount":56,"items":[
        {"releases":{"items":[{"id":"2AyM","name":"Luh Leakyz Vol. 1","type":"ALBUM","date":{"year":2026,"isoString":"2026-10-23T00:00:00Z"},"coverArt":{"sources":[
            {"url":"big","width":640},{"url":"mid","width":300},{"url":"tiny","width":64}]},"tracks":{"totalCount":14}}]}},
        {"releases":{"items":[]}},
        {"releases":{"items":[{"id":"53Kz","name":"COCOON","type":"EP","date":{"year":2026},"coverArt":{"sources":[]}}]}}]}}}}}"#;
    let (r, read, total) = discography(json).unwrap();
    assert_eq!((r.len(), read, total), (2, 3, 56), "the empty entry gives no release but counts for the next page");
    assert_eq!((r[0].kind, r[0].cover.as_deref(), r[0].year, r[0].day), ("ALBUM", Some("mid"), 2026, 1_792_713_600_000));
    assert_eq!((r[1].name.as_str(), r[1].kind, r[1].cover.as_deref()), ("COCOON", "EP", None));
}

/// A picture with a name under it that opens a page: a related artist, an artist or album found by search.
#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    /// The dim line under the name ("2026 · Yeat"); may be empty.
    pub note: String,
}

/// Besides songs, what a search finds.
#[derive(Default, Clone)]
pub struct Found {
    pub artists: Vec<Card>,
    pub albums: Vec<Card>,
    pub playlists: Vec<Playlist>,
}

/// Search through the web player's own query: 50 songs and 50 each of artists, albums and playlists in one
/// request (the Web API gives developer apps pages of 10, songs only).
pub async fn web_search(session: &Session, http: &reqwest::Client, query: &str) -> Option<(Vec<Track>, Found)> {
    let vars = serde_json::json!({ "searchTerm": query, "offset": 0, "limit": 50, "numberOfTopResults": 5, "includeAudiobooks": false,
        "includeArtistHasConcertsField": false, "includePreReleases": false, "includeLocalConcertsField": false, "includeAuthors": false });
    search_answer(&web_query(session, http, "searchDesktop", "21969b655b795601fb2d2204a4243188e75fdc6d3520e7b9cd3f4db2aff9591e", vars).await?)
}

/// Songs and the rest out of a `searchDesktop` answer. Songs Spotify marks unplayable are left out.
fn search_answer(json: &[u8]) -> Option<(Vec<Track>, Found)> {
    #[derive(Deserialize)]
    struct Answer { data: Data }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Data { search_v2: Search }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Search { tracks_v2: Items<Item>, artists: Items<Wrap>, albums: Items<Wrap>, playlists: Items<Wrap> }
    #[derive(Deserialize)]
    struct Items<T> { #[serde(default = "Vec::new")] items: Vec<T> }
    #[derive(Deserialize)]
    struct Item { item: Wrap }
    // Every result comes as `{"data": {...}}`; one that isn't a song, artist, … (another `__typename`) won't parse and is skipped.
    #[derive(Deserialize)]
    struct Wrap { data: serde_json::Value }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Song { id: String, name: String, album_of_track: Album, artists: Items<Who>, duration: Duration, playability: Option<Playable> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Duration { total_milliseconds: u32 }
    #[derive(Deserialize)]
    struct Playable { playable: bool }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Album { uri: String, name: String, #[serde(default)] cover_art: Pics, #[serde(default)] artists: Option<Items<Who>>, date: Option<Year> }
    #[derive(Deserialize)]
    struct Year { year: i32 }
    #[derive(Deserialize)]
    struct Who { uri: String, profile: Name }
    #[derive(Deserialize)]
    struct Name { name: String }
    #[derive(Deserialize)]
    struct Artist { uri: String, profile: Name, visuals: Option<Visuals> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Visuals { avatar_image: Option<Pics> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pl { uri: String, name: String, images: Option<Items<Pics>>, owner_v2: Option<Wrap> }
    #[derive(Deserialize)]
    struct Owner2 { name: Option<String>, username: Option<String> }
    fn parse<T: DeserializeOwned>(items: Vec<Wrap>) -> impl Iterator<Item = T> {
        items.into_iter().filter_map(|i| serde_json::from_value(i.data).ok())
    }
    let id = |uri: &str| uri.rsplit(':').next().unwrap_or_default().to_string();
    let by = |w: Items<Who>| w.items.into_iter().map(|a| Named { id: Some(id(&a.uri)), name: a.profile.name }).collect::<Vec<_>>();
    let s = serde_json::from_slice::<Answer>(json).ok()?.data.search_v2;
    let tracks = parse(s.tracks_v2.items.into_iter().map(|w| w.item).collect()).filter(|t: &Song| t.playability.as_ref().is_none_or(|p| p.playable)).map(|t| {
        // Biggest picture first, as the Web API lists them (the views take the second for a mid size).
        let mut pics = t.album_of_track.cover_art.sources;
        pics.sort_by_key(|p| std::cmp::Reverse(p.width));
        Track {
            id: Some(t.id), name: t.name, artists: by(t.artists), duration_ms: t.duration.total_milliseconds, kind: "track".into(),
            album: crate::api::Album { id: Some(id(&t.album_of_track.uri)), name: t.album_of_track.name, images: pics.into_iter().map(|p| Image { url: p.url }).collect(), ..Default::default() },
            ..Default::default()
        }
    }).collect();
    let artists = parse(s.artists.items).map(|a: Artist| Card {
        image: a.visuals.and_then(|v| v.avatar_image).as_ref().and_then(pick), id: id(&a.uri), name: a.profile.name, note: String::new(),
    }).collect();
    let albums = parse(s.albums.items).map(|a: Album| {
        let names = a.artists.map(by).unwrap_or_default().into_iter().map(|n| n.name).collect::<Vec<_>>().join(", ");
        let note = match a.date { Some(d) if names.is_empty() => d.year.to_string(), Some(d) => format!("{} · {names}", d.year), None => names };
        Card { image: pick(&a.cover_art), id: id(&a.uri), name: a.name, note }
    }).collect();
    let playlists = parse(s.playlists.items).map(|p: Pl| {
        let owner = p.owner_v2.and_then(|o| serde_json::from_value::<Owner2>(o.data).ok());
        Playlist {
            id: id(&p.uri), name: p.name, collaborative: false,
            images: p.images.and_then(|i| i.items.into_iter().next()).and_then(|i| pick(&i)).map(|url| vec![Image { url }]),
            owner: owner.map(|o| Owner { id: o.username, display_name: o.name }),
        }
    }).collect();
    Some((tracks, Found { artists, albums, playlists }))
}

#[test]
fn reads_a_search() {
    let json = br#"{"data":{"searchV2":{
        "tracksV2":{"items":[
            {"item":{"data":{"__typename":"Track","id":"4Se3","name":"Money so big","albumOfTrack":{"uri":"spotify:album:3ZKE","name":"Up 2 Me",
                "coverArt":{"sources":[{"url":"s300","width":300},{"url":"s64","width":64},{"url":"s640","width":640}]}},
                "artists":{"items":[{"uri":"spotify:artist:3qiH","profile":{"name":"Yeat"}}]},"duration":{"totalMilliseconds":160052},"playability":{"playable":true}}}},
            {"item":{"data":{"__typename":"Track","id":"gone","name":"Gone","albumOfTrack":{"uri":"spotify:album:x","name":"x"},
                "artists":{"items":[]},"duration":{"totalMilliseconds":1},"playability":{"playable":false}}}},
            {"item":{"data":{"__typename":"NotFound"}}}]},
        "artists":{"items":[{"data":{"__typename":"Artist","uri":"spotify:artist:3qiH","profile":{"name":"Yeat"},"visuals":{"avatarImage":null}}}]},
        "albums":{"items":[{"data":{"__typename":"Album","uri":"spotify:album:53Kz","name":"COCOON","artists":{"items":[{"uri":"spotify:artist:3qiH","profile":{"name":"Yeat"}}]},
            "coverArt":{"sources":[{"url":"c300","width":300}]},"date":{"year":2026}}}]},
        "playlists":{"items":[{"data":{"__typename":"Playlist","uri":"spotify:playlist:33ei","name":"Prime yeat",
            "images":{"items":[{"sources":[{"url":"p","width":null}]}]},"ownerV2":{"data":{"__typename":"User","name":"Mikysek","username":"a6y5"}}}}]}}}}"#;
    let (tracks, found) = search_answer(json).unwrap();
    assert_eq!(tracks.len(), 1, "the unplayable song and the odd item drop out");
    let t = &tracks[0];
    assert_eq!((t.id.as_deref(), t.artist_line(), t.duration_ms, t.album.id.as_deref()), (Some("4Se3"), "Yeat".to_string(), 160052, Some("3ZKE")));
    assert_eq!(t.album.images.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(), ["s640", "s300", "s64"]);
    assert_eq!((found.artists[0].id.as_str(), found.artists[0].image.as_deref()), ("3qiH", None));
    assert_eq!((found.albums[0].name.as_str(), found.albums[0].note.as_str()), ("COCOON", "2026 · Yeat"));
    let p = &found.playlists[0];
    assert_eq!((p.id.as_str(), p.owner.as_ref().and_then(|o| o.display_name.as_deref()), p.web_readable("a6y5")), ("33ei", Some("Mikysek"), true));
}

/// An upcoming show, as the overview lists it.
pub struct Show {
    /// "30 Oct", the day where it's played.
    pub day: String,
    pub year: i32,
    pub venue: String,
    pub city: String,
    /// Who plays (a festival's name for a festival).
    pub title: String,
    /// Spotify counts it as near you.
    pub near: bool,
}

/// What an artist page gets from the web player's overview query.
pub struct Overview {
    pub listeners: u64,
    pub followers: u64,
    /// "Fans also like".
    pub related: Vec<Card>,
    /// The next shows (the first 10, by date), near-you ones first, and how many there are in all.
    pub shows: Vec<Show>,
    pub shows_total: usize,
}

/// An artist's monthly listeners, followers, related artists and upcoming shows, from the web player's overview
/// query (the session has none of them: its related list is empty, and the Web API doesn't count listeners).
pub async fn artist_overview(session: &Session, http: &reqwest::Client, id: &str) -> Option<Overview> {
    let vars = serde_json::json!({ "uri": format!("spotify:artist:{id}"), "locale": "", "includePrerelease": false });
    overview(&web_query(session, http, "queryArtistOverview", "35648a112beb1794e39ab931365f6ae4a8d45e65396d641eeda94e4003d41497", vars).await?)
}

/// An `Overview` out of a `queryArtistOverview` answer.
fn overview(json: &[u8]) -> Option<Overview> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Union { stats: Stats, related_content: Option<Content>, goods: Option<Goods> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Stats { monthly_listeners: u64, followers: u64 }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Content { related_artists: Items<Item> }
    #[derive(Deserialize)]
    struct Items<T> { items: Vec<T> }
    #[derive(Deserialize)]
    struct Item { id: String, profile: Profile, visuals: Option<Visuals> }
    #[derive(Deserialize)]
    struct Profile { name: String }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Visuals { avatar_image: Option<Pics> }
    #[derive(Deserialize)]
    struct Goods { events: Option<Events> }
    #[derive(Deserialize)]
    struct Events { concerts: Option<Concerts> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Concerts { total_count: usize, items: Vec<Concert> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Concert { title: String, date: Date, venue: Venue, #[serde(default)] near_user: bool }
    #[derive(Deserialize)]
    struct Date { year: i32, month: usize, day: u32 }
    #[derive(Deserialize)]
    struct Venue { name: String, location: Option<Profile> }
    let u = serde_json::from_slice::<Answer<Union>>(json).ok()?.data.artist_union;
    let related = u.related_content.map_or(vec![], |c| c.related_artists.items).into_iter().map(|a| Card {
        image: a.visuals.and_then(|v| v.avatar_image).as_ref().and_then(pick),
        id: a.id, name: a.profile.name, note: String::new(),
    }).collect();
    let concerts = u.goods.and_then(|g| g.events).and_then(|e| e.concerts);
    let shows_total = concerts.as_ref().map_or(0, |c| c.total_count);
    let mut shows: Vec<Show> = concerts.map_or(vec![], |c| c.items).into_iter().map(|c| Show {
        day: format!("{} {}", c.date.day, MONTHS.get(c.date.month.wrapping_sub(1)).unwrap_or(&"")),
        year: c.date.year,
        venue: c.venue.name,
        city: c.venue.location.map_or(String::new(), |l| l.name),
        title: c.title,
        near: c.near_user,
    }).collect();
    shows.sort_by_key(|s| !s.near); // stable: by date within each
    let shows_total = shows_total.max(shows.len());
    Some(Overview { listeners: u.stats.monthly_listeners, followers: u.stats.followers, related, shows, shows_total })
}

#[test]
fn reads_an_overview() {
    let json = br#"{"data":{"artistUnion":{"__typename":"Artist","stats":{"followers":6696904,"monthlyListeners":18208296,"worldRank":0},
        "relatedContent":{"relatedArtists":{"totalCount":2,"items":[
            {"id":"3gBZ","profile":{"name":"Ken Carson"},"visuals":{"avatarImage":{"sources":[{"url":"a640","width":640},{"url":"a320","width":320}]}}},
            {"id":"699O","profile":{"name":"Playboi Carti"},"visuals":{"avatarImage":null}}]}},
        "goods":{"events":{"concerts":{"totalCount":29,"items":[
            {"title":"Yeat, Destroy Lonely","date":{"year":2026,"month":10,"day":30},"venue":{"name":"Thomas & Mack Center","location":{"name":"Las Vegas"}},"nearUser":false},
            {"title":"Yeat","date":{"year":2026,"month":11,"day":14},"venue":{"name":"Palladium","location":{"name":"Koeln"}},"nearUser":true}]}}}}}}"#;
    let o = overview(json).unwrap();
    assert_eq!((o.listeners, o.followers, o.related.len()), (18208296, 6696904, 2));
    assert_eq!((o.related[0].name.as_str(), o.related[0].image.as_deref()), ("Ken Carson", Some("a320")));
    assert_eq!(o.related[1].image, None);
    // The show near you first, then by date; the total counts the ones not listed.
    assert_eq!(o.shows.iter().map(|s| (s.day.as_str(), s.city.as_str())).collect::<Vec<_>>(), [("14 Nov", "Koeln"), ("30 Oct", "Las Vegas")]);
    assert_eq!(o.shows_total, 29);
    let none = overview(br#"{"data":{"artistUnion":{"stats":{"followers":1,"monthlyListeners":2}}}}"#).unwrap();
    assert!(none.shows.is_empty() && none.related.is_empty());
    assert!(overview(br#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#).is_none());
}

#[test]
#[ignore = "live: needs ~/.resona/credentials.json and the network (cargo test -- --ignored)"]
fn artist_page_lists_releases_newest_first() {
    use librespot::core::{cache::Cache, config::SessionConfig};
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cache = Cache::new(Some(std::env::home_dir().unwrap().join(".resona")), None, None, None).unwrap();
        let creds = cache.credentials().expect("signed in");
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(creds, true).await.unwrap();
        let http = reqwest::Client::new();
        let (_, popularity, top, releases, latest) = session_artist(&session, &http, "3qiHUAX7zY4Qnjx8TNUzVx").await.unwrap(); // Yeat
        let o = artist_overview(&session, &http, "3qiHUAX7zY4Qnjx8TNUzVx").await.expect("overview");
        eprintln!("popularity {popularity}, {} listeners, {} followers, {} releases, {} related, {} shows", o.listeners, o.followers, releases.len(), o.related.len(), o.shows_total);
        assert!(popularity > 0 && o.listeners > 0 && o.followers > 0 && !o.related.is_empty());
        let artist = metadata::Artist::get(&session, &SpotifyUri::from_uri("spotify:artist:3qiHUAX7zY4Qnjx8TNUzVx").unwrap()).await.unwrap();
        assert_eq!(releases.len(), session_releases(&session, &artist).await.0.len(), "the web query and the session see the same releases");
        for r in &releases { eprintln!("{} {} {}", r.year, r.kind, r.name) }
        for t in &latest { eprintln!("latest: {} ({})", t.name, t.album.name) }
        assert!(!top.is_empty() && !latest.is_empty());
        assert!(releases.windows(2).all(|w| w[0].year >= w[1].year));
    });
}

/// The artists you follow (ids), through the session (your profile's following list, which also holds
/// the people you follow: left out). Works through a Web API ban. Empty counts as no answer: a profile
/// that hides who it follows may answer that way, and the Web API then says for sure.
pub async fn session_following(session: &Session) -> Option<Vec<String>> {
    following_artists(&session.spclient().get_user_following(&session.username()).await.ok()?).filter(|a| !a.is_empty())
}

fn following_artists(json: &[u8]) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct Following { profiles: Vec<Profile> }
    #[derive(Deserialize)]
    struct Profile { uri: String }
    let f: Following = serde_json::from_slice(json).ok()?;
    Some(f.profiles.into_iter().filter_map(|p| p.uri.strip_prefix("spotify:artist:").map(String::from)).collect())
}

#[test]
fn following_keeps_artists_only() {
    let json = br#"{"profiles":[{"name":"1019","uri":"spotify:artist:3r82"},{"name":"a friend","uri":"spotify:user:corvin"},{"uri":"spotify:artist:78TT"}]}"#;
    assert_eq!(following_artists(json), Some(vec!["3r82".to_string(), "78TT".to_string()]));
}

/// The artists you follow (ids), through the Web API: the fallback when the session's list doesn't answer.
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

/// New releases by these artists (albums, EPs and singles out since `since`, ms since 1970): which ones from
/// the web player's discography query (one request per artist), else through the session, where each list is
/// read newest first only up to the first older one (a few requests per artist). Their songs come through the
/// session and go to `each` one release at a time, `added` set to the release day.
pub async fn session_new_releases(session: &Session, http: &reqwest::Client, artists: &[String], since: i64, mut each: impl FnMut(Vec<Track>) -> bool) {
    let mut seen = std::collections::HashSet::new(); // a release shared by two followed artists comes once
    for chunk in artists.chunks(8) {
        let mut set = tokio::task::JoinSet::new();
        for id in chunk {
            let (session, http, id) = (session.clone(), http.clone(), id.clone());
            set.spawn(async move { fresh_releases(&session, &http, &id, since).await });
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
async fn fresh_releases(session: &Session, http: &reqwest::Client, id: &str, since: i64) -> Vec<metadata::Album> {
    if let Some(releases) = web_releases(session, http, id, since).await {
        let mut out = vec![];
        for r in releases.iter().filter(|r| r.kind != "COMPILATION") {
            let Ok(u) = SpotifyUri::from_uri(&format!("spotify:album:{}", r.id)) else { continue };
            if let Ok(a) = patient(|| metadata::Album::get(session, &u)).await { out.push(a) } // unreadable just now: skip it
        }
        return out;
    }
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
    let d = album.date.as_utc();
    let released = release_day(d.year(), d.month() as u8, d.day());
    let by: Vec<Named> = album.artists.iter().map(|a| Named { id: a.id.to_id().ok(), name: a.name.clone() }).collect();
    let mut tracks = session_tracks(session, &uris).await;
    for t in &mut tracks { (t.album.released, t.album.by) = (released.clone(), by.clone()) }
    Ok(tracks)
}

/// Month names as shown in dates ("9 Aug 2024", "Fri 23 Oct").
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "9 Aug 2024". librespot turns a year-only date into 1 January, so that day shows as the year alone.
// ponytail: a real 1 January release shows as its year too; the date's precision is lost inside librespot.
fn release_day(year: i32, month: u8, day: u8) -> String {
    if (month, day) == (1, 1) { return year.to_string() }
    format!("{day} {} {year}", MONTHS[(month.clamp(1, 12) - 1) as usize])
}

#[test]
fn release_day_reads_like_a_date_and_a_bare_year_stays_bare() {
    assert_eq!(release_day(2024, 8, 9), "9 Aug 2024");
    assert_eq!(release_day(1999, 1, 1), "1999");
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





#[test]
#[ignore = "live: needs ~/.resona/credentials.json and the network (cargo test -- --ignored)"]
fn web_search_finds_more_than_a_web_api_page() {
    use librespot::core::{cache::Cache, config::SessionConfig};
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cache = Cache::new(Some(std::env::home_dir().unwrap().join(".resona")), None, None, None).unwrap();
        let creds = cache.credentials().expect("signed in");
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(creds, true).await.unwrap();
        let (tracks, found) = web_search(&session, &reqwest::Client::new(), "yeat").await.expect("search answers");
        eprintln!("{} songs, {} artists, {} albums, {} playlists", tracks.len(), found.artists.len(), found.albums.len(), found.playlists.len());
        assert!(tracks.len() > 10 && !found.artists.is_empty() && !found.albums.is_empty());
        // A found playlist someone else made opens through the session.
        let p = found.playlists.iter().find(|p| !p.web_readable(&session.username())).expect("someone else's playlist");
        let mut got = 0;
        session_playlist(&session, &p.id, crate::state::Sort::List, |t, _| { got += t.len(); true }).await.unwrap();
        eprintln!("\"{}\": {got} songs", p.name);
        assert!(got > 0);
    });
}
