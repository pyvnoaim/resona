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

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// How the open list is ordered: as Spotify keeps it, or by when songs were added. Lists without dates
/// (search, albums, artists) look the same in every order.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Sort {
    #[default]
    List,
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
        match k { "newest" => Sort::Newest, "oldest" => Sort::Oldest, _ => Sort::List }
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
    /// Set once `members` has been asked for (the first time the menu opens).
    pub members_asked: bool,
    /// A short confirmation for the header ("Added to …"); cleared when another list opens.
    pub notice: Option<String>,
    /// Lyrics for one track id, for the big now-playing view.
    pub lyrics: Option<(String, Lyrics)>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub sort: Sort,
    /// The queue in its original order, to go back to when shuffle is switched off.
    pub base: Vec<api::Track>,
    /// How many tracks right after `qi` were added by hand ("Add to queue" / "Play next").
    pub queued: usize,
    /// The list the queue came from, for "jump to the playing song".
    pub queue_src: Option<Source>,
    /// Portrait and releases when an artist page is open.
    pub artist: Option<ArtistPage>,
}

pub enum Lyrics {
    Loading,
    /// (start ms, words); `synced` = the start times are real.
    Lines { synced: bool, lines: Vec<(u32, String)> },
    Missing,
}

impl State {
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
            shuffle(&mut self.queue, seed());
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
            shuffle(&mut rest, seed());
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

    /// Songs arriving for the open list (a page, or all of them): numbered in list order, then sorted.
    pub fn add_tracks(&mut self, new: Vec<api::Track>) {
        let at = self.tracks.len();
        self.tracks.extend(new.into_iter().enumerate().map(|(i, t)| api::Track { pos: at + i, ..t }));
        self.sort_tracks();
    }

    /// The sort, named for the song count: only when it does something here (search, albums and artists
    /// have no dates, so every order looks the same).
    pub fn sort_label(&self) -> Option<&'static str> {
        if self.view == Some(Source::Queue) { return None } // drawn from the queue, never sorted
        self.sort.label().filter(|_| self.tracks.iter().any(|t| t.added > 0))
    }

    /// List order → newest added → oldest added, remembered for next time.
    pub fn cycle_sort(&mut self) {
        self.sort = match self.sort { Sort::List => Sort::Newest, Sort::Newest => Sort::Oldest, Sort::Oldest => Sort::List };
        crate::settings::set_setting("sort", self.sort.key());
        self.sort_tracks();
    }

    fn sort_tracks(&mut self) {
        // Stable, with list order breaking ties, so songs without dates keep their places.
        match self.sort {
            Sort::List => self.tracks.sort_by_key(|t| t.pos),
            Sort::Newest => self.tracks.sort_by_key(|t| (std::cmp::Reverse(t.added), t.pos)),
            Sort::Oldest => self.tracks.sort_by_key(|t| (t.added, t.pos)),
        }
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

pub fn seed() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    nanos | 1 // xorshift must not start at 0
}

/// An artist page's extras: the portrait and their albums / EPs / singles.
pub struct ArtistPage {
    pub portrait: Option<String>,
    pub releases: Vec<api::Release>,
}

#[test]
fn sorting_by_added_keeps_undated_songs_in_list_order_and_comes_back() {
    let t = |name: &str, added| api::Track { name: name.into(), added, ..Default::default() };
    let mut s = State::default();
    s.add_tracks(vec![t("b", 20), t("a", 10), t("c", 30)]);
    let names = |s: &State| s.tracks.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join("");
    s.sort = Sort::Oldest; s.sort_tracks();
    assert_eq!(names(&s), "abc");
    s.sort = Sort::Newest; s.sort_tracks();
    assert_eq!(names(&s), "cba");
    s.add_tracks(vec![t("x", 0), t("y", 0)]); // a later page, no dates: after the dated ones, in order
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
