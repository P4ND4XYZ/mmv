use super::*;
use crate::config::Group;
use gpui_kit::{
    StyleRefinement, Styled as _,
    component::{Theme, ThemeMode},
    rgb,
};

#[gpui_kit::test]
fn resolved_groups_and_public_text_style_preserve_attribute_overrides(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let config = crate::config::execute(
            r##"
            local m=require('mmv')
            m.set_hl('Heading1',{fg='#100001',bg='#200001'})
            m.set_hl('Heading2',{fg='#100002',bg='#200002'})
            m.set_hl('Heading3',{fg='#123456'})
            m.set_hl('Heading3',{bg='#abcdef'})
            m.set_hl('Heading4',{fg='#100004',bg='#200004'})
            m.set_hl('Heading5',{fg='#100005',bg='#200005'})
            m.set_hl('Heading6',{fg='#100006',bg='#200006'})
            m.set_hl('Link',{fg='#112233',bg='#445566'})
            m.set_hl('CodeBlock',{fg='#778899',bg='#aabbcc'})
            m.set_hl('Quote',{fg='#123123',bg='#456456'})
            m.set_hl('Panel',{fg='#010203'})
            m.set_hl('PanelSelected',{fg='#0a0b0c',bg='#0d0e0f'})
            m.set_hl('Normal',{fg='#112211',bg='#334433'})
        "##,
            "test",
        )
        .unwrap();
        let base = super::super::theme::Palette::resolve(&Default::default(), Theme::global(cx));
        let palette = super::super::theme::Palette::resolve(&config, Theme::global(cx));
        assert_eq!(palette.get(Group::Panel).bg, base.get(Group::Panel).bg);
        assert_eq!(palette.get(Group::Panel).fg, rgb(0x010203).into());
        assert_eq!(palette.get(Group::PanelSelected).fg, rgb(0x0a0b0c).into());
        assert_eq!(palette.get(Group::PanelSelected).bg, rgb(0x0d0e0f).into());
        assert_eq!(palette.get(Group::Normal).bg, rgb(0x334433).into());
        let style = palette.text_style(
            Theme::global(cx),
            StyleRefinement::default(),
            StyleRefinement::default(),
        );
        assert_eq!(style.heading(3).text.color, Some(rgb(0x123456).into()));
        assert_eq!(style.heading(3).background, Some(rgb(0xabcdef).into()));
        for level in [1_u8, 2, 4, 5, 6] {
            assert_eq!(
                style.heading(level).text.color,
                Some(rgb(0x100000 + u32::from(level)).into())
            );
            assert_eq!(
                style.heading(level).background,
                Some(rgb(0x200000 + u32::from(level)).into())
            );
        }
        assert_eq!(style.foreground(), rgb(0x112211).into());
        assert_eq!(style.link(), rgb(0x112233).into());
        assert_eq!(style.code_block().text.color, Some(rgb(0x778899).into()));
        assert_eq!(style.code_block().background, Some(rgb(0xaabbcc).into()));
        assert_eq!(
            style.blockquote_style().text.color,
            Some(rgb(0x123123).into())
        );
        assert_eq!(
            style.blockquote_style().background,
            Some(rgb(0x456456).into())
        );
    });
}

#[gpui_kit::test]
fn scheme_switch_changes_component_syntax_theme_and_installs_highlighter(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let dark = Theme::global(cx).highlight_theme.clone();
        assert!(gpui_kit::base::TextViewDefaults::global(cx).has_code_block_highlighter());
        Theme::change(ThemeMode::Light, None, cx);
        let light = Theme::global(cx).highlight_theme.clone();
        assert!(gpui_kit::base::TextViewDefaults::global(cx).has_code_block_highlighter());
        assert_ne!(
            dark.style.syntax.style("keyword").and_then(|s| s.color),
            light.style.syntax.style("keyword").and_then(|s| s.color)
        );
        assert!(
            dark.style
                .syntax
                .style("keyword")
                .and_then(|s| s.color)
                .is_some()
        );
        assert_eq!(dark.appearance, ThemeMode::Dark);
        assert_eq!(light.appearance, ThemeMode::Light);
        assert_ne!(
            super::super::theme::Palette::resolve(&Default::default(), Theme::global(cx))
                .get(Group::Normal)
                .bg,
            {
                Theme::change(ThemeMode::Dark, None, cx);
                super::super::theme::Palette::resolve(&Default::default(), Theme::global(cx))
                    .get(Group::Normal)
                    .bg
            }
        );
    });
}

#[gpui_kit::test]
fn viewer_selects_configured_theme_before_resolving_colors(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        for (scheme, mode) in [
            (crate::config::ColorScheme::Light, ThemeMode::Light),
            (crate::config::ColorScheme::Dark, ThemeMode::Dark),
        ] {
            let config = crate::config::Config {
                colorscheme: scheme,
                ..Default::default()
            };
            let (_, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Viewer::with_config(PathBuf::new(), Instant::now(), config, false, window, cx)
                })
            })
            .unwrap();
            assert_eq!(Theme::global(cx).mode, mode);
            assert_eq!(Theme::global(cx).highlight_theme.appearance, mode);
            assert_eq!(
                view.read(cx).palette.get(Group::Normal).bg,
                Theme::global(cx).background
            );
            assert!(gpui_kit::base::TextViewDefaults::global(cx).has_code_block_highlighter());
        }
    });
}

