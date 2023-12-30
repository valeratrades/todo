1. just ws stats
- to get current: `swaymsg -t get_workspaces | jq -r '.[] | select(.focused==true).name'`
1. if chrome, get name of the tab
- swaymsg -t get_tree | jq -r '.. | (.nodes? // empty)[] | select(.focused==true)'
1. integrate with daily_ev
1. make an android app that sends me digital wellbeing stats
1. connect the data stream to the app
1. make a youtube video to share it with everybody.
1. once sure the technology is stable, share it on twitter too, append the short version of the backstory. (do you want to have your self-esteem intact or will you burn the bridges?)
