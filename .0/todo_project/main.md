# TODO:
- add an sh command for adding to todo's quickfix instead of writing here


# Design
There should be a concept of last_accessed_type

And then with it in place, we immediately have everything falling into places with grouping.
Because I can specify a new one, or I can have it default to the previous one. That's it.

We will also automatically pull the first letter of each folder, and any command needing specification of the folder should be evocable with it.
// if name\[:1\] is taken, we check if name\[:i\] is, for i<=name.len()

Also, what the fuck, let's sync the groups across the board. (Which currently means folders in ~/Todo and in my_todo cli tool // make a unifying config)
And if any are too specific to be synced, they \(should be subgroups / shouldn't exist\) in the first place.

# MyTodo
## TODO:
- add `-add` and `-sub` flags to the `tev`

# General
Want to create the extension on current `tev`, (probably ditching the old design altogether), joining it correctly with all the optional metrics to be collected for the day, were I to so desire.

How about making a buffer, opened in nvim for this, akin to the `shell_harpoon`. Make it in two steps, first tev, (if entry for today exists, - show `...= THE_ENTRY;` and not `...= None;`), then a new window with all the optional things. And also what the fuck, let's add shortcuts for jumping into ~/data/personal things directly from there, (in a new tab within the opened instance).
And then no need to have a separate thing for timing this - just connect `my_todo start ev 10` and then `my_todo done` onto its close and open.
// Note that I will never retire the option of setting day's tev from the command line.

