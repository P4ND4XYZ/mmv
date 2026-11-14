//! Explicitly selected native-font feasibility check; never starts the application.
use gpui_kit::{Font, TestApp};

#[test]
#[ignore = "constructs a hidden native platform host; run in its own test process"]
fn strict_reveal_native_backend_gate() {
    let platform = gpui_kit::platform::current_platform(false);
    let backend = platform.text_system();
    let fonts = backend.all_font_names();
    assert!(
        !fonts.is_empty(),
        "native font enumeration is empty; no Noop fallback is permitted"
    );
    let font = backend
        .font_id(&Font {
            family: "Segoe UI".into(),
            ..Default::default()
        })
        .expect("native Segoe UI font");
    let width = |ch| {
        backend
            .advance(
                font,
                backend.glyph_for_char(font, ch).expect("native glyph"),
            )
            .expect("native glyph advance")
            .width
    };
    assert!(
        width('W') > width('i'),
        "expected proportional native glyph advances"
    );
    let mut app = TestApp::with_text_system(backend.clone());
    app.update(gpui_kit::init);
    eprintln!(
        "NATIVE_GATE fonts={} proportional_advances=true",
        fonts.len()
    );
    drop(app);
    let mut cx = gpui_kit::TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        Some("strict_reveal_native_geometry_gate"),
        backend,
    );
    super::strict_reveal::geometry_gate(&mut cx);
    super::strict_reveal::strict_contract_cases(&mut cx);
    super::strict_reveal::strict_private_case(&mut cx);
    super::strict_reveal::strict_partial_row_cases(&mut cx);
    super::strict_reveal::strict_native_complex_text(&mut cx);
    drop(cx);
    drop(platform);
}
