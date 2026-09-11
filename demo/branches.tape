# Branches: which are unpushed, which are finished, and deleting a finished one
# where it stands. The cross-repo `slu tidy` is a still, from plain.tape.
#
#   ./render.sh branches

Output "../readme-assets/branches.gif"

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
# ---- what is pushed and what is not --------------------------------------
Show
Sleep 300ms
Type "slu ibranch"
Enter
Sleep 2.5s
# h/l moves between the tabs: yours, the remote's, and the finished ones
Type "l"
Sleep 1.5s
Type "l"
Sleep 1.8s
Type "h"
Type "h"
Sleep 1.2s
# `/` narrows the list the plain keys drive, `?` the commits under it, and both
# say on their own title which key opened them
Type "/"
Sleep 500ms
Type "fix"
Sleep 1.8s
Escape
Sleep 500ms
Type "?"
Sleep 500ms
Type "health"
Sleep 1.8s
Escape
Sleep 600ms
# d on a branch that was never pushed wants its name typed: its commits exist
# nowhere else. Esc backs out
Type "d"
Sleep 2.8s
Escape
Sleep 600ms
Type "j"
Type "j"
Sleep 800ms
# a gone one gets the confirm, which opens on No; Yes is one step left
Type "d"
Sleep 1.2s
Left
Sleep 1s
Enter
Sleep 2s
Type "q"
Sleep 500ms
