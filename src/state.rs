//! Playback and library state: what is open, what is queued, and how the queue moves.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use librespot::{
    core::{SpotifyUri, spotify_id::SpotifyId},
    playback::player::Player,
};

use crate::api;

/// Accepts a bare ID, spotify:track:ID, or an open.spotify.com/track/ID?si=… link.
pub fn track_id(arg: &str) -> &str {
    let tail = arg.trim().rsplit(['/', ':']).next().unwrap();
    tail.split('?').next().unwrap()
}

#[derive(Clone, PartialEq)]
pub enum Source {
    Liked,
    Playlist(String),
    /// Spotify-wide track search for this query.
    Search(String),
    /// What's coming up: the rest of the play queue.
    Queue,
    Artist { id: String, name: String },
    Album { id: String, name: String },
    /// Songs from the last two weeks' releases by the artists you follow.
    NewReleases,
    /// A song's radio: Spotify's mix of similar songs, read as the playlist it hands back.
    Radio { track: String, name: String },
}

impl Source {
    /// How it's remembered in ~/.resona/settings ("last=").
    pub fn key(&self) -> Option<String> {
        match self {
            Source::Liked => Some("liked".into()),
            Source::Playlist(id) => Some(format!("playlist:{id}")),
            Source::Search(q) => Some(format!("search:{q}")),
            _ => None,
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        match key.split_once(':') {
            None if key == "liked" => Some(Source::Liked),
            Some(("playlist", id)) => Some(Source::Playlist(id.into())),
            Some(("search", q)) => Some(Source::Search(q.into())),
            _ => None,
        }
    }
}

/// Which list the keyboard cursor is in: the sidebar or the open list's songs.
#[derive(Clone, Copy, PartialEq)]
pub enum Pane { Lists, Songs }

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// How the open list is ordered: as Spotify keeps it, or by when songs were added. Lists without dates
/// (search, albums, artists) look the same in every order. Each list keeps its own (`sort:<list key>` in settings).
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Sort {
    List,
    #[default]
    Newest,
    Oldest,
}

impl Sort {
    /// How it's named next to the song count; nothing for the list's own order.
    pub fn label(self) -> Option<&'static str> {
        match self { Sort::List => None, Sort::Newest => Some("newest added first"), Sort::Oldest => Some("oldest added first") }
    }
    pub fn key(self) -> &'static str {
        match self { Sort::List => "list", Sort::Newest => "newest", Sort::Oldest => "oldest" }
    }
    pub fn from_key(k: &str) -> Self {
        match k { "list" => Sort::List, "oldest" => Sort::Oldest, _ => Sort::Newest }
    }
    /// Puts (added, pos) pairs in this order, newest first by default; ties keep list order.
    pub fn order<T>(self, v: &mut [T], key: impl Fn(&T) -> (i64, usize)) {
        match self {
            Sort::List => v.sort_by_key(|x| key(x).1),
            Sort::Newest => v.sort_by_key(|x| (std::cmp::Reverse(key(x).0), key(x).1)),
            Sort::Oldest => v.sort_by_key(|x| key(x)),
        }
    }
    /// The order this list was last left in; newest added first for a list never sorted.
    pub fn saved(src: &Source) -> Self {
        src.key().and_then(|k| crate::settings::setting(&format!("sort:{k}"))).map(|v| Self::from_key(&v)).unwrap_or_default()
    }
}

/// Everything that drives playback: buttons, media keys, keyboard.
#[derive(Clone, Copy)]
pub enum Cmd {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    SeekTo(u32),
    SeekBy(i64),
    /// Shuffle on / off; switched on with nothing playing, it starts the open list at a random song.
    Shuffle,
}

/// A toast: what happened (an error raw, shown through `short_err`), whether it went wrong, and when it goes away.
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub until: Instant,
    pub life: std::time::Duration,
}

