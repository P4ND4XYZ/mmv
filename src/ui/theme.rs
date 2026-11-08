use std::collections::BTreeMap;

use gpui_kit::base::text::TextViewStyle;
use gpui_kit::component::Theme;
use gpui_kit::{HighlightStyle, Hsla, StyleRefinement, prelude::*, rgb};

use crate::config::{Color, Config, Group};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Colors {
    pub fg: Hsla,
    pub bg: Hsla,
}

#[derive(Clone)]
pub(super) struct Palette {
    groups: BTreeMap<Group, Colors>,
}

fn color(Color(r, g, b): Color) -> Hsla {
    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
}

impl Palette {
    pub fn resolve(config: &Config, theme: &Theme) -> Self {
        use Group::*;
        let normal = Colors {
            fg: theme.foreground,
            bg: theme.background,
        };
        let mut groups = BTreeMap::new();
        for group in [
            Normal,
            Border,
            Search,
            SearchCurrent,
            StatusLine,
            CommandLine,
            ModeNormal,
            ModeSearch,
            ModeCommand,
            Heading1,
            Heading2,
            Heading3,
            Heading4,
            Heading5,
            Heading6,
            Link,
            CodeBlock,
            Quote,
            Panel,
            PanelSelected,
            Error,
            Warning,
        ] {
            let mut value = match group {
                Border => Colors {
                    fg: theme.border,
                    ..normal
                },
                Search => Colors {
                    bg: rgb(if theme.is_dark() { 0x665c28 } else { 0xffe28a }).into(),
                    ..normal
                },
                SearchCurrent => Colors {
                    bg: rgb(if theme.is_dark() { 0x995500 } else { 0xffb347 }).into(),
                    ..normal
                },
                ModeNormal => Colors {
                    fg: rgb(0xffffff).into(),
                    bg: rgb(0x2563eb).into(),
                },
                ModeSearch => Colors {
                    fg: rgb(0x111827).into(),
                    bg: rgb(0xfbbf24).into(),
                },
                ModeCommand => Colors {
                    fg: rgb(0xffffff).into(),
                    bg: rgb(0x7c3aed).into(),
                },
                Link => Colors {
                    fg: theme.link,
                    bg: gpui_kit::transparent_black(),
                },
                CodeBlock => Colors {
                    fg: theme.foreground,
                    bg: theme.muted,
                },
                Quote => Colors {
                    fg: theme.muted_foreground,
                    bg: theme.background,
                },
                PanelSelected => Colors {
                    fg: theme.foreground,
                    bg: theme.accent,
                },
                Error => Colors {
                    fg: rgb(0xffffff).into(),
                    bg: rgb(0xb91c1c).into(),
                },
                Warning => Colors {
                    fg: rgb(0x111827).into(),
                    bg: rgb(0xfbbf24).into(),
                },
                _ => normal,
            };
            if let Some(override_) = config.highlights.get(&group) {
                if let Some(fg) = override_.fg {
                    value.fg = color(fg);
                }
                if let Some(bg) = override_.bg {
                    value.bg = color(bg);
                }
            }
            groups.insert(group, value);
        }
        Self { groups }
    }

    pub fn get(&self, group: Group) -> Colors {
        self.groups[&group]
    }

    pub fn text_style(
        &self,
        theme: &Theme,
        mut table: StyleRefinement,
        mut code: StyleRefinement,
    ) -> TextViewStyle {
        use Group::*;
        let normal = self.get(Normal);
        let block = self.get(CodeBlock);
        let quote = self.get(Quote);
        let radius = theme.semantic_tokens().radius.md;
        table.corner_radii.top_left = Some(radius.into());
        table.corner_radii.top_right = Some(radius.into());
        table.corner_radii.bottom_left = Some(radius.into());
        table.corner_radii.bottom_right = Some(radius.into());
        code.corner_radii = table.corner_radii.clone();
        code = code.bg(block.bg).text_color(block.fg);
        let headings =
            [Heading1, Heading2, Heading3, Heading4, Heading5, Heading6].map(|g| self.get(g));
        TextViewStyle::default()
            .with_foreground(normal.fg)
            .with_muted_foreground(theme.muted_foreground)
            .with_link(self.get(Link).fg)
            .with_border(self.get(Border).fg)
            .with_selection(theme.selection)
            .with_code_background(block.bg)
            .with_dark(theme.is_dark())
            .with_inline_code(HighlightStyle {
                background_color: Some(theme.accent),
                ..Default::default()
            })
            .with_table(table)
            .with_table_head(
                StyleRefinement::default()
                    .bg(theme.table_head)
                    .text_color(theme.table_head_foreground),
            )
            .with_code_block(code)
            .with_blockquote_style(StyleRefinement::default().bg(quote.bg).text_color(quote.fg))
            .with_heading(move |level| {
                let color = headings[usize::from(level.saturating_sub(1)).min(5)];
                StyleRefinement::default().bg(color.bg).text_color(color.fg)
            })
    }
}
