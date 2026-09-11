# Many repositories at once, with a way in: repos to branches to commits to a
# diff, and back out again. The plain `slu repos` is a still, from plain.tape.
#
#   ./render.sh repos

Output "../readme-assets/repos.gif"

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
Type "clear"
Enter
Sleep 600ms
# ---- every repo, with a way in -------------------------------------------
Show
Sleep 300ms
Type "slu irepos"
Enter
Sleep 1.5s
# `/` narrows the list the plain keys drive and `?` the pane below it, at every
# level: the repos, then that repo's branches, then the commits under those. The
# filter is how you get where you are going, so it replaces the arrow keys here.
Type "/"
Sleep 300ms
Type "proxy"
Sleep 1.2s
Enter
Sleep 500ms
# repos → branches
Enter
Sleep 1.5s
Type "/"
Sleep 300ms
Type "feat"
Sleep 1.2s
Enter
Sleep 400ms
Type "?"
Sleep 300ms
Type "Marek"
Sleep 1.4s
Enter
Sleep 400ms
# ctrl drives the pane below, so ctrl-↓ picks one of what the filter left
Ctrl+Down
Sleep 1s
# branches → commits
Enter
Sleep 1.5s
# commits → the diff, three enters below a list of repositories
Enter
Sleep 1.8s
Ctrl+Down
Ctrl+Down
Ctrl+Down
Sleep 2s
# and esc walks back up the way it came down
Escape
Sleep 800ms
Escape
Sleep 800ms
Escape
Sleep 900ms
Type "q"
Sleep 500ms