#[derive(Default)]
pub struct State {
    pub status: String,
    /// The last failure, raw. Shown as a short line, full text on hover.
    pub error: Option<String>,
    pub playlists: Vec<api::Playlist>,
    pub view: Option<Source>,
    pub tracks: Vec<api::Track>,
    pub loading: bool,
    /// Counts list opens, so a slow load can tell it was replaced by a newer one of the same list.
    pub load_id: u64,
    /// How many tracks the open list will have, once Spotify says.
    pub expected: Option<usize>,
    pub queue: Vec<api::Track>,
    pub qi: usize,
    pub now: Option<api::Track>,
    pub pos_ms: u32,
    /// Set while playing: position = pos_ms + elapsed.
    pub since: Option<Instant>,
    /// Track ids known to be in Liked Songs (checked as lists load).
    pub liked: HashSet<String>,
    /// `liked` holds all of Liked Songs (from ~/.resona/liked or a full read), so it may be saved.
    pub liked_complete: bool,
    /// Counts likes and unlikes, so a Liked Songs load that raced one doesn't undo it.
    pub liked_edits: u32,
    /// Tracks Spotify refused to play this session; skipped and greyed out.
    pub unplayable: HashSet<String>,
    /// Which way we were moving through the queue (1 or -1), so a refused track is skipped the same way.
    pub dir: isize,
    /// Track ids in each of your own playlists (by playlist id), for the "Add to playlist" menu.
    pub members: HashMap<String, HashSet<String>>,
    /// Playlists you last added songs to, latest first (`added_to` in settings): they lead "Add to playlist".
    pub added_to: Vec<String>,
    /// Set once `members` has been asked for (the first time the menu opens).
    pub members_asked: bool,
    /// A short note shown as a toast ("Added to …"), until it runs out or is clicked away.
    pub notice: Option<Notice>,
    /// A newer release's tag (`update::check`), offered in a dialog until it's taken or put off.
    pub update: Option<String>,
    /// Lyrics for one track id, for the big now-playing view.
    pub lyrics: Option<(String, Lyrics)>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub sort: Sort,
    /// The queue in its original order, to go back to when shuffle is switched off.
    pub base: Vec<api::Track>,
    /// How many tracks right after `qi` were added by hand ("Add to queue" / "Play next").
    pub queued: usize,
    /// What the queue view was last built from (`sync_queue_view`).
    queue_seen: Option<(usize, usize, usize, bool)>,
    /// The list the queue came from, for "jump to the playing song".
    pub queue_src: Option<Source>,
    /// Portrait and releases when an artist page is open.
    pub artist: Option<ArtistPage>,
    /// A search's artists, albums and playlists (its songs are `tracks`), when the web player's search answered.
    pub found: Option<api::Found>,
    /// The playlist last opened from search results: not in the sidebar, so its name and owner come from here.
    pub visiting: Option<api::Playlist>,
    /// The last search the web player answered (query, when, songs, the rest): going back to it within 10
    /// minutes shows these again.
    pub searched: Option<(String, Instant, Vec<api::Track>, api::Found)>,
}

pub enum Lyrics {
    Loading,
    /// (start ms, words); `synced` = the start times are real.
    Lines { synced: bool, lines: Vec<(u32, String)> },
    Missing,
}

impl State {
    /// A playlist by id: one of yours from the sidebar, or the one opened from search.
    pub fn playlist(&self, id: &str) -> Option<&api::Playlist> {
        self.playlists.iter().find(|p| p.id == id).or(self.visiting.as_ref().filter(|p| p.id == id))
    }

    /// Shows `text` as a toast: 3 s, or 6 s for an error.
    pub fn say(&mut self, text: String, error: bool) {
        let life = std::time::Duration::from_secs(if error { 6 } else { 3 });
        self.notice = Some(Notice { text, error, until: Instant::now() + life, life });
    }

    /// Puts `playlist` first in "Add to playlist".
    pub fn remember_added_to(&mut self, playlist: &str) {
        self.added_to.retain(|p| p != playlist);
        self.added_to.insert(0, playlist.to_string());
        self.added_to.truncate(100);
    }

