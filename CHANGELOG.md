# Changelog

What changed in each version of LavaTUI, in plain words.

## Unreleased

**Lamp**

- Wax at the top: like a real lava lamp, a thin layer of wax can rest
  under the top. Some rising blobs melt into it, and now and then it lets
  a drop fall. Turn it on in the settings (`,` › look › *wax at the
  top*). It's off unless you turn it on.

**Clock**

- The clock's seconds can be turned off: settings (`,`) › clock & timer ›
  seconds. Off, the big clock shows just hours and minutes at every size
  (no second hand on the analog face), and a paused lamp wakes once a
  minute instead of every second. On is still the default.

**Music**

- A short hiccup no longer hides a song's cover, lyrics or like and add
  buttons until the next song. If Spotify is slow to answer for a moment,
  the music stays on screen. A cover or lyrics that couldn't load are
  tried again while the song plays.
- The add-to-playlist button shows for every Spotify song right away.
- Cover quality (`O`, or in settings) is now sharp, small pixels, medium
  pixels or big pixels, and each one looks clearly different. Sharp is
  the real picture where your terminal can show it. The three pixel
  sizes turn the cover into pixel art with bigger and bigger squares, in
  every terminal. Before, the choices often looked the same. Your old
  choice carries over.
- Adding a song to a playlist that already has it now asks first:
  "already in Lamplight Mix · add it again?" Press `Enter` to add it
  again, or `Esc` to pick another playlist. In the add list, playlists
  that have the song show a `✓`. If LavaTUI can't check, it adds the song
  as before and says it couldn't check.
- Lyrics beside the lamp never cut off the line being sung: long lines
  wrap onto more rows, nearby lines are shortened after a whole word, and
  on big screens the panel widens to fit them.
