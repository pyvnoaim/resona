# Resona

A small native Spotify player for macOS: Rust, egui/eframe for the UI, librespot for sign-in and playback.
Three skins, each with a dark and a light palette: **Wired** (Lain-style cyber/pixel, the default), **Swiss**
(print grid, huge type, one red rule) and **ASCII** (a teletype printout: every widget is 7-bit characters, Courier).
Others (Y2K, macOS, Phosphor, Brutal) were tried and dropped.

## Build and run

```sh
cargo build --release && ./target/release/resona   # use release: debug decodes audio slowly
cargo test
cargo clippy --all-targets                            # keep it warning-free
./bundle.sh                                           # Resona.app (icon in your accent, Info.plist) into /Applications
./target/release/resona cli                           # the same player full-screen in the terminal
```

Needs Spotify Premium (librespot playback). Everything the app keeps lives in `~/.resona/`:

| file | what |
|---|---|
| `client_id` | the client ID of the user's own Spotify developer app (asked for on first launch; `RESONA_CLIENT_ID` overrides) |
| `credentials.json` | librespot sign-in |
| `web_refresh_token` | Web API sign-in (mode 0600) |
| `settings` | `key=value`: skin, theme, accent (a name or `#rrggbb`), volume, last open list, `sort:<list>` (each list's order; newest added first by default), `cli_style`, `added_to` (playlists you last added to, first in "Add to playlist") |
| `unplayable` | track ids Spotify refused to play, one per line |
| `following` | followed artist ids, kept from the last good Web API read: New releases falls back to it during a ban |
| `liked` | liked track ids, one per line: the hearts at startup, refreshed through the session on every launch |
| `lists/` | each playlist and Liked Songs as last loaded, with its version (`api::list_version`: a playlist's revision, a hash of Liked Songs' ids); opening a list asks only the version and shows this copy while it matches |
| `audio/` | librespot's cache of songs played lately, up to 1 GB, oldest dropped first |

Delete the first two to sign in again (and `liked`, `following` and `lists/` too when switching accounts: they hold that account's hearts, artists and lists).

## Layout

| file | what |
|---|---|
| `src/main.rs` | `App` (UI state), startup, `open_list()` (load a list), `command()` (all playback), the frame loop |
| `src/cli.rs` | `resona cli`: full-screen terminal player (crossterm, drawn by hand; `t` switches a flow-style card and a devboard-style board) on the same engine and the same free functions (`open_list`, `command`, `fetch_lyrics`, `add_to`, …); the songs box can hold the keys (`?`), the playlist picker (`p`) or the lyrics (`v`) instead (`Tui::instead`) |
| `src/state.rs` | `State` and the queue: `Source`, `Cmd`, `Repeat`, `start` / `step` / `enqueue` / `set_shuffle` |
| `src/engine.rs` | librespot: `connect()` (sign-in, session, player, event loop), liked-songs and playlist-contents loaders |
| `src/api.rs` | Web API (`get`, `WebAuth`, search, like, create and add to playlist) and the session loaders (`session_playlist`, `session_artist`, `session_album`) |
| `src/ui.rs` | the views: sidebar, track list, player bar, focus view, help overlay, sign-in screen |
| `src/controls.rs` | input: keyboard, macOS menu bar (`muda`), media keys / Now Playing (`souvlaki`) |
| `src/skin.rs` | how things look: `Skin`, `Look` (sizes), `Pal` (colours), fonts, and the drawn widgets |
| `src/settings.rs` | reading and writing `~/.resona/settings` and `unplayable` |
| `assets/` | bundled OFL fonts + licenses: DotGothic16 + VT323 (Wired), Archivo regular + extra-bold (Swiss), Courier Prime regular + bold (ASCII) |

The Archivo files are static instances baked from the variable font with fontTools: egui only reads a variable
font's default instance.

## Things that are not obvious

- **Two client IDs.** Playback signs in with librespot's shared client ID (`CLIENT_ID` in engine.rs). The Web API
  uses each user's own developer app (`api::client_id`, from `~/.resona/client_id`): the shared ID gets 24h
  rate-limit bans there, and its old keymaster token endpoint answers 503. No ID is built in: without one the
  window shows a setup screen (`ui::setup`) and the CLI asks in the terminal; nothing connects until it's saved. Both sign-ins use `REDIRECT` (`http://127.0.0.1:8898/login`),
  which must be listed in the developer app's settings. No client secret is needed (PKCE). Adding a Web API
  scope to `SCOPES` forces one new browser sign-in.
- **The Web API is restricted for developer apps**, so several things go through the librespot session instead:
  playlists you don't own (`Playlist::web_readable` decides), artist pages and album pages. Liked hearts come from
  `~/.resona/liked` at once, then fresh on every launch from `load_liked`: Liked Songs resolved as a playback
  context (`spotify:user:<id>:collection`), every id in one session request. The old "are these liked?" lookup
  (`/me/tracks/contains`) was 403; its replacement, `GET /me/library/contains`, is untried.
- **Sort:** songs keep when they were added (`Track.added`, never shown) and their list position (`pos`); every
  loader hands songs to `State::add_tracks`, which numbers and sorts them. Lists without dates ignore the sort.
- **Refused songs:** librespot reports `Unavailable` for failed *preloads* too; only a failure of the playing track
  counts, and any failed load (a lost audio key, a CDN hiccup) comes that way too, so `api::refused` asks the
  metadata first: only a song with no files and no stand-in is marked; a passing failure gets one more try.
  Refused songs are skipped in the direction you were going and faded in lists; clicking one retries it.
- **One way to drive playback:** buttons, media keys, the menu bar and the keyboard all call `command()` (the CLI too).
  The menu bar owns the ⌘ shortcuts (`keys()` skips them when the menu exists, or they'd fire twice). Plain-key
  shortcuts (Space, S, R, …) are only named in menu titles: as menu key equivalents they'd fire while typing.
- **One cursor:** `App.pane` / `side` / `row` is the highlighted row. Arrows (J/K), PgUp/PgDn, Home/End move it,
  Tab switches sidebar and songs, Enter opens or plays; moving the mouse over a row moves it too, so hover and
  keyboard never disagree and Q / L act on that one row. `raw_input_hook` keeps Tab from egui: a widget it
  focused would swallow the plain-key shortcuts.
- **Mouse cursor:** `show_cursor` sets the `NSCursor` itself each frame; the hand egui asks for through winit's
  cursor rects did not show on macOS 27.
- **Queue:** `queue` is the list being played (shuffled or not), `base` its original order, `queued` the hand-added
  songs right after `qi`. `Source::Queue` is drawn straight from the queue, not loaded. Shuffle is our own
  (`spread_shuffle`): each artist's songs are spaced evenly through the queue; switched on with nothing playing,
  it starts the open list.
- **Rate limits:** `api::get` retries short `retry-after` waits and turns anything over 60s into an error.
  librespot has its own limiter too: 300 session requests per 30 s, then 10/s, shared with playback. Session
  loaders go through `api::patient` (waits it out) and `session_playlist` paces itself past 200 songs; a starved
  player reports a playable song as `Unavailable`.
- **Spotify's February 2026 Web API changes:** saving and liking go through `/me/library` (by URI), playlist
  contents through `/playlists/{id}/items`, new playlists through `POST /me/playlists`, search pages are 10. Check the changelog before adding an endpoint.
- **Session playlists** are read page by page (`/playlist/v2/playlist/{id}?from=&length=`): librespot's
  `Playlist::get` returns only the first part of a long list.
  Developer apps get day-long bans for bursts, so startup stays small: playlists and Liked Songs only; what's in
  each playlist (`load_members`) waits for the first "Add to playlist" menu. A failed library load is shown, not
  fatal: playback runs on librespot and keeps working through a ban, and the sidebar falls back to the session's
  rootlist (`session_playlists`: only uploaded covers, mosaics are rebuilt from each list's first albums by `session_mosaic`; every list then opens through the session).
- **Song radio** (`Source::Radio`): the session's `inspiredby-mix` endpoint turns a song into Spotify's radio
  playlist (`api::radio_playlist`), which then opens like any session playlist. The Web API's recommendations are
  closed to developer apps.
- **New releases** (`Source::NewReleases`): followed artists from the Web API (`GET /me/following`, scope
  `user-follow-read`; the session has no such list), then each artist's albums and singles through the session,
  read newest first only until one is older than two weeks (`session_new_releases`). `Track.added` is the release day.
  It has no `key`, so it's never the list reopened at launch: that would be a burst at startup.
- **Search** asks for pages of 10 and stops at 5 pages; newer developer apps get a lower page-size cap.
- **Repaints:** nothing redraws when idle; 4 fps while playing, 20 fps while the equaliser bars or meter are on
  screen, ~16 fps while a list is loading.
- **Toasts:** `State::say` shows a short note above the player bar (3 s, errors 6 s), one at a time; `enqueue`
  announces itself. The CLI shows the same note in its status line while it lasts.
- **Accent colours** (`skin::Accent`): red is each skin's own; the others are swapped into every palette by
  `tinted` (text on the accent goes black or white), all built once in `pal_of`. `Accent::Custom` is your own
  colour (saved as `accent=#rrggbb`): each skin and theme shows `readable`'s shade of it (pulled toward black or
  white until it stands out 3:1), made on first use in `custom_pal`. In the skin menu, hovering a swatch only
  previews it (`peek_accent`); `kept_accent` is what's saved and what the Dock icon follows. The app icon is drawn in Rust
  (`skin::app_icon`): the Dock icon is repainted on change (`set_app_icon`, AppKit; eframe sets one only at
  startup), and `bundle.sh` takes the icon file's pixels from `resona icon`. The CLI reads the accent at start.
- **Other systems:** the menu bar (`muda`), the cursor fix and the Dock icon are macOS-only (`cfg(target_os =
  "macos")`, and macOS-only dependencies); elsewhere the keyboard handles the Ctrl shortcuts, winit the cursor,
  and `set_app_icon` sets the window icon. Shortcut labels go through `ui::cmd_keys` (⌘ → Ctrl). Media keys:
  macOS and Linux (MPRIS over zbus), in the window and the CLI (`controls::media_controls`, shared; the CLI turns
  the main run loop itself, where macOS delivers them); Windows needs an HWND, not wired yet. `.github/workflows/build.yml` builds
  all three on every push: the only check that Windows and Linux still compile.
- Errors are stored raw in `State.error` (or an error toast's `Notice`) and shown through `short_err()` (full text on hover).

## Conventions

- Keep it small: stdlib and existing deps first, no new abstractions without a second user.
- Mark deliberate shortcuts with a `// ponytail:` comment that names the limit and the upgrade path.
- Non-trivial logic gets one small `#[test]` next to it.
- Every skin must keep working: anything drawn goes through `skin.rs`, and `skin::every_skin_lays_out_text_in_every_font`
  catches a font that fails to load. ASCII draws its frames, bars and covers as text (`char_box`, `char_bar`,
  `char_cover`: the real art turned into characters; `char_scroll` letters over egui's unseen scrollbar) and never shows an image.
