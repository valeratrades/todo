# NOT posix
TODO_PATH="${HOME}/s/todo/.0"

local pull() {
	( git -C "$TODO_PATH" pull > /dev/null 2>&1 & ) & disown
}
local push() {
	( git -C "$TODO_PATH" add -A && git -C "$TODO_PATH" commit -m "." && git -C "$TODO_PATH" push ) > /dev/null 2>&1 & disown
}
todo() {
	pull
	e "$TODO_PATH"
	push
}

tq() {
	git -C "$TODO_PATH" pull > /dev/null 2>&1
	sleep 0.1
	e "${TODO_PATH}/quickfix.md"
	push
}

tadd() {
  mkfile "${TODO_PATH}/${1}.md"
}

taddo() {
  mkfile "${TODO_PATH}/${1}.md"
	pull
  e "${TODO_PATH}/${1}.md"
	push
}

tder() {
  mkfile "${TODO_PATH}/${1}/main.md"
	mkfile "${TODO_PATH}/${1}/quickfix.md" # idea is to have this as a place for the my_todo tool to be used upon (Am I hearing 'automate this too?). So in the future will likely automatically schedule the next task from here after the previous is done/failed; but for now just promising to exclude the ones I'm not able to finish immediately || not include them in the first place
	pull
  e "${TODO_PATH}/${1}/main.md"
	push
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
