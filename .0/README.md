# Format
Every entry has the following format:
`{importance}_{difficulty}_{name}`,
where:
- importance: 0->9, the higher the more important
- difficulty: 0->9, the higher the more difficult
- day_section is provided as argument. "w:work", "e:evening", and default is "evening"
// we assume morning is meant for more physical things, so that and its brevity make it so that no tasks are assigned to the morning section. 
- name: name of the task, with spaces substituted by '-', not '_'

# Style
- to express a blocking or non-blocking nature of a step, I shall use the following apprehension:
```md
1. this is blocking
2.a this is non-blocking
2.b neither is this
3. this is blocking && requires both of the previous steps to have been finished.
```

# Quickfix
It is foremost for things I will do.

The rest of _ideas_ I shall just write down wherever, which currently happens to be tg.

Obviously, after that, quickfix is for things that do not require much thought, planning or synchronising.
