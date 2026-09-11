# The plain commands, as stills: their output is read rather than watched, so
# it stays out of the feature GIFs, which open on the interactive tools. Each
# still is a Screenshot; the GIF this records lives in the stage and goes when
# it is torn down.
#
#   ./render.sh plain

Output "home/plain.gif"

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 960
Set Height 600
Set Padding 20
Set Framerate 20
Set TypingSpeed 1ms

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
Show
Sleep 300ms

# ---- every repo at once ---------------------------------------------------
Type "slu repos"
Enter
Sleep 1.5s
Type "slu repos --dirty"
Enter
Sleep 1.5s
Screenshot ../readme-assets/repos-plain.png
Sleep 1s

# ---- the usual secret terms, across every repo -----------------------------
Type "clear"
Enter
Sleep 400ms
Type "slu scan"
Enter
Sleep 3s
Screenshot ../readme-assets/sleuth-plain.png
Sleep 1s

# ---- a log you can read ----------------------------------------------------
Type "cd edge-proxy && clear"
Enter
Sleep 400ms
Type "slu trace -a"
Enter
Sleep 2s
Screenshot ../readme-assets/history-plain.png
Sleep 1s

# ---- finished branches across every repo -----------------------------------
Type "clear"
Enter
Sleep 400ms
# -a keeps the already-clean repos in the report, which is what makes the sweep
# visible: five repos with nothing to clean up, one with four finished branches
Type "slu tidy -a .."
Enter
Sleep 2.5s
Screenshot ../readme-assets/branches-plain.png
Sleep 1s

# ---- tags, as git prints them ----------------------------------------------
Type "clear"
Enter
Sleep 400ms
Type "slu tag -n"
Enter
Sleep 1.2s
Screenshot ../readme-assets/tags-plain.png
Sleep 1s

# ---- status and log, passed straight through to git -----------------------
Type "cd ../checkout-service && clear"
Enter
Sleep 400ms
Type "slu status -sb"
Enter
Sleep 1.2s
Type "slu log --oneline -4"
Enter
Sleep 1.2s
Screenshot ../readme-assets/status-plain.png
Sleep 1s
