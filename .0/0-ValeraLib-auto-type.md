

1) change v.load() command to go through the filetypes (if none specified), from the most to least performant, checking if any exists in the dir.

2) v.dump() will have to now have a manual specification of the desired format (by prefixing it, so like `v.pdump` or `v.jdump`). if not already in it, will convert.
