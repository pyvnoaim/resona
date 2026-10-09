# Resona

A small native Spotify player for macOS: Rust, egui/eframe for the UI, librespot for sign-in and playback.
Two skins, each with a dark and a light palette: **Wired** (Lain-style cyber/pixel, the default) and **Swiss**
(print grid, huge type, one red rule). Others (Y2K, macOS, Phosphor, Brutal) were tried and dropped.

## Build and run

```sh
cargo build --release && ./target/release/resona   # use release: debug decodes audio slowly
cargo test
cargo clippy --all-targets                            # keep it warning-free
./bundle.sh                                           # Resona.app (icon, Info.plist) into /Applications
./target/release/resona cli                           # the same player full-screen in the terminal
```

Needs Spotify Premium (librespot playback). Everything the app keeps lives in `~/.resona/`:

| file | what |
|---|---|
| `credentials.json` | librespot sign-in |
| `web_refresh_token` | Web API sign-in (mode 0600) |
| `settings` | `key=value`: skin, theme, volume, last open list, `sort:<list>` (each list's order; newest added first by default), `cli_style` |
| `unplayable` | track ids Spotify refused to play, one per line |
| `liked` | liked track ids, one per line: the hearts at startup, refreshed through the session on every launch |

Delete the first two to sign in again (and `liked` too when switching accounts: it holds that account's hearts).

## Layout

| file | what |
|---|---|
| `src/main.rs` | `App` (UI state), startup, `open_list()` (load a list), `command()` (all playback), the frame loop |
| `src/cli.rs` | `resona cli`: full-screen terminal player (crossterm, drawn by hand; `t` switches a flow-style card and a devboard-style board) on the same engine, `open_list` and `command` |
| `src/state.rs` | `State` and the queue: `Source`, `Cmd`, `Repeat`, `start` / `step` / `enqueue` / `set_shuffle` |
| `src/engine.rs` | librespot: `connect()` (sign-in, session, player, event loop), liked-songs and playlist-contents loaders |
| `src/api.rs` | Web API (`get`, `WebAuth`, search, like, add to playlist) and the session loaders (`session_playlist`, `session_artist`, `session_album`) |
| `src/ui.rs` | the views: sidebar, track list, player bar, focus view, help overlay, sign-in screen |
| `src/controls.rs` | input: keyboard, macOS menu bar (`muda`), media keys / Now Playing (`souvlaki`) |
| `src/skin.rs` | how things look: `Skin`, `Look` (sizes), `Pal` (colours), fonts, and the drawn widgets |
| `src/settings.rs` | reading and writing `~/.resona/settings` and `unplayable` |
| `assets/` | bundled OFL fonts + licenses: DotGothic16 + VT323 (Wired), Archivo regular + extra-bold (Swiss) |

The Archivo files are static instances baked from the variable font with fontTools: egui only reads a variable
font's default instance.

## Things that are not obvious

- **Two client IDs.** Playback signs in with librespot's shared client ID (`CLIENT_ID` in engine.rs). The Web API
  uses the user's own developer app (`WEB_CLIENT_ID` in api.rs): the shared ID gets 24h rate-limit bans there,
  and its old keymaster token endpoint answers 503. Both sign-ins use `REDIRECT` (`http://127.0.0.1:8898/login`),
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
  counts. Refused songs are skipped in the direction you were going and faded in lists; clicking one retries it.
- **One way to drive playback:** buttons, media keys, the menu bar and the keyboard all call `command()` (the CLI too).
  The menu bar owns the ⌘ shortcuts (`keys()` skips them when the menu exists, or they'd fire twice). Plain-key
  shortcuts (Space, S, R, …) are only named in menu titles: as menu key equivalents they'd fire while typing.
- **Queue:** `queue` is the list being played (shuffled or not), `base` its original order, `queued` the hand-added
  songs right after `qi`. `Source::Queue` is drawn straight from the queue, not loaded. Shuffle is our own
  (`spread_shuffle`): each artist's songs are spaced evenly through the queue; switched on with nothing playing,
  it starts the open list.
- **Rate limits:** `api::get` retries short `retry-after` waits and turns anything over 60s into an error.
  librespot has its own limiter too: 300 session requests per 30 s, then 10/s, shared with playback. Session
  loaders go through `api::patient` (waits it out) and `session_playlist` paces itself past 200 songs; a starved
  player reports a playable song as `Unavailable`.
- **Spotify's February 2026 Web API changes:** saving and liking go through `/me/library` (by URI), playlist
  contents through `/playlists/{id}/items`, search pages are 10. Check the changelog before adding an endpoint.
- **Session playlists** are read page by page (`/playlist/v2/playlist/{id}?from=&length=`): librespot's
  `Playlist::get` returns only the first part of a long list.
  Developer apps get day-long bans for bursts, so startup stays small: playlists and Liked Songs only; what's in
  each playlist (`load_members`) waits for the first "Add to playlist" menu. A failed library load is shown, not
  fatal: playback runs on librespot and keeps working through a ban, and the sidebar falls back to the session's
  rootlist (`session_playlists`: only uploaded covers, mosaics are rebuilt from each list's first albums by `session_mosaic`; every list then opens through the session).
- **Search** asks for pages of 10 and stops at 5 pages; newer developer apps get a lower page-size cap.
- **Repaints:** nothing redraws when idle; 4 fps while playing, 20 fps while the equaliser bars or meter are on
  screen, ~16 fps while a list is loading.
- Errors are stored raw in `State.error` and shown through `short_err()` (full text on hover).

## Conventions

- Keep it small: stdlib and existing deps first, no new abstractions without a second user.
- Mark deliberate shortcuts with a `// ponytail:` comment that names the limit and the upgrade path.
- Non-trivial logic gets one small `#[test]` next to it.
- Both skins must keep working: anything drawn goes through `skin.rs`, and `skin::every_skin_lays_out_text_in_every_font`
  catches a font that fails to load.
