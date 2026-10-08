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
- [ ] `block` (pulled in by `souvlaki` for media keys) has a future-incompatibility warning from rustc.

## Ideas

- [ ] Rename the Spotify developer app to "Resona" (dashboard, by hand): its name shows on the sign-in page.

- [ ] Spotify Connect (control from the phone, hand playback over); librespot supports it.
- [ ] Filter box for the open playlist (search only covers all of Spotify right now).
- [ ] Clicking the artist or cover in the focus view could open the artist / album page.
- [ ] Click the `>` prompt to focus the search box (only the text area takes clicks now).
- [ ] Measure CPU of the release build while playing (idle is 0%, ~106 MB). The CLI: ~2% playing, ~34 MB.
- [ ] Try `GET /me/library/contains` for hearts on visible songs only (would drop the first-run full read).
- [ ] Characters the skin fonts lack (`⁻ ˢ ☆` in some titles) show as boxes in the window; a macOS fallback font
      (Arial Unicode, ~23 MB in memory) would cover them.
- [ ] Plays through librespot don't reach Spotify's history / Wrapped; Last.fm scrobbling would give own stats.
