# The stash: every entry, the files each one holds, their diffs, and a drop
# behind the typed gate. git's plain list is a still, from plain.tape.
#
#   ./render.sh stash

Output "../readme-assets/stash.gif"

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
# lands in the *real* home, which is how a README image ends up showing
# somebody's actual data.
Type "exec ./stage.sh shell"
Enter
Sleep 2s
Type "cd checkout-service && clear"
Enter
Sleep 600ms
# ---- what each stash holds ------------------------------------------------
Show
Sleep 300ms
Type "slu istash"
Enter
Sleep 2s
# the files a stash holds preview below it; enter opens one's diff
Enter
Sleep 2.5s
Escape
Sleep 500ms
Ctrl+J
Sleep 400ms
Enter
Sleep 2.5s
Escape
Sleep 500ms
# the week-old idea: d wants its name typed, since a dropped stash is gone
Type "j"
Sleep 1.2s
Type "d"
Sleep 1.5s
Type "stash@{1}"
Sleep 600ms
Enter
Sleep 2s
Type "q"
Sleep 500ms
