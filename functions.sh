TODO_PATH="${HOME}/s/todo/.0"

todo() {
	git -C "$TODO_PATH" pull &
	${EDITOR} "$TODO_PATH"
}

tq() {
	git -C "$TODO_PATH" pull > /dev/null 2>&1 &
	e "${TODO_PATH}/quickfix.md"
}

tadd() {
  mkfile "${TODO_PATH}/${1}.md"
}

taddo() {
  mkfile "${TODO_PATH}/${1}.md"
	git -C "$TODO_PATH" pull > /dev/null 2>&1 &
  e "${TODO_PATH}/${1}.md"
}

tder() {
  mkfile "${TODO_PATH}/${1}/main.md"
	mkfile "${TODO_PATH}/${1}/quickfix.md" # idea is to have this as a place for the my_todo tool to be used upon (Am I hearing 'automate this too?). So in the future will likely automatically schedule the next task from here after the previous is done/failed; but for now just promising to exclude the ones I'm not able to finish immediately || not include them in the first place
	git -C "$TODO_PATH" pull > /dev/null 2>&1 &
  e "${TODO_PATH}/${1}/main.md"
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
	elif [ "$1" = "-y" ]; then
		shift
		${HOME}/s/todo/daily_ev/target/release/daily_ev "$1" "-y"
		# easier than making a perfect error-throwing mechanism for my_todo
		return 0
	else
		${HOME}/s/todo/daily_ev/target/release/daily_ev "$1"
		tdone > /dev/null 2>&1
		# easier than making a perfect error-throwing mechanism for my_todo
		return 0
	fi
}
#
