#!/bin/sh
# Note that we're using some shortcuts from .bashrc. However, no env needs to be imported, as it happens automatically when we are sorced from .bashrc

todo() {
  e ~/Todo
}

tadd() {
  mkp "${HOME}/Todo/${1}.md"
}

taddo() {
  mkp "${HOME}/Todo/${1}.md"
  e "${HOME}/Todo/${1}.md"
}

tder() {
  mkp "${HOME}/Todo/${1}/main.md"
  e "${HOME}/Todo/${1}/main.md"
}

## `my_todo` stuff
tstart() {
	my_todo start "$@" &
}
tdone() {
	my_todo done
}
tfailed() {
	my_todo failed
}
tkill() {
	pkill -f my_todo
}
#
