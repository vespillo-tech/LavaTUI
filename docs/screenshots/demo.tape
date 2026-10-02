# The animated README demo: docs/screenshots/demo.gif
#
#     vhs docs/screenshots/demo.tape        # from the repo root
#     gifsicle -O3 --colors 80 -b docs/screenshots/demo.gif
#
# Needs vhs (with ttyd + ffmpeg) and gifsicle. Builds the release binary,
# runs it with a fixed --seed, a scratch config (your own settings are
# untouched) and the hidden --demo player (made-up songs, an abstract
# cover and invented lyrics: no real music in the GIF). Then it walks
# through styles, palettes, the style picker, the clock and timer on the
# lamp, the music widgets, a glimpse of the settings and minimal mode.
#
# Size: keep the GIF under 5 MB. Truecolor wax changes most pixels every
# frame, so it is recorded small and at 8 fps, and squeezed to 80 colours
# (about 4.8 MB). Don't use gifsicle --lossy: it smears the panel.

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
Type "export PS1='> ' COLORTERM=truecolor; cargo build --release -q && clear"
Enter
Wait@300s
Type `printf '[ui]\nwelcome = false\n[lamp]\nstyle = "solid"\n' > /tmp/lavatui-demo.toml && clear && ./target/release/lavatui --seed 7 --fps 8 --demo --config /tmp/lavatui-demo.toml; clear`
Enter
# Let the first blobs bud off the pool before the camera rolls.
Sleep 5s
Show

# solid; start the focus timer
Sleep 2s
Type " "
Sleep 1.5s
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
# Clock and timer onto the lamp.
Type "t"
Sleep 1.5s
Type "f"
Sleep 2s
# Music: the card beside the lamp, then lyrics and the cover on it.
Type "t"
Sleep 0.3s
Type "f"
Sleep 0.3s
Type "a"
Sleep 2s
Type "y"
Sleep 0.6s
Type "y"
Sleep 0.6s
Type "o"
Sleep 0.6s
Type "o"
Sleep 3.5s
# → topo, chrome
Type "ss"
Sleep 1.5s
Type "s"
Sleep 1.5s
# A glimpse of the settings.
Type ","
Sleep 1.2s
Type "j"
Sleep 0.6s
Enter
Sleep 2s
Escape
Sleep 0.3s
Escape
Sleep 1s
# Minimal mode: just the lamp; then back.
Type "m"
Sleep 2s
Type "m"
Sleep 1s

Hide
Type "q"
Sleep 0.5s
