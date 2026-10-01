# The animated README demo: docs/screenshots/demo.gif
#
#     vhs docs/screenshots/demo.tape        # from the repo root
#     gifsicle -O3 --crop 6,0+984x580 -b docs/screenshots/demo.gif
#
# Needs vhs (with ttyd + ffmpeg) and gifsicle. Builds the release binary,
# runs it with a fixed --seed and a scratch config (your own settings are
# untouched), then walks through lighting, styles, a palette, the style
# picker and minimal mode. The crop trims the strip outside the cell grid
# (6 px left, 10 px right), which keeps the vhs theme's background after
# the palette changes; recheck it if you change the font or window size.

Output docs/screenshots/demo.gif

Set Shell "bash"
Set FontFamily "Menlo"
Set FontSize 13
Set LineHeight 1.0
Set Width 1000
Set Height 580
Set Padding 0
Set Framerate 24
Set TypingSpeed 0
Set CursorBlink false
# No padding: the palette changes mid-demo, so any border would show.
Set Theme { "name": "lavatui", "background": "#0F0B0A", "foreground": "#E9DCCF", "cursor": "#0F0B0A", "selection": "#3A2A20", "black": "#0F0B0A", "red": "#CD3131", "green": "#0DBC79", "yellow": "#E5E510", "blue": "#2472C8", "magenta": "#BC3FBC", "cyan": "#11A8CD", "white": "#E5E5E5", "brightBlack": "#666666", "brightRed": "#F14C4C", "brightGreen": "#23D18B", "brightYellow": "#F5F543", "brightBlue": "#3B8EEA", "brightMagenta": "#D670D6", "brightCyan": "#29B8DB", "brightWhite": "#FFFFFF" }

Hide
Type "export PS1='> ' COLORTERM=truecolor; cargo build --release -q && clear"
Enter
Wait@300s
Type `printf '[lamp]\nstyle = "solid"\nlighting = true\n' > /tmp/lavatui-demo.toml && clear && ./target/release/lavatui --seed 7 --fps 30 --config /tmp/lavatui-demo.toml; clear`
Enter
# Let the first blobs bud off the pool before the camera rolls.
Sleep 5s
Show

# solid, lit; lighting off and back on
Sleep 3s
Type "l"
Sleep 1.8s
Type "l"
Sleep 1.5s
# → heatmap (outline flashes past)
Type "ss"
Sleep 2.5s
# → braille (ascii, dither flash past), then the next palette
Type "sss"
Sleep 2s
Type "p"
Sleep 2.2s
# The style picker previews live: halftone, crt, synthwave; keep it.
Type "S"
Sleep 0.9s
Type "j"
Sleep 0.7s
Type "j"
Sleep 0.7s
Type "j"
Sleep 1s
Enter
Sleep 2s
# → chrome (matrix, topo flash past)
Type "sss"
Sleep 2.5s
# Minimal mode: just the lamp; then back.
Type "m"
Sleep 2.5s
Type "m"
Sleep 1.5s

Hide
Type "q"
Sleep 0.5s
