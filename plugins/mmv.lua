---@meta
-- Editor-only, partial public API declarations for link_picker.lua and pdf_zoom.lua.
-- Never execute this file or put its directory on package.path.

---@alias mmv.Id string|integer
---@alias mmv.Dimension number|{ratio: number} Positive pixels or ratio in (0, 1].
---@alias mmv.ComponentDimension mmv.Dimension|'auto'
---@alias mmv.Keys table<string, fun()>

---@class mmv.Source Opaque immutable source snapshot.
---@class mmv.Target Opaque navigation target tied to a document snapshot.
---@class mmv.SourceRange Zero-based UTF-8 byte range, stop exclusive; tied to a source.
---@field start integer
---@field stop integer
---@class mmv.ByteRange Zero-based UTF-8 byte range, stop exclusive.
---@field start integer
---@field stop integer
---@class mmv.Match
---@field score integer
---@field ranges mmv.ByteRange[]
---@class mmv.Link
---@field id integer
---@field display_name string
---@field destination string
---@field kind 'heading'|'markdown'|'markdown_heading'|'web'
---@field target mmv.Target
---@field source_range? mmv.SourceRange

---@class mmv.Document
local Document = {}
---@return mmv.Link[]
function Document:links() end
---@return mmv.Source
function Document:source() end

---@class mmv.Subscription
local Subscription = {}
---@return boolean canceled False if already inactive.
function Subscription:cancel() end

---@class mmv.State Values are copied on get/set; mutations are forbidden during render.
local State = {}
---@return any
function State:get() end
---@param value any
---@return nil
function State:set(value) end

---@class mmv.ListControl
local ListControl = {}
---@return boolean
function ListControl:select_next() end
---@return boolean
function ListControl:select_previous() end
---@return boolean accepted False for empty, stale, or unsynchronized lists.
function ListControl:confirm() end

---@class mmv.SourceControl
local SourceControl = {}
---@param options {pages: number}
---@return boolean
function SourceControl:scroll(options) end
---@param range mmv.SourceRange
---@param options? {align?: 'start'|'center'|'end'|'nearest'}
---@return boolean
function SourceControl:scroll_to(range, options) end

---@class mmv.Window
local Window = {}
---@return boolean
function Window:is_open() end
---@return boolean closed
function Window:close() end
---@param key string
---@return boolean
function Window:focus(key) end
---@param key string
---@return mmv.ListControl|false control False if window closed; invalid keys raise errors.
function Window:list(key) end
---@param key string
---@return mmv.SourceControl|false control False if window closed; invalid keys raise errors.
function Window:source(key) end

---@class mmv.Description Opaque display description, not an ordinary table.
---@class mmv.LayoutOptions
---@field key? string
---@field width? mmv.ComponentDimension
---@field height? mmv.ComponentDimension
---@field grow? number Nonnegative.
---@field padding? number Nonnegative pixels.
---@field hl? string Existing highlight group name.
---@class mmv.ContainerOptions: mmv.LayoutOptions
---@field children mmv.Description[] Dense array.
---@field gap? number Nonnegative pixels.
---@field align? 'start'|'center'|'end'|'stretch'
---@class mmv.TextHighlight
---@field range mmv.ByteRange
---@field hl? string
---@field bold? boolean
---@field underline? boolean
---@class mmv.TextOptions: mmv.LayoutOptions
---@field text string
---@field wrap? boolean
---@field highlights? mmv.TextHighlight[]
---@class mmv.ListItem
---@field id mmv.Id Unique within the list; additional copyable fields are allowed.
---@class mmv.ListOptions: mmv.LayoutOptions
---@field key string
---@field items mmv.ListItem[]
---@field selection mmv.State Holds mmv.Id or nil.
---@field render_item fun(item: mmv.ListItem, active: boolean): mmv.Description Synchronous render-only callback.
---@field empty? mmv.Description
---@field reset_on? string|number|boolean
---@field wrap_navigation? boolean
---@field on_selection_changed? fun(id: mmv.Id|nil) Deferred event callback.
---@field on_confirm? fun(id: mmv.Id) Deferred event callback.
---@class mmv.InputOptions: mmv.LayoutOptions
---@field key string
---@field bind mmv.State Holds a UTF-8 string.
---@field placeholder? string
---@field on_change? fun(value: string)
---@field on_submit? fun()
---@field keys? mmv.Keys
---@class mmv.SourceOptions: mmv.LayoutOptions
---@field key string
---@field source mmv.Source
---@field highlight? mmv.SourceRange Must belong to source.
---@field wrap? true
---@class mmv.FloatOptions
---@field width mmv.Dimension
---@field height mmv.Dimension
---@field placement? 'center'
---@field modal? boolean Defaults to true.
---@field focus? string Focusable component key.
---@field render fun(): mmv.Description Synchronous, exactly one description; no mutations.
---@field dismiss? {escape?: boolean, outside_click?: boolean}
---@field keys? mmv.Keys
---@field on_close? fun(event: {reason: 'programmatic'|'escape'|'outside_click'|'shutdown'})
---@field on_error? fun(error: {message: string, traceback: string, phase: 'event'|'render', key?: string})