#[gpui_kit::test]
fn footer_styles_and_notification_severity_follow_state(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "body".into());
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config = crate::config::execute(
                r##"local m=require('mmv');
            m.set_hl('StatusLine',{fg='#111111',bg='#222222'});
            m.set_hl('CommandLine',{fg='#333333',bg='#444444'});
            m.set_hl('Warning',{fg='#555555',bg='#666666'});
            m.set_hl('Error',{fg='#777777',bg='#888888'});
            m.set_hl('ModeNormal',{fg='#100001',bg='#200001'});
            m.set_hl('ModeSearch',{fg='#100002',bg='#200002'});
            m.set_hl('ModeCommand',{fg='#100003',bg='#200003'});
            m.set_hl('Border',{fg='#abcdef'})"##,
                "test",
            )
            .unwrap();
            viewer.palette =
                super::super::theme::Palette::resolve(&viewer.config, Theme::global(cx));
            let mut status = super::super::status::render_status_line(
                &viewer.app.mode,
                &viewer.path,
                &viewer.palette,
            );
            assert_eq!(status.style().text.color, Some(rgb(0x111111).into()));
            assert_eq!(status.style().background, Some(rgb(0x222222).into()));
            let mut line = super::super::status::render_command_line(
                &viewer.app.mode,
                &viewer.input.state,
                String::new(),
                None,
                &viewer.palette,
            );
            assert_eq!(line.style().text.color, Some(rgb(0x333333).into()));
            assert_eq!(line.style().background, Some(rgb(0x444444).into()));
            let mut prompt = super::super::status::render_prompt(
                &viewer.input.state,
                viewer.palette.get(Group::CommandLine).fg,
            );
            assert_eq!(prompt.style().text.color, Some(rgb(0x333333).into()));
            for (mode, index) in [
                (crate::app::Mode::Normal, 1),
                (crate::app::Mode::Search, 2),
                (crate::app::Mode::Command, 3),
            ] {
                let mut label = super::super::status::render_mode(&mode, &viewer.palette);
                assert_eq!(label.style().text.color, Some(rgb(0x100000 + index).into()));
                assert_eq!(label.style().background, Some(rgb(0x200000 + index).into()));
            }
            assert_eq!(viewer.status_severity(), None);
            viewer.config_notice = true;
            assert_eq!(viewer.status_severity(), Some(Group::Error));
            viewer.message = Some("notice".into());
            viewer.message_is_error = false;
            assert_eq!(viewer.status_severity(), Some(Group::Warning));
            viewer.display_error = Some("failure".into());
            assert_eq!(viewer.status_severity(), Some(Group::Error));
            viewer.display_error = None;
            viewer.message = None;
            viewer.config_notice = false;
            cx.notify();
        })
    });
    cx.update_window(window, |_, window, cx| {
        window.press(":", cx);
        window.input("unknown", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.status_severity(), Some(Group::Warning));
        let mut line = super::super::status::render_command_line(
            &viewer.app.mode,
            &viewer.input.state,
            viewer.status_text(),
            viewer.status_severity(),
            &viewer.palette,
        );
        assert_eq!(line.style().text.color, Some(rgb(0x555555).into()));
        assert_eq!(line.style().background, Some(rgb(0x666666).into()));
    });
}

#[gpui_kit::test]
fn html_block_anchor_keeps_link_click_without_link_background(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "<div><a href=\"other.md\">HTMLリンク</a></div>".into());
    cx.run_until_parked();
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    view.read_with(cx, |viewer, _| {
        assert!(viewer.range_highlights().is_empty())
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    visual.simulate_click(body.origin + point(px(20.), px(20.)), Default::default());
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(requests.borrow()[0].path.file_name().unwrap(), "other.md");
}

#[gpui_kit::test]
fn markdown_link_backgrounds_survive_nohl_and_exclude_html_and_images(cx: &mut TestAppContext) {
    let source = "# [見出し](a)\n\n[日本 **語**](b) and [ref][id] and [前![alt](x.png)後](c) and [![only](x.png)](c)\n\n| col |\n| --- |\n| [表リンク](d) |\n\n<https://example.org>\n\n<div><a href=\"e\">HTMLリンク</a></div>\n\n[id]: https://example.com\n";
    let (window, view) = open(cx, source.into());
    cx.run_until_parked();
    cx.update(|cx| view.update(cx, |viewer, cx| {
        viewer.config = crate::config::execute("local m=require('mmv'); m.set_hl('Link',{bg='#123456'}); m.set_hl('Search',{bg='#654321'}); m.set_hl('SearchCurrent',{bg='#abcdef'})", "test").unwrap();
        viewer.palette = super::super::theme::Palette::resolve(&viewer.config, Theme::global(cx));
        viewer.apply_search(false, cx);
    }));
    cx.run_until_parked();
    view.read_with(cx, |viewer, cx| {
        let text = viewer.document.text.read(cx).rendered_text();
        let ranges = viewer.range_highlights();
        let labels = ranges
            .iter()
            .map(|h| &text.as_str()[h.range()])
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "見出し",
                "日本 語",
                "ref",
                "前後",
                "表リンク",
                "https://example.org"
            ]
        );
        assert!(
            ranges
                .iter()
                .all(|h| h.background() == rgb(0x123456).into())
        );
        assert!(text.as_str().contains("HTMLリンク"));
        assert!(!labels.contains(&"HTMLリンク"));
    });
    cx.update_window(window, |_, window, cx| {
        window.press("/", cx);
        window.input("日本", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |viewer, _| {
        let h = viewer.range_highlights();
        assert_eq!(h.len(), 7);
        assert_eq!(h.last().unwrap().background(), rgb(0xabcdef).into());
    });
    cx.update_window(window, |_, window, cx| {
        window.press(":", cx);
        window.input("noh", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |viewer, _| {
        let h = viewer.range_highlights();
        assert_eq!(h.len(), 6);
        assert!(h.iter().all(|h| h.background() == rgb(0x123456).into()));
        assert!(viewer.display_error.is_none());
    });
}
