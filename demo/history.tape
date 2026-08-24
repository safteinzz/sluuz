# History: the aligned log, then the full-screen browser with a real side-by-side
# diff under it.
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
Set Width 1400
Set Height 860
Set Padding 20
Set Framerate 20
Set TypingSpeed 55ms

Hide
Type "./stage.sh shell"
Enter
Sleep 2s
Type "cd edge-proxy && clear"
Enter
Sleep 600ms
Show
Sleep 400ms

# ---- a log you can read ---------------------------------------------------
Type "slu trace -a"
Enter
Sleep 3.2s

# ---- and the same history with the diffs attached -------------------------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
Type "slu ilog"
Enter
Sleep 1.5s
Down 2
Sleep 1.4s
Enter
Sleep 1.5s
Ctrl+D
Sleep 1.5s
Screenshot ../readme-assets/ilog-diff.png
Sleep 1.5s
# ctrl and the same keys drive the diff: down it, then sideways for long lines
Ctrl+D
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
Sleep 3s
