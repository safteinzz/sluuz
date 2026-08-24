# Sleuthing: the same leaked password found three ways - the plain pickaxe, the
# audit, and the interactive scanner that puts the query inside the tool.
#
#   ./render.sh sleuth
#
# Also takes scan.png: the audit's report is dense enough to be worth reading at
# your own pace, which a loop does not let you do.
#
# 1400x860 at font size 14 is 148 columns by 44 rows. Every tape uses it: a TUI
# fills whatever frame it is given, but scrolling output cannot be cropped, so
# the frame is sized for the longest report and the panes get the room as a
# bonus.

Output "../readme-assets/sleuth.gif"

Set Shell bash
# The prompt is drawn with nerd-font glyphs; without this they render as boxes.
Set FontFamily "JetBrainsMono NF"
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

# ---- one string, every branch of every repo under you ---------------------
Type `slu search -r "1337-let-me-in"`
Enter
Sleep 3.5s

# ---- no string in mind: sweep for the usual ones --------------------------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
Type "slu scan"
Enter
Sleep 2.5s
Screenshot ../readme-assets/scan.png
Sleep 1.5s

# ---- and the same hunt with the query inside the tool ---------------------
Hide
Type "clear"
Enter
Sleep 400ms
Show
Sleep 300ms
Type "slu iscan"
Enter
Sleep 1.2s
Type "password"
Sleep 800ms
Enter
Sleep 2.2s
# the diff of the commit behind each hit follows the cursor
Down
Sleep 1.4s
Down 2
Sleep 1.4s
Down
Sleep 2.5s
Type "q"
Sleep 3s