- When a song has no lyrics, the lyrics widget now says whose shelf is
  bare: "no lyrics on lrclib.net for this song" (just "not on
  lrclib.net" where space is tight). Songs that are instrumental say
  "instrumental", including ones where lrclib.net only has a note saying
  so.
- Lyrics light up word by word, like karaoke: the words already sung are
  bright, the word being sung is in the accent colour, and the rest of
  the line waits in grey. Without colours, the word being sung is
  underlined. Most lyrics only say when each line starts, so LavaTUI
  guesses when each word comes from its syllables and the line's commas
  and full stops. The guess is close but not exact. Lyrics that do time
  every word are followed exactly.
- Lyrics keep better time with Spotify: within a few milliseconds of
  where the song really is, where before they could be up to a tenth of
  a second early or late for a whole song. While lyrics are on screen,
  LavaTUI checks Spotify four times a second, so a pause, skip or jump
  you make in Spotify shows up within about a quarter of a second.
- Browsing a playlist that has removed songs no longer shows some songs
  twice when you scroll to the next batch.
- If lyrics come too early or too late for your ears (Bluetooth
  headphones often play a little late), move them: settings (`,`) ›
  music & lyrics › lyrics timing, up to a second sooner or later.

## 1.2.0 — 2026-10-02 — first public release

**Easier to get started**

- A welcome card on the first run shows the five keys you need most.
  Press `w` to see it again. In a tiny window it waits until there's
  room, and shows just `? help · q quit`.
- A settings screen (`,`): change the look, the clock and timer, where
  things go, music, the mouse and the window with the arrow keys. Every
  change shows on the lamp right away and saves by itself. Each page can
  be reset.
- Guided Spotify setup inside the settings. It explains who can use the
  Spotify library before you start, opens the right web page, copies the
  address you need and takes your Client ID by paste.
- Plainer words everywhere: "Space timer" instead of "␣ pomo", "new wax
  pattern", "performance info", and cover quality called photo, fine,
  medium and coarse.
- Help fits an 80×24 window, has a mouse section, and shows one action a
  line in narrow windows.
- Pickers say `Enter save · Esc cancel`, and the clock-face picker shows
  a preview.
- While the music controls are on, a quiet line says so and how to leave.

**Music**

- Album covers as real pictures in iTerm2, WezTerm, foot, mlterm and
  Konsole too (iTerm2 images and sixel), after a quick check that the
  terminal really supports them.
- Covers on Windows.
- Find a playlist or song in the playlist browser: press `/` and type.
- Play a song from a playlist without Premium on a Mac; the rest of the
  playlist follows.
- No Keychain question when LavaTUI starts on a Mac. Your Spotify login
  is read only when you first use a library feature, and LavaTUI tells
  you first that macOS may ask (choose *Always Allow*). You can keep the
  login in a private file instead, which never asks.
- Saved lyrics and covers stay small: old ones are removed on their
  own. The settings show how much is saved and can clear it all.
- Like, add and shuffle now follow the song you're actually playing on
  Linux and Windows too, and never act on another player.
- The mouse is on by default: click the music buttons and the progress
  bar, the cover to play or pause, and the wax to warm it.

**Looks**

- No more thin dark lines between rows in macOS Terminal and Ghostex:
  the wax is drawn as cell background there.
- Text on the lamp picks a dark or light colour for each letter, against
  whatever is behind it, so it stays readable over every style.
  Prefer one colour? Settings › widgets › *text on the lamp*: light or
  dark.
- The wax no longer jumps between frames when blobs join or split.
- Synthwave has a smooth, anti-aliased grid and a crisp horizon.
- One safe set of symbols for terminals that can't draw the fancy ones.

**Other**

- Easy installs: `brew install vespillo-tech/tap/lavatui` on a Mac or
  Linux, or one command that downloads and installs it (Mac, Linux and
  Windows).
- Ready-made downloads for macOS (Apple and Intel), Linux (PC and ARM)
  and Windows on the Releases page.
- Licensed under MIT or Apache 2.0.

## 1.1.0

**The lamp fills the window**

- The glass lamp outline is gone: the wax now fills the whole window,
  edge to edge.
- The lighting effect is gone, and so are the heatmap, dither and crt
  styles. Nine styles remain. Old settings that used them quietly switch
  to `solid`.
- Bigger, more varied blobs.

**Widgets**

- The clock and the focus timer can each sit beside the lamp, float on
  it, or be turned off (`t`, `f`). Each item on the lamp has its own spot
  (`l` moves it, `L` picks which one).
- When space runs out, the less important items shrink first, then fold
  into one small row in the corner.

**Music** (all off until you turn it on)

- Now playing (`a`): the song, artist, album, a small cover, a progress
  bar and the volume, beside the lamp or on it.
- Music controls (`Shift+A`): play, pause, skip, seek and volume from
  the keyboard.
- Works with the Spotify app on a Mac, any MPRIS player on Linux and the
  Windows media controls.
- Synced lyrics (`y`) from lrclib.net, with the current line bright and
  a gentle fade from line to line.
- A big album cover (`o`), as a real picture in kitty and Ghostty and
  drawn with text elsewhere (`Shift+O` changes the quality).
- Your Spotify library (optional, needs setup): browse playlists, like
  songs and add them to playlists; shuffle and repeat with Premium.

**Other**

- See-through Ghostty windows no longer show stripes in the wax.
- Smoother frames, and less work for your computer.

## 1.0.0

The first version.

- A simulated lava lamp: warm wax rises, cools, sinks, joins and splits.
  The same `--seed` always plays out the same lamp.
- Twelve render styles and eight colour themes, with a style picker that
  previews as you move.
- A glass lamp outline (or the wax edge to edge) and a lighting effect.
- Six clock faces and a focus timer with breaks, a flash and a bell.
- Lamp-only mode (`m`), help (`?`), and a layout that fits any window,
  from tiny to huge.
- Works in full colour, 256 colours, 16 colours or none.
- Settings saved to a file that you can also edit by hand.
