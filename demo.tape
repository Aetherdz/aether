Output site/demo.gif
Set FontSize 15
Set Width 1150
Set Height 640
Set Padding 20
Set Margin 10
Set PlaybackSpeed 1.2

# Aether TUI v2 — session list behind the chrome header + tab bar
Type "AETHER_CONFIG_DIR=/tmp/aether-gif-demo ./target/release/aether tui"
Enter
Sleep 2s

# Open the first session into the chat transcript
Enter
Sleep 2s

# Type a question into the input box
Type "summarize our agent loop work"
Sleep 1.5s

# Tab to the Agent screen (live plan -> build -> route panels)
Tab
Sleep 3s

# Tab back to the session list
Tab
Sleep 2s

# Quit
Type "q"
