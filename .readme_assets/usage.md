```sh
# Blocker management (main workflow)
todo blocker add "implement feature X"    # Add a new blocker
todo blocker                              # Open current blocker file in $EDITOR
todo blocker pop                          # Complete current blocker, move to next
todo blocker set projectname              # Switch to different project

# GitHub Issues
todo open https://github.com/owner/repo/issues/123  # Fetch and open issue
todo open -t owner/repo/my-issue                    # Create new issue (touch mode)
todo open pattern                                   # Fuzzy find local issue

# Milestones
todo milestones                           # Show current milestones
todo milestones push "goal description"   # Add goal to current milestone

# Time tracking
todo clockify start                       # Start tracking current blocker
todo clockify stop                        # Stop tracking

# Shell integration (add to your shell rc)
eval "$(todo init zsh)"                   # Or: bash, fish
```
