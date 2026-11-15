# Optional Lua plugins

The application does not discover or load plugins automatically. With no user
configuration, `f` is unbound. `link_picker.lua` registers `f` in normal mode
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
`link_picker.lua`, plus a few directly related handle methods, are declared.
Settings/highlight configuration, buttons, additional list controls, and other
APIs are not fully described. Opaque userdata are modeled as classes; invalid
arguments raise errors, and event-phase / render-phase restrictions still apply.
Descriptions are opaque values, source ranges belong to a particular immutable
snapshot, and list item IDs must be unique strings or integers.
