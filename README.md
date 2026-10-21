<div align="center">

# Resona

**A small native Spotify player for macOS.** Rust, [egui](https://github.com/emilk/egui) and
[librespot](https://github.com/librespot-org/librespot): no Electron, no web view.

[![build](https://github.com/pyvnoaim/resona/actions/workflows/build.yml/badge.svg)](https://github.com/pyvnoaim/resona/actions/workflows/build.yml)
[![release](https://img.shields.io/github/v/release/pyvnoaim/resona?sort=semver)](https://github.com/pyvnoaim/resona/releases/latest)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![macOS · Linux · Windows](https://img.shields.io/badge/platform-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows-lightgrey)

[Install](#install) · [Features](#features) · [Use](#use) · [Build from source](#build-from-source)

<img src="docs/skins.png" alt="Resona in its three skins: Wired, Swiss and ASCII" width="900">

<sub>Wired · Swiss · ASCII</sub>

</div>

*Latin: resound!* Three skins, each with a dark and a light theme:

- **Wired**: cyber/pixel, black and red, in the spirit of *Serial Experiments Lain*. The default.
- **Swiss**: print grid, huge type, one red rule.
- **ASCII**: a teletype printout. Every frame, bar, heart and cover is drawn in plain characters, in Courier.

## Install

You need Spotify **Premium** (playback goes through librespot, which Spotify only allows for Premium accounts)
and a free Spotify developer app of your own (step 2).

### 1. Install Resona

On a Mac (Apple silicon or Intel), paste this into Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/pyvnoaim/resona/main/install.sh | sh
```

It puts `Resona.app` into `/Applications` and the terminal player `resona` into `~/.local/bin`. Run it again to
update. On Linux and Windows, download a build from the
[releases page](https://github.com/pyvnoaim/resona/releases/latest) (see [Windows and Linux](#windows-and-linux)).

### 2. Make your Spotify developer app

Resona reads your library through a small app of your own on Spotify's developer site. Spotify limits apps in
development mode to their owner, so everyone makes their own. It's free and takes two minutes:

1. Go to [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), sign in with your Spotify
   account and accept the developer terms.
2. Click **Create app**. Any name and description will do (for example "Resona").
3. Under **Redirect URIs**, add exactly `http://127.0.0.1:8898/login` and click Add.
4. Under **Which API/SDKs are you planning to use?**, tick **Web API**. Accept the terms and click **Save**.
5. Open the app's **Settings** and copy its **Client ID**. You don't need the client secret.

### 3. First launch

Open Resona from Applications or Spotlight. It asks for the client ID: paste it and click Connect. Your browser
then opens twice to sign in, once for playback and once for your app. After that Resona signs in by itself.

The terminal version asks for the client ID the same way: run `resona cli`.

## Features

- Liked Songs, your playlists, playlists you follow but don't own, artist and album pages
- New releases: songs from the last two weeks' releases by the artists you follow
- Song radio: similar songs to any song (right-click → Go to song radio)
- Search across Spotify
- Queue, shuffle, repeat
- Like and unlike songs, add songs to playlists or a new one (right-click, also on the playing song)
- Big cover view with synced lyrics; artist and album names open their pages
- Media keys, Now Playing, and a native menu bar with shortcuts (press `?` in the app for the full list)
- Six accent colours or your own hex colour (menu or ⌘,): hover a swatch to try it on the whole app, click to
  keep it. Your own colour is shaded per theme so it stays readable. The terminal version and the app icon follow it
- Remembers skin, theme, accent, volume and the last open list

## Use

Click a list on the left, then a song to play it. Right-click a song for the queue, likes,
playlists and song radio. Press `?` for every shortcut. Media keys and Control Center's Now Playing work too.

`resona cli` runs the same player full-screen in the terminal: arrows to move, tab to switch between your lists
and the songs, enter to play, `/` to search, `t` to switch between the two looks, `q` to quit. It does most of
what the window does: `a` / `A` queue a song or play it next, `p` adds it to a playlist, `v` shows the lyrics,
`i` starts a song radio, `g` / `G` open the artist / album, `c` copies the link, `m` mutes, and the queue shows in
the library while it has songs. Media keys and Now Playing work there too. Press `?` for every key.

## Files and signing in again

Everything Resona stores lives in `~/.resona/`. Delete `credentials.json` and `web_refresh_token` there to sign
in again (and `liked` when switching Spotify accounts), and `client_id` to use another developer app. Setting
`RESONA_CLIENT_ID` overrides the saved ID.

## Build from source

Needs Xcode's command line tools and [Rust](https://rustup.rs):

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/pyvnoaim/resona.git
cd resona
./bundle.sh
```

`bundle.sh` builds the release version (a few minutes the first time), puts `Resona.app` into `/Applications`
and `resona` into `~/.local/bin`. To just run it: `cargo build --release && ./target/release/resona` (use the
release build: the debug build decodes audio too slowly).

Releases are made by pushing a tag (`git tag v0.2.0 && git push --tags`): GitHub Actions builds the universal
Mac app and the Linux and Windows binaries and attaches them to the release.

## Windows and Linux

Resona builds there too (checked on every push), but it's only used day to day on macOS, so expect rough
edges. Ready-made builds are on the [releases page](https://github.com/pyvnoaim/resona/releases/latest). What's different:

- No menu bar: the same shortcuts use Ctrl instead of ⌘ (press `?` for the list).
- Media keys: Linux yes (MPRIS, so desktop media controls work); Windows not yet.
- No app bundle: a plain `resona` binary (Windows: `resona.exe`).

To build it yourself:

```sh
# Linux (Debian/Ubuntu) only: audio and TLS libraries
sudo apt install libasound2-dev libssl-dev pkg-config

cargo build --release
./target/release/resona          # Windows: target\release\resona.exe
```

Resona keeps its files in `~/.resona` (on Windows `C:\Users\<you>\.resona`). The first-launch steps are the
same as on the Mac.

## License

MIT, see [LICENSE](LICENSE). The bundled fonts (DotGothic16, VT323, Archivo, Courier Prime) are under the SIL
Open Font License; their licenses are in `assets/`.

Resona is not affiliated with Spotify.
