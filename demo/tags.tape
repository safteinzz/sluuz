# Tags against the remote, with what went into each. git's own list is a
# still, from plain.tape.
#
#   ./render.sh tags

Output "../readme-assets/tags.gif"

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
# ---- the tags against the remote ------------------------------------------
Show
Sleep 300ms
Type "slu itag"
Enter
Sleep 2s
# one of each: not pushed, pushed, re-cut after the push (⚑), and only on the
# remote (↓). d on a pushed one deletes it for everyone, so it wants the name
# typed; Esc backs out
Type "j"
Sleep 800ms
Type "d"
Sleep 2.5s
Escape
Sleep 500ms
# the release candidate: the commits it added since the tag before it, a diff
Type "k"
Sleep 800ms
Enter
Sleep 1.3s
Enter
Sleep 2.2s
Escape
Sleep 500ms
Escape
Sleep 700ms
# and, never pushed, it can go
Type "d"
Sleep 1.2s
Left
Sleep 800ms
Enter
Sleep 1.8s
Type "q"
Sleep 500ms
