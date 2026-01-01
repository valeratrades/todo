```sh
nix build
```

# Configuration

Create `~/.config/todo/settings.toml`:

```toml
# Required for GitHub integration
github_token = "ghp_..."

# Required for Clockify integration
clockify_api_key = "..."
clockify_workspace_id = "..."

# Required for Google Calendar milestones
google_calendar_client_id = "..."
google_calendar_client_secret = "..."

# Optional
default_extension = "md"  # or "typ" for typst
```
