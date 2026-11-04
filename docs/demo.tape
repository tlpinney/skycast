# The README demo. Regenerate with: cargo build --release && vhs docs/demo.tape
# It records live data, so each run looks a little different.
Output docs/demo.gif

Set Shell "zsh"
Set FontSize 14
Set Width 1200
Set Height 820
Set Padding 12
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Hide
Type "export PATH=$PWD/target/release:$PATH && clear"
Enter
Show

Type "skycast -l Miami"
Sleep 400ms
Enter
Sleep 6s

# 7 days
Tab
Sleep 3s

# Radar: zoom out, then play the last two hours
Tab
Sleep 4s
Type "-"
Sleep 4s
Space
Sleep 8s
Space
Sleep 1s

# Sun and moon
Tab
Sleep 4s

# Tides
Tab
Sleep 6s

Type "q"
Sleep 500ms
