# Staging: git's own status, and then the same status you can act on.
#
#   ./render.sh status

Output "../readme-assets/status.gif"

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
Type "cd checkout-service && clear"
Enter
Sleep 600ms
Show
Sleep 400ms

# ---- it is still git ------------------------------------------------------
Type "slu status -sb"
Enter
Sleep 2.2s
Type "slu log --oneline -4"
Enter
Sleep 2.5s

# ---- the same two columns, with the diff and the staging keys -------------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
Type "slu istatus"
Enter
Sleep 1.5s
Down 2
Sleep 1.8s
# s stages it: the M jumps from the right column to the left, and the diff
# pane switches to what is now staged
Type "s"
Sleep 2.5s
Type "u"
Sleep 2.2s
# and h/l narrows the list to one half of that two-column code
Type "h"
Sleep 1.8s
Type "l"
Type "l"
Sleep 2s
Type "q"
Sleep 3s
