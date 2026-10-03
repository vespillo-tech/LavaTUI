# Changelog

What changed in each version of LavaTUI, in plain words.

## 1.3.0 — 2026-10-03

Lyrics now light up word by word, Apple Music works on a Mac, and the
lamp can grow a layer of wax at the top. Covers, timing and many small
things got smoother too.

**Lyrics**

- Lyrics light up word by word, like karaoke. The words already sung
  are bright, the word being sung is in the accent colour, and the rest
  of the line waits in grey. Without colours, the word being sung is
  underlined. Most lyrics only say when each line starts, so LavaTUI
  guesses when each word comes from its syllables, its commas and the
  singer's usual pace. The guess is close but not exact. Lyrics that
  time every word are followed exactly. Don't want it? Settings (`,`) ›
  music & lyrics › word by word.
- Lyrics keep time with Spotify to within a few milliseconds. Before,
  they could be up to a tenth of a second off for a whole song. When you
  pause, play or skip in the player itself, the lyrics follow straight
  away.
- Lyrics a little early or late for your ears? (Bluetooth headphones
  often play a bit late.) Move them up to a second either way: settings
  (`,`) › music & lyrics › lyrics timing.
- Lyrics on the lamp stay easy to read, even over busy looks like
  synthwave. Each word keeps one colour, and over bright wax the words
  turn dark. The clock and music on the lamp get the same care.
- Lyrics beside the lamp never cut off the line being sung. Long lines
  wrap onto more rows, and on big screens the panel widens to fit them.
  Lyrics in Chinese, Japanese, Korean or Thai wrap too.
- When a song has no lyrics, LavaTUI says so plainly: "no lyrics on
  lrclib.net for this song", or "instrumental".

**Music**

- Apple Music works on a Mac. LavaTUI shows the song, the cover and the
  lyrics, and you can play, pause, skip, seek, change the volume, and
  turn shuffle and repeat on or off. With both Spotify and Music open,
  LavaTUI follows the one that's playing. It never opens either app. The
  first time, your Mac asks if your terminal may control Music.
- On every system, LavaTUI follows whatever is playing. Spotify comes
  first only while it plays, and when you pause, the keys stay with the
  player you paused.
- Adding a song to a playlist that already has it now asks first:
  "already in Lamplight Mix · add it again?" Press `Enter` to add it
  again, or `Esc` to pick another playlist. In the add list, playlists
  that have the song show a `✓`.
- Liking songs and adding them to playlists say they're for Spotify
  songs when another app is playing. The add button shows for every
  Spotify song right away.
- A short hiccup no longer hides a song's cover, lyrics or buttons until
  the next song. A cover or lyrics that couldn't load are tried again
  while the song plays.
- Performance info (`d`) also shows the lyrics' timing: where the song
  is, the line and word, and whether the word times are exact or a
  guess. Handy for telling us what you see.

**Lamp**

- Wax at the top: like a real lava lamp, a thin layer of wax can rest
  under the top. A blob that rises and cools a little can stick to it
  and slowly seep in. Where wax joined, the layer sags into a drop that
  stretches, snaps and falls. It's off unless you turn it on: settings
  (`,`) › look › wax at the top.
- Showing or hiding the side panel, pressing `m` for just the lamp, or
  resizing the window no longer makes the wax jump. The wax stays where
  it is and slowly settles into the new space.
- A paused lamp with music or lyrics on screen uses much less of your
  computer.

**Covers**

- Cover quality (`O`, or in settings) is now sharp, small pixels, medium
  pixels or big pixels, and each one looks clearly different. Sharp is
  the real picture where your terminal can show it. The pixel sizes turn
  the cover into pixel art, in every terminal. Your old choice carries
  over.
- In terminals that show pictures, the cover no longer flashes a
  blocky version while a new picture loads.

**Clock**

- The clock's seconds can be turned off: settings (`,`) › clock & timer
  › seconds. The big clock then shows just hours and minutes (no second
  hand on the analog face), and a paused lamp wakes once a minute
  instead of every second.

**Fixes**

- Pausing or jumping with LavaTUI's keys no longer flickers back for a
  moment.
- Browsing a playlist that has removed songs no longer shows some songs
  twice.
- When Spotify asks LavaTUI to slow down, it now waits as long as asked,
  everywhere. A song list that didn't load is tried again a few seconds
  later.
- On Windows, after you quit and reopen your music app, play, pause and
  skip show up straight away again.

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
