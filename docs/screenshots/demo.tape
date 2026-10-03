# The animated README demo: docs/screenshots/demo.gif
#
#     vhs docs/screenshots/demo.tape        # from the repo root
#     gifsicle -O3 --colors 80 -b docs/screenshots/demo.gif
#
# Needs vhs (with ttyd + ffmpeg) and gifsicle. Builds the release binary,
# runs it with a fixed --seed, a scratch config (your own settings are
# untouched) and the hidden --demo player (made-up songs, covers, lyrics
# and Spotify account: no real music in the GIF).
#
# The story (screenplay: lava-0bl): the lamp and its looks (styles,
# colours, the style picker; same seed and timing as the first demo, so
# the wax moves the same) → it's a clock and a focus timer → it's your
# music (card, karaoke lyrics, cover, its pixel sizes, the playlists, a
# new song, like, add with the "already in" question) → wax at the top →
# just the lamp, in synthwave, with the lyrics sung on it.
#
# The scratch config turns on the soft backing (dock.backing): over
# synthwave's grid, floating text picks a contrast ink per letter, which
# hides the karaoke colours.
#
# Size: keep the GIF under 5 MB. Truecolor wax changes most pixels every
# frame, so it is recorded small and at 8 fps, and squeezed to 80 colours.
# Don't use gifsicle --lossy: it smears the panel.

Output docs/screenshots/demo.gif

Set Shell "bash"
Set FontFamily "Menlo"
Set FontSize 11
Set LineHeight 1.0
Set Width 840
Set Height 480
Set Padding 0
Set Framerate 8
Set TypingSpeed 0
Set CursorBlink false
# No padding: the palette changes mid-demo, so any border would show.
Set Theme { "name": "lavatui", "background": "#0F0B0A", "foreground": "#E9DCCF", "cursor": "#0F0B0A", "selection": "#3A2A20", "black": "#0F0B0A", "red": "#CD3131", "green": "#0DBC79", "yellow": "#E5E510", "blue": "#2472C8", "magenta": "#BC3FBC", "cyan": "#11A8CD", "white": "#E5E5E5", "brightBlack": "#666666", "brightRed": "#F14C4C", "brightGreen": "#23D18B", "brightYellow": "#F5F543", "brightBlue": "#3B8EEA", "brightMagenta": "#D670D6", "brightCyan": "#29B8DB", "brightWhite": "#FFFFFF" }

Hide
# A plain terminal: nothing from the one vhs was started in (a Ghostty or
# Ghostex there would change how the lamp draws itself).
Type "export PS1='> ' COLORTERM=truecolor; unset TERM_PROGRAM TERM_PROGRAM_VERSION ZMX_SESSION ${!GHOSTEX_*} ${!LAVATUI_*}; cargo build --release -q && clear"
Enter
Wait@300s
Type `printf '[ui]\nwelcome = false\n[lamp]\nstyle = "solid"\n[dock]\nbacking = "soft"\n' > /tmp/lavatui-demo.toml && clear && ./target/release/lavatui --seed 7 --fps 8 --demo --config /tmp/lavatui-demo.toml; clear`
Enter
# Let the first blobs bud off the pool before the camera rolls.
Sleep 5s
Show

# 1. The lamp and its looks. Hook: solid lava, nothing pressed.
Sleep 3.5s
# → outline, ascii, braille
Type "s"
Sleep 1.2s
Type "s"
Sleep 1.2s
Type "s"
Sleep 1.5s
# new colours: ultraviolet, abyss
Type "pp"
Sleep 2s
# The style picker previews live: halftone, synthwave; keep it.
Type "S"
Sleep 0.9s
Type "j"
Sleep 0.8s
Type "j"
Sleep 1s
Enter
Sleep 2s

# 2. A clock and a focus timer, on the lamp; start the timer.
Type "t"
Sleep 1.3s
Type "f"
Sleep 0.6s
Type " "
Sleep 1.5s

# 3. Your music. Clock and timer off; the music card beside the lamp.
Type "tf"
Sleep 0.3s
Type "a"
Sleep 2s
# Lyrics beside it, lit word by word (a long line wraps, never cut off).
Type "y"
Sleep 2.5s
# The cover on the lamp; as pixel art (big pixels), and back.
Type "oo"
Sleep 1.1s
Type "OOOO"
Sleep 1.2s
Type "O"
Sleep 0.8s
# The playlists (music controls on: A); play another song, like it.
Type "Ab"
Sleep 1.2s
Enter
Sleep 1s
Type "j"
Sleep 0.4s
Enter
Sleep 1s
Escape
Sleep 0.1s
Escape
Sleep 0.2s
Type "s"
Sleep 1.2s
# Add it: ✓ marks a playlist that has it, so that one asks first.
Type "a"
Sleep 1.3s
Enter
Sleep 2s
Escape
Sleep 0.1s
Escape
Sleep 0.1s
Escape
Sleep 0.3s

# 4. Wax at the top, from the settings.
Type ","
Sleep 0.5s
Enter
Sleep 0.3s
Type "jjjj"
Sleep 0.4s
Right
Sleep 1s
Escape
Sleep 0.1s
Escape
Sleep 0.2s

# 5. Just the lamp, synthwave's horizon edge to edge, the new song's
# lyrics sung on it (cover off, lyrics onto the lamp, lamp only). The soft
# backing keeps the karaoke colours readable over the grid.
Type "oym"
Sleep 4.5s

Hide
Type "q"
Sleep 0.5s
