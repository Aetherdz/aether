# aether TUI v2 demo — session list, chat transcript, slash-command palette
Output demo.gif
Set Width 1400
Set Height 900
Set FontSize 20
Set Padding 24
Set Margin 20
Set PlaybackSpeed 0.9
Set LoopOffset 0.15
Set Framerate 30
Set Theme "Dracula"

Type "env AETHER_CONFIG_DIR=/tmp/aether-gif-demo ./target/release/aether tui"
Sleep 1s
Enter
Sleep 1.2s

# Open the demo session
Enter
Sleep 1.2s

# Scroll the transcript to show the plan card + user blocks
Down
Sleep 0.5s
Down
Sleep 0.5s
Up
Sleep 0.8s

# Open the slash-command palette
Type "/"
Sleep 0.8s

# Navigate the palette
Down
Sleep 0.4s
Down
Sleep 0.4s
Up
Sleep 0.6s

# Close palette back to chat
Escape
Sleep 0.6s

# Show the input bar + usage row
Type "what is the agent loop?"
Sleep 1s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Backspace
Sleep 0.3s
Sleep 0.5s

# Quit back to sessions
Escape
Sleep 0.5s
Escape
Sleep 0.8s
