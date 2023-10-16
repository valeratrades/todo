#!/bin/sh
# Note that we're using some shortcuts from .bashrc. However, no env needs to be imported, as it happens automatically when we are sorced from .bashrc

todo() {
  v ~/Todo
}

tadd() {
  mkfile "${HOME}/Todo/${1}.md"
}

taddo() {
  mkfile "${HOME}/Todo/${1}.md"
  v "${HOME}/Todo/${1}.md"
}

tder() {
  mkfile "${HOME}/Todo/${1}/main.md"
  v "${HOME}/Todo/${1}/main.md"
}
