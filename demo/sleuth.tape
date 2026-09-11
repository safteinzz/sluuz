# Sleuthing: the leaked password hunted with the query inside the tool, and the
# diff of each hit under it. The plain `search` and `scan` are stills, taken by
# plain.tape.
#
#   ./render.sh sleuth

Output "../readme-assets/sleuth.gif"

Set Shell bash
# The prompt is drawn with nerd-font glyphs; without this they render as boxes.
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 960
Set Height 600
Set Padding 20
Set Framerate 20
Set TypingSpeed 55ms

Hide
# `exec`, so a stage that refuses to open takes the terminal with it: without it
# the script exits, the outer shell survives, and every keystroke after this
# lands in the *real* home - which is how a README image ends up showing
# somebody's actual connections.
Type "exec ./stage.sh shell"
Enter
Sleep 2s
Type "clear"
Enter
Sleep 600ms
# ---- the hunt, with the query inside the tool -----------------------------
Show
Sleep 300ms
Type "slu iscan"
Enter
Sleep 1.2s
Type "password"
Sleep 800ms
Enter
Sleep 2.2s
# the diff of the commit behind each hit follows the cursor
Down
Sleep 1.4s
Down 2
Sleep 1.4s
Down
Sleep 2.5s
Type "q"
Sleep 500ms
