-- Optional link selection is composed exclusively from the public mmv API.
local mmv = require("mmv")
local ui = mmv.ui
local labels = {
  heading = "Heading",
  markdown = "Markdown",
  markdown_heading = "Markdown heading",
  web = "Web",
}

local function highlights(match)
  local result = {}
  if match then
    for _, range in ipairs(match.ranges) do
      result[#result + 1] = { range = range, bold = true }
    end
  end
  return result
end

local function open_links()
  local doc = mmv.document.current()
  if not doc then return end
  local links, source = doc:links(), doc:source()
  local by_id = {}
  for _, link in ipairs(links) do by_id[link.id] = link end
  local query, selection = ui.state(""), ui.state()
  local win

  local function scroll(pages)
    local link = by_id[selection:get()]
    if link and link.source_range then
      win:source("links-source"):scroll({ pages = pages })
    end
  end

  win = ui.float.open({
    width = { ratio = 0.8 }, height = { ratio = 0.8 },
    placement = "center", modal = true, focus = "links-query",
    render = function()
      local text, items = query:get(), {}
      for order, link in ipairs(links) do
        local name = mmv.match.fuzzy(text, link.display_name)
        local destination = mmv.match.fuzzy(text, link.destination)
        if name or destination then
          items[#items + 1] = {
            id = link.id, display_name = link.display_name,
            destination = link.destination, kind = link.kind,
            name_highlights = highlights(name),
            destination_highlights = highlights(destination),
            score = math.max(name and name.score or -math.huge,
                             destination and destination.score or -math.huge),
            order = order,
          }
        end
      end
      table.sort(items, function(a, b)
        if a.score == b.score then return a.order < b.order end
        return a.score > b.score
      end)
      local selected = by_id[selection:get()]
      local right
      if #items == 0 then
        right = ui.text({ key = "links-no-source", text = "", grow = 1 })
      elseif selected and selected.source_range then
        right = ui.source({
          key = "links-source", source = source,
          highlight = selected.source_range, grow = 1, height = { ratio = 1 },
        })
      else
        right = ui.text({
          key = "links-unavailable", text = "Source position unavailable", grow = 1,
        })
      end
      return ui.column({
        key = "links-layout", width = { ratio = 1 }, height = { ratio = 1 },
        children = {
          ui.row({
            key = "links-content", grow = 1,
            children = {
              ui.list({
                key = "links-list", grow = 1, height = { ratio = 1 },
                items = items, selection = selection, reset_on = text,
                empty = ui.text({ key = "links-empty", text = "No matching links" }),
                render_item = function(item, active)
                  return ui.column({
                    key = "links-item-" .. tostring(item.id),
                    hl = active and "PanelSelected" or "Panel", padding = 4,
                    children = {
                      ui.text({ text = item.display_name, highlights = item.name_highlights }),
                      ui.text({ text = item.destination, highlights = item.destination_highlights }),
                      ui.text({ text = labels[item.kind] }),
                    },
                  })
                end,
                on_selection_changed = function(id)
                  local link = by_id[id]
                  if link and link.source_range then
                    win:source("links-source"):scroll_to(link.source_range, { align = "center" })
                  end
                end,
                on_confirm = function(id)
                  local target = by_id[id].target
                  win:close()
                  mmv.navigation.open(target)
                end,
              }),
              right,
            },
          }),
          ui.input({
            key = "links-query", bind = query, height = 32,
            placeholder = "Search links",
            on_submit = function() win:list("links-list"):confirm() end,
            keys = {
              down = function() win:list("links-list"):select_next() end,
              up = function() win:list("links-list"):select_previous() end,
              ["ctrl-n"] = function() win:list("links-list"):select_next() end,
              ["ctrl-p"] = function() win:list("links-list"):select_previous() end,
              ["ctrl-u"] = function() scroll(-0.5) end,
              ["ctrl-d"] = function() scroll(0.5) end,
            },
          }),
        },
      })
    end,
  })
  mmv.document.on_replaced({ owner = win }, function()
    win:close()
    mmv.notify("Link selection closed because the document changed", { level = "warning" })
  end)
end

mmv.keymap.set("f", open_links, { mode = "normal" })
