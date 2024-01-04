local pull() {
	( git -C "$TODO_PATH" pull > /dev/null 2>&1 & ) & disown
}
local push() {
	( git -C "$TODO_PATH" add -A && git -C "$TODO_PATH" commit -m "." && git -C "$TODO_PATH" push ) > /dev/null 2>&1 & disown
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

#==============================================================================

alias tev="todo manual --ev="
