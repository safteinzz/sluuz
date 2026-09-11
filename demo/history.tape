# History: the browser with a real side-by-side diff under it. The plain
# `slu trace` is a still, from plain.tape.
#
#   ./render.sh history
#
# Also takes ilog-diff.png, because a diff is something you read rather than
# watch.

Output "../readme-assets/history.gif"

Set Shell bash
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
Type "cd edge-proxy && clear"
Enter
Sleep 600ms
# ---- the history, with the diffs attached ---------------------------------
Show
Sleep 300ms
Type "slu ilog"
Enter
Sleep 1.5s
Down 2
Sleep 1.4s
Enter
Sleep 1.5s
Ctrl+Down
Ctrl+Down
Ctrl+Down
Sleep 1.5s
Screenshot ../readme-assets/ilog-diff.png
Sleep 1.5s
# ctrl and the same keys drive the diff: down it, then sideways for long lines
Ctrl+Down
Ctrl+Down
Ctrl+Down
Sleep 1.8s
Ctrl+L
Ctrl+L
Ctrl+L
Sleep 1.8s
Ctrl+H
Ctrl+H
Ctrl+H
Sleep 1.2s
Escape
Sleep 1s
Type "q"
Sleep 500ms
