# NOT posix

#NB all of this will be moved into rust itself, using clap.

TODO_PATH="${HOME}/Todo"

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

tev() {
	if [ -z "$1" ]; then
		tstart 15 c daliy-ev-calculation
	else
		if [ "$1" = "-y" ]; then
			shift
			${HOME}/s/todo/daily_stats/target/release/daily_stats "$1" "-y"
		else
			${HOME}/s/todo/daily_stats/target/release/daily_stats "$1"
			tdone > /dev/null 2>&1
		fi
		# easier than making a perfect error-throwing mechanism for my_todo
		return 0
	fi
}
tevo() {
	tev "$@"
	nvim ${HOME}/data/personal/daily_stats.json
}
#
