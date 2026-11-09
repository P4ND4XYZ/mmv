use super::{Group, InputState, Item, Palette, input_element, item_element, match_style};
use gpui_kit::{
    AppContext as _, Context, FontWeight, IntoElement, Render, Styled as _, TestAppContext, Window,
    WindowOptions, component::Theme, div, rgb,
};

#[gpui_kit::test]
fn selection_panel_palette_applies_to_input_and_both_row_states_without_overriding_match_colors(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let config = crate::config::execute(
            r##"
            local m = require('mmv')
            m.set_hl('Panel', {fg='#123456', bg='#234567'})
            m.set_hl('PanelSelected', {fg='#345678', bg='#456789'})
        "##,
            "selection-panel-test",
        )
        .unwrap();
        let palette = Palette::resolve(&config, Theme::global(cx));
        let item = Item {
            id: 42,
            primary: "日本語 design".into(),
            secondary: "#日本語-design".into(),
            badge: "Test",
            primary_ranges: vec![0..3, 10..11],
            secondary_ranges: vec![1..4, 11..12],
        };
        for (selected, fg, bg) in [(false, 0x123456, 0x234567), (true, 0x345678, 0x456789)] {
            let mut row = item_element(&item, selected, &palette);
            assert_eq!(row.style().text.color, Some(rgb(fg).into()));
            assert_eq!(row.style().background, Some(rgb(bg).into()));
        }
        let _ = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let mut input = input_element(&state, palette.get(Group::Panel));
            assert_eq!(input.style().text.color, Some(rgb(0x123456).into()));
            assert_eq!(input.style().background, Some(rgb(0x234567).into()));
            cx.new(|_| Empty)
        })
        .unwrap();
        let emphasis = match_style();
        assert_eq!(emphasis.font_weight, Some(FontWeight::BOLD));
        assert_eq!(
            emphasis.color, None,
            "matching must retain PanelSelected.fg"
        );
        assert_eq!(
            emphasis.background_color, None,
            "matching must retain PanelSelected.bg"
        );
    });
}

struct Empty;
impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
