#!/bin/sh
# Following is a standard set of rust run shortcuts

alias b="cargo build --release --manifest-path=./daily_stats/Cargo.toml && cargo build --release --manifest-path=./my_todo/Cargo.toml"
alias r="cargo run"

# f for full
alias f="cargo build --release && my_todo"
## e for executable
#alias e="my_todo"
alias g="git add -A && git commit -m '.' && git push"
alias gr="git reset --hard"
