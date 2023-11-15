that includes:

1) have git repo, with bash script and plugin-specific settings in it. (dir is ~/plugins/, and the settings is mainly this one: '''--make netrw sort Todo directory in reverse order
vim.cmd [[
  autocmd FileType netrw if expand("%:p") =~ '~/Todo' | let g:netrw_sort_direction='reverse' | endif
]]```)


2)
a) figure out the links
b) extend tadd and taddo to detect when base_folder/0-task is passed instead of 0-task; deprecate tder.
c) on detection now create a link to them in general Todo folder.
d) test if links handle themselves when base is deleted, or somehow in the plugin configuration make a function to do so.
