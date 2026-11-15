-- Optional Normal-mode PDF zoom bindings. Load explicitly from your init.lua.
local mmv = require("mmv")

mmv.keymap.set("+", function() mmv.pdf.zoom_in() end)
mmv.keymap.set("-", function() mmv.pdf.zoom_out() end)
mmv.keymap.set("=", function() mmv.pdf.fit_width() end)
-- Native topmost-page fit toggle; this does not center or isolate a page.
mmv.keymap.set("p", function() mmv.pdf.fit_page() end)
