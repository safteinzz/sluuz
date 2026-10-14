# Every remote against the checked-out branch, with what each one holds. git's
# own list is a still, from plain.tape.
#
#   ./render.sh remote

Output "../readme-assets/remote.gif"

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1056
Set Height 664
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
Type "cd notifications-worker && clear"
Enter
Sleep 600ms
# ---- a fork, against both of its remotes ----------------------------------
Show
Sleep 300ms
Type "slu iremote"
Enter
Sleep 2.5s
# main has three commits origin lacks: p sends them
Type "p"
Sleep 2.2s
# upstream has two of its own: its branches, then what they hold
Type "j"
Sleep 1.2s
Enter
Sleep 1.2s
Enter
Sleep 2.2s
Escape
Sleep 500ms
Escape
Sleep 800ms
# a new remote through the form, then removed behind the gate, which opens on No
Type "c"
Sleep 800ms
Type "backup"
Sleep 400ms
Tab
Sleep 300ms
Type "git@gitlab.com:acme/worker-backup.git"
Sleep 800ms
Enter
Sleep 1.5s
Type "d"
Sleep 1.2s
Left
Sleep 800ms
Enter
Sleep 1.5s
Type "q"
Sleep 500ms
