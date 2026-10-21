# TODO

## Known issues

- [ ] The error screen says "Restart resona to try again": add a retry instead.
- [ ] Track-row separators in Wired are drawn dot by dot (`Shape::dashed_line` with 1px dashes): thousands of
      shapes per frame with a full screen of rows. A solid faint line would be nearly free.
- [ ] Dithered stand-in covers draw one quad per 2px cell (~4k for the 132px header). Bake a texture if they grow.
- [ ] Session playlists fetch each song's details one request at a time, paced under librespot's limiter: 588
      songs take ~45 s. Spotify's batch metadata endpoint (extended-metadata) would make it a few requests.
- [ ] Songs refused while the limiter starved the player were saved to `~/.resona/unplayable` as false positives;
      clicking one retries it. Telling a starved request apart from a real refusal would avoid that.
- [ ] The rootlist fallback reads one page of 500 entries, and has no covers for lists with Spotify's mosaic.
- [ ] A followed playlist that wasn't loaded (partial library) can be routed to the Web API and fail (`open_list`).
- [ ] Two expiring Web API calls can both refresh the token at once (`WebAuth::token`). Harmless today.
- [ ] New releases asks the Web API for every followed artist and reads them all again on each open; with
      hundreds of artists the first songs take a while. Cache the artist list (and each artist's newest release)
      for the session if it gets slow.
- [ ] New releases leaves out songs an artist only features on and compilations (`albums_current` and
      `singles_current` only).
- [ ] "Add to playlist" puts first the lists you last added to from Resona (`added_to`); adds made in other
      Spotify clients don't count.
- [ ] A song radio is not kept: it isn't in the sidebar and isn't reopened at launch (`Source::Radio` has no key).
- [ ] The sidebar lays out every playlist row each frame (the song list uses `show_rows`); slow with hundreds of
      playlists. Keyboard reveal (`scroll_to_me`) would need the song list's approach.
- [ ] egui's image cache keeps every cover it decoded until quit; `ctx.forget_image` on the list being left
      would cap memory over long sessions.
- [ ] The whole window redraws at 20 fps while the playing row's equaliser is on screen.
- [ ] Courier is wide, so long playlist names in the ASCII sidebar are cut early.
- [ ] The accent swatches take the mouse only (no arrow keys yet), and the menu bar's Accent submenu has no
      custom colour.
- [ ] Every custom accent tried stays in memory until quit (`skin::custom_pal`, ~100 bytes each).
- [ ] The CLI wakes 10 times a second while it waits, to hear media keys (macOS delivers them on the main run
      loop, `cli::turn_run_loop`). A waker thread would bring idle back to zero.
- [ ] The CLI's key list (`?`) is cut off in short terminals; `c` needs `pbcopy`, `wl-copy`, `xclip` or `clip`
      (nothing over SSH; OSC 52 would reach the local terminal).

## Ideas

- [ ] Spotify Connect (control from the phone, hand playback over); librespot supports it.
- [ ] Filter box for the open playlist (search only covers all of Spotify right now).
- [ ] Click the `>` prompt to focus the search box (only the text area takes clicks now).
- [ ] Measure CPU of the release build while playing (idle is 0%, ~106 MB). The CLI: ~2% playing, ~34 MB.
- [ ] Try `GET /me/library/contains` for hearts on visible songs only (would drop the first-run full read).
- [ ] Characters the skin fonts lack (`⁻ ˢ ☆` in some titles) show as boxes in the window; a macOS fallback font
      (Arial Unicode, ~23 MB in memory) would cover them.
- [ ] Plays through librespot don't reach Spotify's history / Wrapped; Last.fm scrobbling would give own stats.
- [ ] Keep New releases as a real Spotify playlist too (add the new songs, drop ones older than two weeks), so it
      shows on the phone.
- [ ] Release Radar: Spotify's own weekly list; readable through the session once its playlist id is known.