    /// Plays queue index `i` by skipping ahead to it (from the queue view or up next), using up hand-queued songs on
    /// the way.
    pub fn jump_to(&mut self, player: &Player, i: usize) {
        self.queued = self.queued.saturating_sub(i.saturating_sub(self.qi));
        self.dir = 1;
        self.play_at(player, i);
    }

    /// The library lists the queue while it has hand-queued songs, or while it's open.
    pub fn lists_queue(&self) -> bool {
        self.queued > 0 || self.view == Some(Source::Queue)
    }

    /// The queue view's songs: the playing one, then what's next. Rebuilt only when the queue moved.
    /// ponytail: a reorder that keeps the length, position and shuffle (none today) wouldn't show until the next move.
    pub fn sync_queue_view(&mut self) {
        if self.view != Some(Source::Queue) { return }
        let now = (self.qi, self.queue.len(), self.queued, self.shuffle);
        if self.queue_seen == Some(now) && !self.tracks.is_empty() { return }
        self.queue_seen = Some(now);
        self.tracks = self.queue.iter().skip(self.qi).cloned().collect();
    }

    /// The next `n` songs playback would actually reach, with their queue index: refused ones are skipped there,
    /// so they're left out here too.
    pub fn up_next(&self, n: usize) -> Vec<(usize, &api::Track)> {
        (self.qi + 1..)
            .zip(self.queue.iter().skip(self.qi + 1))
            .filter(|(_, t)| !t.id.as_ref().is_some_and(|id| self.unplayable.contains(id)))
            .take(n)
            .collect()
    }

