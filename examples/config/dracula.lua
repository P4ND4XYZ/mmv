-- Dracula-inspired colors for mmv.
-- Copy to %LOCALAPPDATA%\mmv\init.lua, or merge into your existing config.
-- Restart mmv to apply changes.
local mmv = require("mmv")

local colors = {
  bg = "#282a36",
  fg = "#f8f8f2",
  selection = "#44475a",
  comment = "#6272a4",
  purple = "#bd93f9",
  cyan = "#8be9fd",
  green = "#50fa7b",
  orange = "#ffb86c",
  pink = "#ff79c6",
  red = "#ff5555",
  yellow = "#f1fa8c",
}

mmv.setup({ colorscheme = "dark" })

mmv.set_hl("Normal", { fg = colors.fg, bg = colors.bg })
mmv.set_hl("Border", { fg = colors.purple })
mmv.set_hl("Panel", { fg = colors.fg, bg = colors.bg })
mmv.set_hl("PanelSelected", { fg = colors.fg, bg = colors.selection })
mmv.set_hl("StatusLine", { fg = colors.fg, bg = colors.selection })
mmv.set_hl("CommandLine", { fg = colors.fg, bg = colors.bg })

mmv.set_hl("ModeNormal", { fg = colors.bg, bg = colors.purple })
mmv.set_hl("ModeSearch", { fg = colors.bg, bg = colors.yellow })
mmv.set_hl("ModeCommand", { fg = colors.bg, bg = colors.pink })

mmv.set_hl("Heading1", { fg = colors.purple, bg = colors.bg })
mmv.set_hl("Heading2", { fg = colors.pink, bg = colors.bg })
mmv.set_hl("Heading3", { fg = colors.cyan, bg = colors.bg })
mmv.set_hl("Heading4", { fg = colors.green, bg = colors.bg })
mmv.set_hl("Heading5", { fg = colors.orange, bg = colors.bg })
mmv.set_hl("Heading6", { fg = colors.yellow, bg = colors.bg })

mmv.set_hl("Link", { fg = colors.cyan })
mmv.set_hl("CodeBlock", { fg = colors.fg, bg = colors.selection })
mmv.set_hl("Quote", { fg = colors.comment, bg = colors.bg })
mmv.set_hl("Error", { fg = colors.bg, bg = colors.red })
mmv.set_hl("Warning", { fg = colors.bg, bg = colors.orange })
mmv.set_hl("Search", { bg = "#514b35" })
mmv.set_hl("SearchCurrent", { bg = "#634238" })
