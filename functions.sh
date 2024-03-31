# For whatever reason, tev freaks out on providing any -d* that is not 0. Ho clue why, but I blame bash.
#tev() {
#	days_back="-d0"
#	if [ -n "$1" ]; then
#		if [ "$1" = "-d*" ] || [ "$1" = "--days*" ]; then
#			days_back=${1}
#			shift
#		fi
#	fi
#
#	if [ -z "$1" ]; then
#		todo manual ${days_back} open
#	else
#		todo manual ${days_back} ev ${@}
#	fi
#}
alias tm="todo manual"
alias tdo="cs ${HOME}/.data/personal/"
alias tstart="todo timer start"
alias tdone="todo timer done"
alias tfailed="todo timer failed"