    /// The toast, while it lasts.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref().filter(|n| n.until > Instant::now())
    }

    pub fn position(&self) -> u32 {
        let extra = self.since.map_or(0, |t| t.elapsed().as_millis() as u32);
        let dur = self.now.as_ref().map_or(1, |t| t.duration_ms.max(1));
        (self.pos_ms + extra).min(dur)
    }

    /// From queue[from], plays the next track in direction `dir` (1 / -1) that isn't known to be
    /// unplayable; with repeat-all it wraps around. Returns false when there's nothing to play.
    pub fn step(&mut self, player: &Player, from: usize, dir: isize) -> bool {
        self.dir = dir;
        let found = next_playable(&self.queue, &self.unplayable, from, dir)
            .or_else(|| (self.repeat == Repeat::All).then(|| first_playable(&self.queue, &self.unplayable, dir)).flatten());
        let Some(i) = found else { return false };
        // Hand-queued tracks sit right after `qi`; moving forward uses them up, moving back drops the mark.
        self.queued = if dir > 0 && i > from { self.queued.saturating_sub(i - from) } else { 0 };
        self.play_at(player, i)
    }

    /// Starts playing `list` from `at` (or its first track), remembering where it came from.
    /// Hand-queued tracks survive; with shuffle on, the rest is shuffled with the chosen track first.
    pub fn start(&mut self, player: &Player, list: Vec<api::Track>, at: Option<usize>, src: Option<Source>) {
        let kept = self.take_queued();
        self.base = list.clone();
        self.queue = list;
        self.queue_src = src;
        let mut first = at.unwrap_or(0);
        if self.shuffle {
            let pick = at.map(|i| self.queue.remove(i));
            spread_shuffle(&mut self.queue, seed());
            if let Some(t) = pick { self.queue.insert(0, t) }
            first = 0;
        }
        self.queue.splice(first + 1..first + 1, kept.iter().cloned());
        self.queued = kept.len();
        self.dir = 1;
        let skip_known_bad = at.is_none() && self.queue.get(first).and_then(|t| t.id.as_ref()).is_some_and(|id| self.unplayable.contains(id));
        if skip_known_bad || !self.play_at(player, first) {
            self.step(player, first, 1);
        }
    }

    /// The hand-queued tracks after the current one, removed from the queue.
    pub fn take_queued(&mut self) -> Vec<api::Track> {
        let from = (self.qi + 1).min(self.queue.len());
        let to = (from + self.queued).min(self.queue.len());
        self.queued = 0;
        self.queue.drain(from..to).collect()
    }

    /// Shuffle on: the current track stays, everything after it gets shuffled. Off: back to the
    /// original order, carrying on from the current track.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        let kept = self.take_queued();
        let now = self.now.as_ref().and_then(|t| t.id.clone());
        if on {
            let mut rest: Vec<api::Track> = self.base.iter().filter(|t| t.id != now).cloned().collect();
            spread_shuffle(&mut rest, seed());
            self.queue = self.now.iter().cloned().chain(rest).collect();
            self.qi = 0;
        } else {
            self.queue = self.base.clone();
            self.qi = self.queue.iter().position(|t| t.id == now).unwrap_or(0);
        }
        let at = (self.qi + 1).min(self.queue.len());
        self.queue.splice(at..at, kept.iter().cloned());
        self.queued = kept.len();
    }

    /// Songs arriving for the open list (a page, or all of them), each with its `pos` set by the loader:
    /// pages can arrive in any order (newest first loads the end of a list first).
    pub fn add_tracks(&mut self, new: Vec<api::Track>) {
        self.tracks.extend(new);
        self.sort_tracks();
    }

    /// The sort, named for the song count: only when it does something here (search, albums and artists
    /// have no dates, so every order looks the same).
    pub fn sort_label(&self) -> Option<&'static str> {
        if self.view == Some(Source::Queue) { return None } // drawn from the queue, never sorted
        self.sort.label().filter(|_| self.tracks.iter().any(|t| t.added > 0))
    }

    /// List order → newest added → oldest added, remembered for this list.
    pub fn cycle_sort(&mut self) {
        self.sort = match self.sort { Sort::List => Sort::Newest, Sort::Newest => Sort::Oldest, Sort::Oldest => Sort::List };
        // Searches have no dates to sort by, and a query can hold the `=` the settings file splits on.
        if let Some(k) = self.view.as_ref().filter(|v| !matches!(v, Source::Search(_))).and_then(Source::key) {
            crate::settings::set_setting(&format!("sort:{k}"), self.sort.key());
        }
        self.sort_tracks();
    }

    fn sort_tracks(&mut self) {
        // List order breaks ties, so songs without dates keep their places.
        self.sort.order(&mut self.tracks, |t| (t.added, t.pos));
    }

    pub fn toggle_shuffle(&mut self) {
        let on = !self.shuffle;
        self.set_shuffle(on);
    }

    /// Off → all → one → off.
    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat { Repeat::Off => Repeat::All, Repeat::All => Repeat::One, Repeat::One => Repeat::Off };
    }

    /// "Play next" (front of the hand queue) or "Add to queue" (its end). With nothing playing, it just plays.
    pub fn enqueue(&mut self, player: &Player, t: api::Track, next: bool) {
        self.say(format!("{} \u{201c}{}\u{201d}", if next { "Playing next:" } else { "Queued:" }, t.name), false);
        if self.now.is_none() {
            self.start(player, vec![t], None, None);
            return;
        }
        let at = (self.qi + 1 + if next { 0 } else { self.queued }).min(self.queue.len());
        self.queue.insert(at, t);
        self.queued += 1;
    }

    /// Loads queue[i] into the player. Returns false past either end.
    pub fn play_at(&mut self, player: &Player, i: usize) -> bool {
        let Some(t) = self.queue.get(i) else { return false };
        let Ok(id) = SpotifyId::from_base62(track_id(t.id.as_deref().unwrap_or_default())) else { return false };
        // ponytail: reloading the song that just ended makes librespot 0.8 seek its spent decoder, which can
        // leave the player dead (repeat one, a one-song repeat all); stopping first makes it load the song fresh.
        // Costs a refetch when restarting the playing song; drop it once librespot's same-track path is fixed.
        if self.now.as_ref().is_some_and(|n| n.id == t.id) { player.stop() }
        self.qi = i;
        self.now = Some(t.clone());
        (self.pos_ms, self.since) = (0, None);
        player.load(SpotifyUri::Track { id }, true, 0);
        true
    }
}

