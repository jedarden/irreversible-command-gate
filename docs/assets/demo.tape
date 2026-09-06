# vhs tape for docs/assets/icg-demo.gif -- see demo.sh for the commands it runs.
#
#   cargo build --release
#   PATH="$PWD/target/release:$PATH" vhs docs/assets/demo.tape

Output docs/assets/icg-demo.gif

Set Shell bash
Set FontSize 15
Set Width 1000
Set Height 800
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 0ms
Set Framerate 20
Set PlaybackSpeed 1.0

Hide
Type "clear" Enter
Show
Type "bash docs/assets/demo.sh" Enter
Sleep 32s
