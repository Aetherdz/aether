Output site/assets/img/demo.gif
Set Shell bash
Set FontSize 22
Set Width 1150
Set Height 640
Set Padding 20
Set Margin 10
Set PlaybackSpeed 1

# Aether v0.3 TUI — sessions list (right rail shows usage live)
Type "AETHER_CONFIG_DIR=/tmp/aether-gif-demo /tmp/opencode/aether-build/release/aether tui"
Enter
Sleep 2.5s

# Open the first session into the chat transcript
Enter
Sleep 2.5s

# Type a question into the input box
Type "summarize our agent loop work"
Sleep 2s

# Tab to the Agent screen (live plan -> build -> route panels)
Tab
Sleep 3s

# Tab to the session list
Tab
Sleep 2s

# Tab back to the chat
Tab
Sleep 2s

# Quit
Type "q"