---@class mmv.Api
local mmv = { ui = { float = {} }, document = {}, keymap = {}, match = {}, navigation = {}, pdf = {} }

-- Key registration is available in init and event callbacks, not render.
---@param key string Normalized app key; not a built-in key.
---@param callback fun()
---@param options? {mode?: 'normal'}
---@return true
function mmv.keymap.set(key, callback, options) end
---@param key string
---@param options? {mode?: 'normal'}
---@return boolean removed
function mmv.keymap.remove(key, options) end

-- Document reads are unavailable during init.
---@return mmv.Document|nil
function mmv.document.current() end
---@param options {owner: mmv.Window}
---@param callback fun() Deferred after document replacement; canceled on owner close.
---@return mmv.Subscription|false subscription False if owner already closed.
function mmv.document.on_replaced(options, callback) end

---@param query string
---@param text string
---@return mmv.Match|nil
function mmv.match.fuzzy(query, text) end

---@param initial? any Copyable value; omitted means nil.
---@return mmv.State
function mmv.ui.state(initial) end
---@param options mmv.FloatOptions
---@return mmv.Window
function mmv.ui.float.open(options) end
---@param options mmv.ContainerOptions
---@return mmv.Description
function mmv.ui.row(options) end
---@param options mmv.ContainerOptions
---@return mmv.Description
function mmv.ui.column(options) end
---@param options mmv.TextOptions
---@return mmv.Description
function mmv.ui.text(options) end
---@param options mmv.ListOptions
---@return mmv.Description
function mmv.ui.list(options) end
---@param options mmv.InputOptions
---@return mmv.Description
function mmv.ui.input(options) end
---@param options mmv.SourceOptions
---@return mmv.Description
function mmv.ui.source(options) end

---@class mmv.NavigationResult
---@field ok boolean
---@field code? string Failure code when ok=false.
---@field message? string Failure message when ok=false.
---@field warning? {code: string, message: string} Optional warning when ok=true.
---@param target mmv.Target
---@param options? {owner?: mmv.Window, on_complete?: fun(result: mmv.NavigationResult)}
---@return nil Queues a host operation; completion is deferred.
function mmv.navigation.open(target, options) end
---@param message string
---@param options? {level?: 'info'|'warning'|'error'}
---@return true
function mmv.notify(message, options) end

-- PDF modifiers are event-only; no options, owner, or completion callbacks.
-- True means acceptance, not application; false means no PDF at call time.
-- Requests bind the current display and are discarded if it is replaced.
---@param factor number Finite and >0; clamped to [0.01, 3], native tolerance applies.
---@return boolean accepted
function mmv.pdf.set_zoom(factor) end
---@return boolean accepted Multiplies applied zoom by 1.1; disables fit even at limits.
function mmv.pdf.zoom_in() end
---@return boolean accepted Divides applied zoom by 1.1; disables fit even at limits.
function mmv.pdf.zoom_out() end
---@return boolean accepted Idempotent width fit; tracks viewport resize.
function mmv.pdf.fit_width() end
---@return boolean accepted Native topmost-page fit toggle; no centering/whole-page guarantee.
function mmv.pdf.fit_page() end
-- Runtime event/render only, forbidden during init; never a queued future value.
---@return number|nil zoom Actual applied zoom; nil for non-PDF / before initial usable layout.
function mmv.pdf.get_zoom() end

return mmv
