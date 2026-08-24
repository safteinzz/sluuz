# Many repositories at once: the dashboard, then the same dashboard with a way
# in - repos to branches to commits to a diff, and back out again.
#
#   ./render.sh repos

Output "../readme-assets/repos.gif"

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
Type "clear"
Enter
Sleep 600ms
Show
Sleep 400ms

# ---- who needs attention --------------------------------------------------
Type "slu repos"
Enter
Sleep 2.5s
Type "slu repos --dirty"
Enter
Sleep 2.8s

# ---- the same list, with a way in ----------------------------------------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
Type "slu irepos"
Enter
Sleep 1.5s
Down 3
Sleep 1.5s
# repos → branches
Enter
Sleep 1.8s
Down 4
Sleep 1.4s
# branches → commits
Enter
Sleep 1.8s
Down 2
Sleep 1.4s
# commits → the diff, three enters below a list of repositories
Enter
Sleep 1.8s
Ctrl+D
Sleep 2.5s
# and esc walks back up the way it came down
Escape
Sleep 900ms
Escape
Sleep 900ms
Escape
Sleep 1.2s
Type "q"
Sleep 3s
