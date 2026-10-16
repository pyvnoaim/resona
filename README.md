# Resona

*Latin: resound!* A small native Spotify player for macOS, written in Rust with
[egui](https://github.com/emilk/egui) and [librespot](https://github.com/librespot-org/librespot).

Two skins, each with a dark and a light theme:

- **Wired**: cyber/pixel, black and red, in the spirit of *Serial Experiments Lain*. The default.
- **Swiss**: print grid, huge type, one red rule.

## Features

- Liked Songs, your playlists, playlists you follow but don't own, artist and album pages
- New releases: songs from the last two weeks' releases by the artists you follow
- Song radio: similar songs to any song (right-click → Go to song radio)
- Search across Spotify
- Queue, shuffle, repeat
- Like and unlike songs, add songs to playlists or a new one (right-click, also on the playing song)
- Big cover view with synced lyrics; artist and album names open their pages
- Media keys, Now Playing, and a native menu bar with shortcuts (press `?` in the app for the full list)
- Six accent colours (menu or ⌘,), also for the terminal version and the app icon
- Remembers skin, theme, accent, volume and the last open list

## Requirements

- A Mac (Windows and Linux build too, with fewer extras: see [Windows and Linux](#windows-and-linux))
- Spotify **Premium**: playback goes through librespot, which Spotify only allows for Premium accounts
- Your own Spotify developer app: free, made in step 3 below

## Install

### 1. Install the tools

Xcode's command line tools (for the compiler and git) and Rust:

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new terminal afterwards, so `cargo` is on your path.

### 2. Build and install Resona

```sh
git clone https://github.com/pyvnoaim/resona.git
cd resona
./bundle.sh
```

`bundle.sh` builds the release version (a few minutes the first time), puts `Resona.app` into `/Applications`
and the terminal version `resona` into `~/.local/bin`. Run it again after pulling updates.

### 3. Make your Spotify developer app

Resona reads your library through a small app of your own on Spotify's developer site. Spotify limits apps in
development mode to their owner, so everyone makes their own. It's free and takes two minutes:

1. Go to [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), sign in with your Spotify
   account and accept the developer terms.
2. Click **Create app**. Any name and description will do (for example "Resona").
3. Under **Redirect URIs**, add exactly `http://127.0.0.1:8898/login` and click Add.
4. Under **Which API/SDKs are you planning to use?**, tick **Web API**. Accept the terms and click **Save**.
5. Open the app's **Settings** and copy its **Client ID**. You don't need the client secret.

### 4. First launch

Open Resona from Applications or Spotlight. It asks for the client ID: paste it and click Connect. Your browser
then opens twice to sign in, once for playback and once for your app. After that Resona signs in by itself.

The terminal version asks for the client ID the same way: run `resona cli`.

## Use

Click a list on the left, then a song to play it. Right-click a song for the queue, likes,
playlists and song radio. Press `?` for every shortcut. Media keys and Control Center's Now Playing work too.

`resona cli` runs the same player full-screen in the terminal: arrows to move, tab to switch between your lists
and the songs, enter to play, `/` to search, `t` to switch between the two looks, `q` to quit.

## Files and signing in again

Everything Resona stores lives in `~/.resona/`. Delete `credentials.json` and `web_refresh_token` there to sign
in again (and `liked` when switching Spotify accounts), and `client_id` to use another developer app. Setting
`RESONA_CLIENT_ID` overrides the saved ID.

## Building by hand

```sh
cargo build --release
./target/release/resona
```

Use the release build: the debug build decodes audio too slowly.

## Windows and Linux

Resona builds there too (checked on every push by GitHub Actions), but it's only used day to day on macOS, so
expect rough edges. What's different:

- No menu bar: the same shortcuts use Ctrl instead of ⌘ (press `?` for the list).
- Media keys: Linux yes (MPRIS, so desktop media controls work); Windows not yet.
- No `bundle.sh`: build and run it directly.

```sh
# Linux (Debian/Ubuntu) only: audio and TLS libraries
sudo apt install libasound2-dev libssl-dev pkg-config

cargo build --release
./target/release/resona          # Windows: target\release\resona.exe
```

Resona keeps its files in `~/.resona` (on Windows `C:\Users\<you>\.resona`). The first-launch steps are the
same as on the Mac.

## License

MIT, see [LICENSE](LICENSE). The bundled fonts (DotGothic16, VT323, Archivo) are under the SIL Open Font
License; their licenses are in `assets/`.

Resona is not affiliated with Spotify.
