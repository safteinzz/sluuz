# Branches: which are unpushed, which are finished, and deleting the finished
# ones where they stand.
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
Type "./stage.sh shell"
Enter
Sleep 2s
Type "cd edge-proxy && clear"
Enter
Sleep 600ms
Show
Sleep 400ms

# ---- what is pushed and what is not --------------------------------------
Type "slu ibranch"
Enter
Sleep 2.5s
# h/l slides the list between local, all and remote
Type "l"
Sleep 1.8s
Type "l"
Sleep 1.8s
Type "h"
Type "h"
Sleep 1.5s
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
Type "q"
Sleep 800ms

# ---- finished branches, across every repo, then deleted in place ----------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
# -a keeps the already-clean repos in the report, which is what makes the sweep
# visible: five repos with nothing to clean up, one with four finished branches
Type "slu tidy -a .."
Enter
Sleep 3.5s
Type "slu itidy"
Enter
Sleep 2.2s
Enter
Sleep 1.5s
# the confirm opens on No, so a second enter would cancel; Yes is one step left
Left
Sleep 1.2s
Enter
Sleep 2s
Type "q"
Sleep 1.5s
