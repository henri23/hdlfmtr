-- hdlfmtr — entry point auto-loaded by Neovim on plugin load.
--
-- Defines the :HdlFmt user command. Configuration (conform.nvim
-- integration, options) happens when the user calls
-- `require("hdlfmtr").setup()` from their config.

if vim.g.loaded_hdlfmtr then
  return
end
vim.g.loaded_hdlfmtr = 1

vim.api.nvim_create_user_command("HdlFmt", function()
  require("hdlfmtr").format_buffer(0)
end, { desc = "Format the current buffer with hdlfmtr" })