/// The first queue index after `from` in direction `dir` whose track isn't in `skip`.
pub fn next_playable(queue: &[api::Track], skip: &HashSet<String>, from: usize, dir: isize) -> Option<usize> {
    let mut i = from as isize + dir;
    while let Some(t) = usize::try_from(i).ok().and_then(|i| queue.get(i)) {
        if !t.id.as_ref().is_some_and(|id| skip.contains(id)) {
            return Some(i as usize);
        }
        i += dir;
    }
    None
}

/// The first playable index from one end of the queue (start for `dir` 1, end for -1).
pub fn first_playable(queue: &[api::Track], skip: &HashSet<String>, dir: isize) -> Option<usize> {
    let ok = |i: &usize| !queue[*i].id.as_ref().is_some_and(|id| skip.contains(id));
    if dir > 0 { (0..queue.len()).find(ok) } else { (0..queue.len()).rev().find(ok) }
}

/// Fisher-Yates with a small xorshift; good enough to shuffle a playlist, no crate needed.
pub fn shuffle<T>(v: &mut [T], mut seed: u64) {
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// Shuffle that keeps an artist's songs apart (like Spotify's): each artist's songs get evenly spaced
/// slots across the queue from a random start, nudged a little, and the queue is sorted by slot.
pub fn spread_shuffle(tracks: &mut Vec<api::Track>, mut seed: u64) {
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut by_artist: HashMap<String, Vec<api::Track>> = HashMap::new();
    for t in tracks.drain(..) {
        by_artist.entry(t.artists.first().map(|a| a.name.clone()).unwrap_or_default()).or_default().push(t);
    }
    let mut slotted = Vec::new();
    for (_, mut songs) in by_artist {
        shuffle(&mut songs, (rand() * u64::MAX as f64) as u64 | 1);
        let gap = 1.0 / songs.len() as f64;
        let start = rand() * gap;
        for (k, t) in songs.into_iter().enumerate() {
            slotted.push((start + k as f64 * gap + (rand() - 0.5) * gap * 0.2, t));
        }
    }
    slotted.sort_by(|a, b| a.0.total_cmp(&b.0));
    tracks.extend(slotted.into_iter().map(|(_, t)| t));
}

pub fn seed() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    nanos | 1 // xorshift must not start at 0
}

/// An artist page's extras: the portrait, their numbers and their albums / EPs / singles.
pub struct ArtistPage {
    pub portrait: Option<String>,
    /// Spotify's 0–100 score, not a count.
    pub popularity: u8,
    /// Monthly listeners, followers, related artists and shows; `None` when the web player's query didn't answer.
    pub overview: Option<api::Overview>,
    pub releases: Vec<api::Release>,
    /// How many of `State::tracks` are the top songs; the latest songs follow them.
    pub top: usize,
    /// Whether you follow them, as `~/.resona/following` last said.
    pub followed: bool,
}

#[test]
fn sorting_by_added_keeps_undated_songs_in_list_order_and_comes_back() {
    let t = |name: &str, added, pos| api::Track { name: name.into(), added, pos, ..Default::default() };
    let mut s = State::default();
    s.add_tracks(vec![t("b", 20, 0), t("a", 10, 1), t("c", 30, 2)]);
    let names = |s: &State| s.tracks.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join("");
    s.sort = Sort::Oldest; s.sort_tracks();
    assert_eq!(names(&s), "abc");
    s.sort = Sort::Newest; s.sort_tracks();
    assert_eq!(names(&s), "cba");
    s.add_tracks(vec![t("x", 0, 3), t("y", 0, 4)]); // a later page, no dates: after the dated ones, in order
    assert_eq!(names(&s), "cbaxy");
    s.sort = Sort::List; s.sort_tracks();
    assert_eq!(names(&s), "bacxy"); // the list's own order again
}

