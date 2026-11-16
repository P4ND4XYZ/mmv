-- Optional Normal-mode PDF zoom bindings. Load explicitly from your init.lua.
local mmv = require("mmv")

local M = {}

-- Call once from init.lua; false skips registration without removing other mappings.
function M.setup(opts)
  if opts == nil then opts = {} end
  assert(type(opts) == "table", "pdf_zoom.setup expects a table")
  local keys = opts.keys
  if keys == nil then keys = {} end
  assert(type(keys) == "table", "pdf_zoom.setup keys must be a table")
  local bindings = {
    { "zoom_in", "+", function() mmv.pdf.zoom_in() end },
    { "zoom_out", "-", function() mmv.pdf.zoom_out() end },
    { "fit_width", "=", function() mmv.pdf.fit_width() end },
    -- Native topmost-page fit toggle; this does not center or isolate a page.
    { "fit_page", "p", function() mmv.pdf.fit_page() end },
  }
  for _, binding in ipairs(bindings) do
    local key = keys[binding[1]]
    if key == nil then key = binding[2] end
    if key ~= false then
      mmv.keymap.set(key, binding[3], { mode = "normal" })
    end
  end
end

return M
