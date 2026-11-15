# Optional Lua plugins

The application does not discover or load plugins automatically. With no user
configuration, `f`, `+`, `-`, `=`, and `p` are unbound. `link_picker.lua` registers `f` in normal mode
only when your own `init.lua` loads it. It provides fuzzy link selection,
source previews, navigation, and closes its window on document replacement.

## Install and enable

Copy `link_picker.lua` to `%LOCALAPPDATA%/mmv/plugins/link_picker.lua`.
Create or edit your user-managed `%LOCALAPPDATA%/mmv/init.lua`:

```lua
local base = assert(os.getenv("LOCALAPPDATA"), "LOCALAPPDATA is required")
-- The loader requires an absolute LOCALAPPDATA path; no process cwd dependency.
dofile(base .. "/mmv/plugins/link_picker.lua")
```

The application never creates or updates this configuration for you. Keep other
settings in the same file. Registrations can be overridden or removed after
loading the plugin:

```lua
require("mmv").keymap.remove("f")
```

If any uncaught init error (including a missing plugin file) or invalid settings /
key registration occurs, the entire candidate VM is discarded. Defaults are
restored with no `f` binding, not with a built-in copy of the plugin.

## Optional PDF zoom keys

Copy `pdf_zoom.lua` to `%LOCALAPPDATA%/mmv/plugins/pdf_zoom.lua` and explicitly
load it from your own `init.lua` (independently of the link picker):

```lua
local base = assert(os.getenv("LOCALAPPDATA"), "LOCALAPPDATA is required")
dofile(base .. "/mmv/plugins/pdf_zoom.lua")
-- Optional overrides/removal after loading:
-- require("mmv").keymap.set("+", function() require("mmv").pdf.set_zoom(1.5) end)
-- require("mmv").keymap.remove("p")
```

The plugin registers Normal-mode `+` / `-` (multiply/divide applied zoom by
1.1), `=` (idempotent width fit), and `p` (native page-fit **toggle**).
Without explicit loading there are no built-in PDF zoom keys. Non-PDF calls
are harmless no-ops. Existing app-keymap routing protects Command/Search,
native text editing, and IME composition. App Normal mappings have no body-focus
condition: an unhandled key in the Normal tree can also reach the plugin.

`mmv.pdf.set_zoom(factor)`, `zoom_in()`, `zoom_out()`, `fit_width()`, and
`fit_page()` return acceptance only: true for a request bound to the current
PDF display, false when no PDF is present. They accept no options/callbacks,
are event-only, and do not change mode/focus. `get_zoom()` is also allowed in
runtime render callbacks; it returns the actual applied zoom, or nil for
non-PDF / before the first usable layout. All are unavailable during init.
Invalid arity/types raise Lua errors; `set_zoom` requires a finite positive
number and clamps it to [0.01, 3]. Native floating-point tolerance applies.

Page fit uses the **topmost** page, not the viewport-center page. It does not
reposition the page to the center or guarantee whole-page visibility. While
enabled, resize re-fits the current topmost page, not a saved target; scrolling
alone does not re-fit. Calling again turns it off without resetting zoom.
Manual zoom disables either fit mode, including at zoom limits. Deferred
requests stay ordered and never migrate to a replacement display.
See the [PDF zoom contract](../docs/requirements/features/pdf-zoom.md).

## Editor annotations

`mmv.lua` is an editor-only LuaLS/EmmyLua `---@meta` file, **not** a runtime
implementation. Add its directory as an absolute LuaLS workspace library path
in your editor, for example in editor settings:

```json
{
  "Lua.runtime.version": "Lua 5.4",
  "Lua.workspace.library": ["C:/path/to/my-markdown-viewer/plugins"]
}
```

Do not execute `mmv.lua`, copy it into a runtime module directory, prepend this
directory to `package.path`, or use it as a replacement for `require("mmv")`.
The host supplies the real `mmv` module. The `dofile` example above needs no
`package.path` changes and does not risk loading the type stub.

Coverage is intentionally partial: all public API surfaces used by
`link_picker.lua` and `pdf_zoom.lua`, plus a few directly related handle methods, are declared.
Settings/highlight configuration, buttons, additional list controls, and other
APIs are not fully described. Opaque userdata are modeled as classes; invalid
arguments raise errors, and event-phase / render-phase restrictions still apply.
Descriptions are opaque values, source ranges belong to a particular immutable
snapshot, and list item IDs must be unique strings or integers.