#[test]
pub fn skipping_passes_refused_tracks_in_both_directions() {
    let q: Vec<api::Track> = ["a", "b", "c", "d"].iter().map(|id| api::Track { id: Some(id.to_string()), ..Default::default() }).collect();
    let skip: HashSet<String> = ["b".to_string(), "c".to_string()].into();
    assert_eq!(next_playable(&q, &skip, 0, 1), Some(3));
    assert_eq!(next_playable(&q, &skip, 3, -1), Some(0));
    assert_eq!(next_playable(&q, &skip, 3, 1), None);
    assert_eq!(next_playable(&q, &skip, 0, -1), None);
}

#[test]
fn spread_shuffle_keeps_every_song_and_artists_apart() {
    let t = |artist: &str, pos| api::Track { artists: vec![api::Named { name: artist.into(), ..Default::default() }], pos, ..Default::default() };
    for s in 1..20 {
        let mut v: Vec<api::Track> = (0..30).map(|i| t(["a", "b", "c"][i % 3], i)).collect();
        spread_shuffle(&mut v, s * 7919);
        let together = v.windows(2).filter(|w| w[0].artists[0].name == w[1].artists[0].name).count();
        assert!(together <= 2, "seed {s}: {together} same-artist neighbours"); // plain shuffle averages ~9
        let mut pos: Vec<usize> = v.iter().map(|t| t.pos).collect();
        pos.sort();
        assert_eq!(pos, (0..30).collect::<Vec<_>>());
    }
}

#[test]
pub fn shuffle_keeps_every_track_and_lengths_read_well() {
    let mut v: Vec<u32> = (0..50).collect();
    shuffle(&mut v, 12345);
    assert_ne!(v, (0..50).collect::<Vec<_>>());
    v.sort();
    assert_eq!(v, (0..50).collect::<Vec<_>>());
    assert_eq!(crate::ui::total_length(33 * 60_000 + 59_000), "33 min");
    assert_eq!(crate::ui::total_length(252 * 60_000), "4 h 12 min");
}

#[test]
pub fn repeat_all_wraps_to_the_first_playable_track() {
    let q: Vec<api::Track> = ["a", "b", "c"].iter().map(|id| api::Track { id: Some(id.to_string()), ..Default::default() }).collect();
    let skip: HashSet<String> = ["a".to_string()].into();
    assert_eq!(first_playable(&q, &skip, 1), Some(1));
    assert_eq!(first_playable(&q, &skip, -1), Some(2));
}

#[test]
fn last_added_to_comes_first() {
    let mut s = State::default();
    for p in ["a", "b", "a", "c"] { s.remember_added_to(p) }
    assert_eq!(s.added_to, ["c", "a", "b"]);
}

#[test]
fn up_next_skips_refused_songs_and_keeps_queue_indices() {
    let queue = ["a", "b", "c", "d", "e"].iter().map(|id| api::Track { id: Some(id.to_string()), ..Default::default() }).collect();
    let s = State { queue, qi: 1, unplayable: ["c".to_string()].into(), ..Default::default() };
    let next: Vec<(usize, Option<&str>)> = s.up_next(5).into_iter().map(|(i, t)| (i, t.id.as_deref())).collect();
    assert_eq!(next, [(3, Some("d")), (4, Some("e"))]);
}

#[test]
fn queue_view_is_rebuilt_only_when_the_queue_moves() {
    let t = |id: &str| api::Track { id: Some(id.into()), ..Default::default() };
    let mut s = State { queue: vec![t("a"), t("b"), t("c")], view: Some(Source::Queue), ..Default::default() };
    s.sync_queue_view();
    assert_eq!(s.tracks.len(), 3);
    s.tracks[0].name = "kept".into();
    s.sync_queue_view(); // nothing moved: not copied again
    assert_eq!(s.tracks[0].name, "kept");
    s.qi = 1;
    s.sync_queue_view();
    assert_eq!(s.tracks.iter().map(|t| t.id.as_deref().unwrap()).collect::<Vec<_>>(), ["b", "c"]);
}
