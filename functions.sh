tev() {
	if [ -z "$1" ]; then
		todo manual -o
	else
		todo manual --ev ${@}
	fi
}
alias tdo="cs ${HOME}/.data/personal/"
alias tstart="todo timer start"
alias tdone="todo timer done"
alias tfailed="todo timer failed"
