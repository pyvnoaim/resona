# TODO

## Known issues

- [ ] The error screen says "Restart sonus to try again": add a retry instead.
- [ ] Track-row separators in Wired are drawn dot by dot (`Shape::dashed_line` with 1px dashes): thousands of
      shapes per frame with a full screen of rows. A solid faint line would be nearly free.
- [ ] Dithered stand-in covers draw one quad per 2px cell (~4k for the 132px header). Bake a texture if they grow.
- [ ] Playlists you don't own load 25 tracks per batch; a 1000-track list takes a while (it streams in, though).
- [ ] Two expiring Web API calls can both refresh the token at once (`WebAuth::token`). Harmless today.
- [ ] `block` (pulled in by `souvlaki` for media keys) has a future-incompatibility warning from rustc.

## Ideas

- [ ] Spotify Connect (control from the phone, hand playback over); librespot supports it.
- [ ] Filter box for the open playlist (search only covers all of Spotify right now).
- [ ] Clicking the artist or cover in the focus view could open the artist / album page.
- [ ] Click the `>` prompt to focus the search box (only the text area takes clicks now).
- [ ] Measure CPU of the release build while playing (idle is 0%, ~106 MB).
