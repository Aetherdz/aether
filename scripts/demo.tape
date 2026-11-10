# aether TUI demo — session list, welcome screen with new logo, chat transcript, agent loop
Output "demo.gif"
Set Shell bash
Set FontSize 22
Set Width 1150
Set Height 640
Set Padding 20
Set Margin 10
Set PlaybackSpeed 1.1

# 1. Launch Aether TUI (starts in sessions view with token ledger)
Type "AETHER_CONFIG_DIR=/tmp/aether-gif-demo /home/zaikabdou1/bin/aether tui"
Enter
Sleep 2.5s

# 2. Open new session to showcase the new mathematically aligned ASCII logo
Enter
Sleep 3.5s

# 3. Type a message into the chat prompt
Type "what is the 3-model agent loop?"
Sleep 2s

# 4. Tab to the 3-panel Agent screen (Plan -> Build -> Route)
Tab
Sleep 3s

# 5. Tab back to the Sessions screen
Tab
Sleep 2s

# 6. Select and open the existing session to show transcript & token accounting
Down
Sleep 1s
Enter
Sleep 3.5s

# 7. Return to sessions and quit cleanly with session epilogue
Escape
Sleep 1s
Type "q"
Sleep 1.5s
