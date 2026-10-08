# Resona

*Latin: resound!* A small native Spotify player for macOS, written in Rust with
[egui](https://github.com/emilk/egui) and [librespot](https://github.com/librespot-org/librespot).

Two skins, each with a dark and a light theme:

- **Wired**: cyber/pixel, black and red, in the spirit of *Serial Experiments Lain*. The default.
- **Swiss**: print grid, huge type, one red rule.

## Features

- Liked Songs, your playlists, playlists you follow but don't own, artist and album pages
- Search across Spotify
- Queue, shuffle, repeat
- Like and unlike songs, add songs to playlists (right-click)
- Big cover view with synced lyrics
- Media keys, Now Playing, and a native menu bar with shortcuts (press `?` in the app for the full list)
- Remembers skin, theme, volume and the last open list

## Requirements

- macOS
- Spotify **Premium** (playback goes through librespot)
- Rust (stable)
- Your own Spotify developer app (free, see below)

## Setup

1. Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard).
2. Add the redirect URI `http://127.0.0.1:8898/login`.
3. Put its **client ID** into `WEB_CLIENT_ID` in `src/api.rs`. No client secret needed: sign-in uses PKCE.

## Build and run

```sh
cargo build --release
./target/release/resona
```

Use the release build: the debug build decodes audio too slowly.

To install it as an app (Dock, Spotlight), run `./bundle.sh`: it builds `Resona.app` with its icon and moves it
into `/Applications`. Run it again after each change.

`resona` (installed into `~/.local/bin` by `bundle.sh`) runs the same player full-screen in the terminal: arrows to move, tab to switch
between your lists and the songs, enter to play, `/` to search, `t` to switch between the two looks, `q` to quit.

On first launch the browser opens twice to sign in, once for playback and once for the Web API.
Everything Resona stores lives in `~/.resona/`. Delete `credentials.json` and `web_refresh_token` there
to sign in again.

## License

MIT, see [LICENSE](LICENSE). The bundled fonts (DotGothic16, VT323, Archivo) are under the SIL Open Font
License; their licenses are in `assets/`.

Resona is not affiliated with Spotify.
