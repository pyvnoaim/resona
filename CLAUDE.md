# Sonus

A small native Spotify player for macOS: Rust, egui/eframe for the UI, librespot for sign-in and playback.
Two skins, each with a dark and a light palette: **Wired** (Lain-style cyber/pixel, the default) and **Swiss**
(print grid, huge type, one red rule). Others (Y2K, macOS, Phosphor, Brutal) were tried and dropped.

## Build and run

```sh
cargo build --release && ./target/release/sonus   # use release: debug decodes audio slowly
cargo test
cargo clippy --all-targets                            # keep it warning-free
```

Needs Spotify Premium (librespot playback). Everything the app keeps lives in `~/.sonus/`:

| file | what |
|---|---|
| `credentials.json` | librespot sign-in |
| `web_refresh_token` | Web API sign-in (mode 0600) |
| `settings` | `key=value`: skin, theme, volume, last open list |
| `unplayable` | track ids Spotify refused to play, one per line |

Delete the first two to sign in again.

## Layout

| file | what |
|---|---|
| `src/main.rs` | `App` (UI state), startup, `open()` (load a list), `command()` (all playback), the frame loop |
| `src/state.rs` | `State` and the queue: `Source`, `Cmd`, `Repeat`, `start` / `step` / `enqueue` / `set_shuffle` |
| `src/engine.rs` | librespot: `connect()` (sign-in, session, player, event loop), liked-songs and playlist-contents loaders |
| `src/api.rs` | Web API (`get`, `WebAuth`, search, like, add to playlist) and the session loaders (`session_playlist`, `session_artist`, `session_album`) |
| `src/ui.rs` | the views: sidebar, track list, player bar, focus view, help overlay, sign-in screen |
| `src/controls.rs` | input: keyboard, macOS menu bar (`muda`), media keys / Now Playing (`souvlaki`) |
| `src/skin.rs` | how things look: `Skin`, `Look` (sizes), `Pal` (colours), fonts, and the drawn widgets |
| `src/settings.rs` | reading and writing `~/.sonus/settings` and `unplayable` |
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
  reading all of Liked Songs once at startup (`load_liked`), because the "are these liked?" lookup is 403.
- **Refused songs:** librespot reports `Unavailable` for failed *preloads* too; only a failure of the playing track
  counts. Refused songs are skipped in the direction you were going and faded in lists; clicking one retries it.
- **One way to drive playback:** buttons, media keys, the menu bar and the keyboard all call `App::command`.
  The menu bar owns the ⌘ shortcuts (`keys()` skips them when the menu exists, or they'd fire twice). Plain-key
  shortcuts (Space, S, R, …) are only named in menu titles: as menu key equivalents they'd fire while typing.
- **Queue:** `queue` is the list being played (shuffled or not), `base` its original order, `queued` the hand-added
  songs right after `qi`. `Source::Queue` is drawn straight from the queue, not loaded.
- **Rate limits:** `api::get` retries short `retry-after` waits and turns anything over 60s into an error.
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
