todo() {
  e ~/Todo
}

tadd() {
  mkfile "${HOME}/Todo/${1}.md"
}

taddo() {
  mkfile "${HOME}/Todo/${1}.md"
  e "${HOME}/Todo/${1}.md"
}

tder() {
  mkfile "${HOME}/Todo/${1}/main.md"
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
tedit() {
	e ${HOME}/data/personal/todo.json
}
tev() {
	if [ -z "$1" ]; then
		tstart 15 c daliy-ev-calculation
	else
		tdone
		${HOME}/s/todo/daily_ev/target/release/daily_ev "$1"
	fi
}
#
