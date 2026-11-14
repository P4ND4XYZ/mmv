use super::*;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, test::TestWindowExt as _,
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}
fn ready(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<Viewer>) {
    for _ in 0..20 {
        frame(cx, window);
        if view.read_with(cx, |v, _| v.app.position_request().is_none()) {
            return;
        }
    }
    panic!("position did not settle within 20 frames");
}
fn event(view: &Entity<Viewer>, cx: &mut TestAppContext, source: &str) {
    view.update(cx, |v, cx| {
        let function = v
            .lua
            .lua()
            .load(format!("return function() {source} end"))
            .eval::<mlua::Function>()
            .unwrap();
        let ctx = v.lua.context();
        let callback = ctx
            .register_once(v.lua.lua(), function, crate::lua::CallbackScope::app())
            .unwrap()
            .unwrap();
        ctx.enqueue(v.lua.lua(), callback, ()).unwrap();
        v.pump_lua(cx);
        cx.notify();
    });
}
fn assert_lua(view: &Entity<Viewer>, cx: &TestAppContext, source: &str) {
    view.read_with(cx, |v, _| v.lua.lua().load(source).exec().unwrap());
}
fn outcome(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<Viewer>) {
    for _ in 0..30 {
        frame(cx, window);
        if view.read_with(cx, |v, _| {
            !v.lua
                .lua()
                .globals()
                .get::<mlua::Value>("result")
                .unwrap()
                .is_nil()
        }) {
            return;
        }
    }
    panic!("completion was not delivered within 30 frames");
}

#[gpui_kit::test]
fn lua_snapshots_use_real_markdown_extraction_and_keep_the_first_usage(cx: &mut TestAppContext) {
    let source = "日本語\r\n\r\n[Go](next.md) [Go](./next.md)\r\n\r\n[参照][id]\r\n\r\n[before ![alt](image.png) after](mixed.md)\r\n\r\n[id]: other.md\r\n\r\n<p><a href=\"hidden.md\">Hidden</a></p>";
    let (window, view) = super::super::tests::open(cx, source.into());
    ready(cx, window, &view);
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); doc=m.document.current(); links=doc:links()
        assert(#links==3 and links[1].display_name=='Go' and links[2].display_name=='参照' and links[3].display_name=='before alt after')
        assert(links[1].source_range.start==13 and links[1].source_range.stop==26)
        assert(links[2].source_range.start < links[3].source_range.start)
        assert(doc==m.document.current() and doc:source()==m.document.current():source())
        checked=true
    "#,
    );
    assert_lua(&view, cx, "assert(checked)");
}

#[gpui_kit::test]
fn same_document_heading_completes_after_position_and_missing_heading_notifies_once(
    cx: &mut TestAppContext,
) {
    let source = format!(
        "[One](#one) [Missing](#absent)\n\n{}\n# One\n\nend",
        "paragraph\n\n".repeat(80)
    );
    let (window, view) = super::super::tests::open(cx, source);
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    let generation = view.read_with(cx, |v, _| v.app.document().unwrap().generation);
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); local u=m.ui; replaced=0
        w=u.float.open{width=100,height=100,render=function()return u.text{text='x'}end}
        sub=m.document.on_replaced({owner=w},function()replaced=replaced+1 end)
        m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)result=r end})
        assert(result==nil)
    "#,
    );
    outcome(cx, window, &view);
    assert_lua(
        &view,
        cx,
        "assert(result.ok and result.warning==nil and replaced==0)",
    );
    view.read_with(cx, |v, cx| {
        assert!(v.document.distance(cx) > 500.);
        assert_eq!(v.app.document().unwrap().generation, generation);
    });
    assert!(reads.borrow().is_empty());
    event(
        &view,
        cx,
        "result=nil; local m=require('mmv'); m.navigation.open(m.document.current():links()[2].target,{on_complete=function(r)result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(
        &view,
        cx,
        "assert(result.ok and result.warning.code=='heading_not_found' and replaced==0)",
    );
    view.read_with(cx, |v, cx| {
        assert_eq!(v.document.distance(cx), 0.);
        assert!(v.message.is_none());
    });
    event(
        &view,
        cx,
        "require('mmv').navigation.open(require('mmv').document.current():links()[2].target)",
    );
    ready(cx, window, &view);
    frame(cx, window);
    view.read_with(cx, |v, _| {
        assert_eq!(v.message.as_deref(), Some("Heading not found: absent"))
    });
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mmv-lua-nav-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn url(&self, name: &str) -> String {
        url::Url::from_file_path(self.0.join(name)).unwrap().into()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[gpui_kit::test]
fn runtime_real_file_failures_and_os_launch_results_preserve_body_without_gui_duplicates(
    cx: &mut TestAppContext,
) {
    let temp = Temp::new();
    std::fs::write(temp.0.join("invalid.md"), [255, 254]).unwrap();
    std::fs::create_dir(temp.0.join("folder.md")).unwrap();
    let source = format!(
        "[Missing]({}) [Invalid]({}) [Directory]({}) [Web](https://example.com)",
        temp.url("absent.md"),
        temp.url("invalid.md"),
        temp.url("folder.md")
    );
    let (window, view) = super::super::tests::open(cx, source.clone());
    ready(cx, window, &view);
    cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    let launched = Rc::new(RefCell::new(Vec::<String>::new()));
    let captured = launched.clone();
    view.update(cx, |v, _| {
        v.browser_launcher = Box::new(move |url| {
            captured.borrow_mut().push(url.into());
            Err(std::io::Error::other("controlled launcher rejection"))
        })
    });
    for (index, code) in [
        (1, "file_not_found"),
        (2, "invalid_utf8"),
        (3, "read_failed"),
        (4, "browser_open_failed"),
    ] {
        event(
            &view,
            cx,
            &format!(
                "result=nil; local m=require('mmv'); m.navigation.open(m.document.current():links()[{index}].target,{{on_complete=function(r)result=r end}})"
            ),
        );
        outcome(cx, window, &view);
        assert_lua(
            &view,
            cx,
            &format!("assert(not result.ok and result.code=='{code}')"),
        );
        view.read_with(cx, |v, _| {
            assert_eq!(v.app.document().unwrap().source.as_str(), source);
            assert!(v.app.error().is_none());
            assert!(v.message.is_none());
        });
    }
    assert_eq!(launched.borrow().as_slice(), ["https://example.com/"]);
    view.update(cx, |v, _| v.browser_launcher = Box::new(|_| Ok(())));
    event(
        &view,
        cx,
        "result=nil; local m=require('mmv'); m.navigation.open(m.document.current():links()[4].target,{on_complete=function(r)result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(&view, cx, "assert(result.ok)");
}

#[gpui_kit::test]
fn actual_adoption_notifies_before_completion_owner_close_does_not_cancel_navigation(
    cx: &mut TestAppContext,
) {
    let temp = Temp::new();
    std::fs::write(temp.0.join("next.md"), "# Next\n\n[Self](#next)").unwrap();
    let (window, view) =
        super::super::tests::open(cx, format!("[Next]({}#next)", temp.url("next.md")));
    ready(cx, window, &view);
    cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); local u=m.ui; events=''; old=m.document.current(); saved=old:links()[1].target
        w=u.float.open{width=100,height=100,render=function()return u.text{text='x'}end}
        m.document.on_replaced({owner=w},function()
            assert(m.document.current()~=old); events=events..'R'; w:close()
        end)
        m.navigation.open(saved,{owner=w,on_complete=function()events=events..'C' end})
    "#,
    );
    ready(cx, window, &view);
    frame(cx, window);
    assert_lua(
        &view,
        cx,
        "assert(events=='R' and not w:is_open()); assert(old:links()[1].target==saved)",
    );
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.app.document().unwrap().source.as_str(),
            "# Next\n\n[Self](#next)"
        );
        assert!(v.message.is_none());
    });
    event(
        &view,
        cx,
        "result=nil; require('mmv').navigation.open(saved,{on_complete=function(r)result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(&view, cx, "assert(result.code=='stale_target')");
    event(
        &view,
        cx,
        "result=nil; local m=require('mmv'); m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)events=events..'C';result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(&view, cx, "assert(result.ok and events=='RC')");
}

#[gpui_kit::test]
fn later_requests_and_reverse_completions_supersede_once_and_only_adoption_notifies(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::super::tests::open(cx, "[A](a.md) [B](b.md)".into());
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); local u=m.ui; replacements=0; calls=0
        w=u.float.open{width=100,height=100,render=function()return u.text{text='x'}end}
        m.document.on_replaced({owner=w},function()replacements=replacements+1 end)
        local links=m.document.current():links()
        m.navigation.open(links[1].target,{on_complete=function(r)first=r; calls=calls+1 end})
        m.navigation.open(links[2].target,{on_complete=function(r)result=r; calls=calls+1 end})
    "#,
    );
    cx.run_until_parked();
    assert_eq!(reads.borrow().len(), 2);
    assert_lua(
        &view,
        cx,
        "assert(first.code=='superseded' and calls==1 and replacements==0)",
    );
    let second = reads.borrow_mut().pop().unwrap();
    let first = reads.borrow_mut().pop().unwrap();
    view.update(cx, |v, cx| {
        v.complete(
            crate::app::ReadResult {
                request: second.clone(),
                content: Ok(crate::app::LoadedDocument {
                    key: crate::app::DocumentKey(second.path),
                    source: "# Accepted".into(),
                }),
            },
            cx,
        )
    });
    outcome(cx, window, &view);
    view.update(cx, |v, cx| {
        v.complete(
            crate::app::ReadResult {
                request: first,
                content: Err("old failure".into()),
            },
            cx,
        )
    });
    frame(cx, window);
    assert_lua(
        &view,
        cx,
        "assert(result.ok and first.code=='superseded' and calls==2 and replacements==1 and w:is_open())",
    );
}

#[gpui_kit::test]
fn unowned_completion_runs_after_adoption_subscribers_and_applied_position(
    cx: &mut TestAppContext,
) {
    let temp = Temp::new();
    std::fs::write(temp.0.join("target.md"), "# Here").unwrap();
    let (window, view) =
        super::super::tests::open(cx, format!("[Target]({}#here)", temp.url("target.md")));
    ready(cx, window, &view);
    cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); events=''
        w=m.ui.float.open{width=100,height=100,render=function()return m.ui.text{text='x'}end}
        m.document.on_replaced({owner=w},function()events=events..'R';w:close()end)
        m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)assert(events=='R');events=events..'C';result=r end})
    "#,
    );
    outcome(cx, window, &view);
    assert_lua(&view, cx, "assert(events=='RC' and result.ok)");
    view.read_with(cx, |v, _| assert!(v.app.position_request().is_none()));
}

#[gpui_kit::test]
fn host_failure_and_closed_owner_delivery_rules_do_not_create_fallback_duplicates(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::super::tests::open(cx, "[Next](next.md)".into());
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    for (options, close_before, close_after, callback_expected, fallback) in [
        ("{}", false, false, false, true),
        (
            "{on_complete=function(r)result=r end}",
            false,
            false,
            true,
            false,
        ),
        (
            "{owner=w,on_complete=function(r)result=r end}",
            true,
            false,
            false,
            false,
        ),
        (
            "{owner=w,on_complete=function(r)result=r end}",
            false,
            true,
            false,
            false,
        ),
        (
            "{on_complete=function(r)result=r end}",
            false,
            true,
            true,
            false,
        ),
    ] {
        view.update(cx, |v, _| {
            v.message = None;
            v.message_level = None;
        });
        event(
            &view,
            cx,
            &format!(
                "local m=require('mmv'); result=nil; w=m.ui.float.open{{width=100,height=100,render=function()return m.ui.text{{text='x'}}end}}; {} m.navigation.open(m.document.current():links()[1].target,{options}); {}",
                if close_before { "w:close();" } else { "" },
                if close_after { "w:close();" } else { "" }
            ),
        );
        cx.run_until_parked();
        let request = reads.borrow_mut().pop().unwrap();
        view.update(cx, |v, cx| {
            v.complete(
                crate::app::ReadResult {
                    request,
                    content: Err("Unable to navigate to document".into()),
                },
                cx,
            )
        });
        frame(cx, window);
        if callback_expected {
            assert_lua(&view, cx, "assert(result.code=='navigation_failed')");
        } else {
            assert_lua(&view, cx, "assert(result==nil)");
        }
        view.read_with(cx, |v, _| {
            assert_eq!(v.message.is_some(), fallback);
            assert_eq!(v.app.document().unwrap().source.as_str(), "[Next](next.md)");
        });
        event(&view, cx, "w:close()");
    }
}

#[cfg(windows)]
#[gpui_kit::test]
fn real_windows_file_lock_is_classified_as_access_denied(cx: &mut TestAppContext) {
    use std::os::windows::fs::OpenOptionsExt as _;
    let temp = Temp::new();
    let path = temp.0.join("locked.md");
    std::fs::write(&path, "# Locked").unwrap();
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let (window, view) =
        super::super::tests::open(cx, format!("[Locked]({})", temp.url("locked.md")));
    ready(cx, window, &view);
    cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    event(
        &view,
        cx,
        "local m=require('mmv'); m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(
        &view,
        cx,
        "assert(not result.ok and result.code=='access_denied')",
    );
    view.read_with(cx, |v, _| assert!(v.message.is_none()));
}

#[gpui_kit::test]
fn adopted_position_failure_is_warning_success_and_terminal_result_never_changes(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::super::tests::open(cx, "[Next](next.md#one)".into());
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    event(
        &view,
        cx,
        "local m=require('mmv'); calls=0; m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)calls=calls+1;result=r end})",
    );
    cx.run_until_parked();
    let request = reads.borrow_mut().pop().unwrap();
    // Only the analyzer failure is substituted. Acceptance, adoption, history,
    // position ownership, terminal classification and Lua delivery are real.
    view.update(cx, |v, cx| {
        v.complete(
            crate::app::ReadResult {
                request: request.clone(),
                content: Ok(crate::app::LoadedDocument {
                    key: crate::app::DocumentKey(request.path),
                    source: "# One".into(),
                }),
            },
            cx,
        );
        let position = v.app.position_request().unwrap().clone();
        assert!(v.lua_position_unavailable(
            &position,
            "Unable to determine document positions".into(),
            cx
        ));
        assert!(!v.lua_position_unavailable(&position, "late failure".into(), cx));
        v.pump_lua(cx);
    });
    outcome(cx, window, &view);
    frame(cx, window);
    assert_lua(
        &view,
        cx,
        "assert(result.ok and result.warning.code=='position_unavailable' and calls==1)",
    );
    view.read_with(cx, |v, _| {
        assert_eq!(v.app.document().unwrap().source.as_str(), "# One");
        assert!(v.message.is_none());
    });
}

#[gpui_kit::test]
fn superseded_heading_cannot_move_before_a_following_read_failure(cx: &mut TestAppContext) {
    let source = format!(
        "[Old](#bottom) [New](missing.md)\n\n{}\n# Bottom\n\nend",
        "paragraph\n\n".repeat(100)
    );
    let (window, view) = super::super::tests::open(cx, source);
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    let before = view.read_with(cx, |v, cx| v.document.distance(cx));
    assert_eq!(before, 0.);
    event(
        &view,
        cx,
        "local m=require('mmv');local links=m.document.current():links();m.navigation.open(links[1].target,{on_complete=function(r)first=r end});m.navigation.open(links[2].target,{on_complete=function(r)second=r end})",
    );
    for _ in 0..5 {
        frame(cx, window);
    }
    let request = reads.borrow_mut().pop().expect("following navigation read");
    view.update(cx, |v, cx| {
        v.complete(
            crate::app::ReadResult {
                request,
                content: Err("Unable to read file".into()),
            },
            cx,
        )
    });
    frame(cx, window);
    assert_lua(
        &view,
        cx,
        "assert(first.code=='superseded' and not second.ok)",
    );
    view.read_with(cx, |v, cx| {
        assert_eq!(
            v.document.distance(cx),
            before,
            "superseded position must not move the document"
        )
    });
    // A later valid request must still be applied normally, not cancelled by
    // the old request's serial or by the callback owner's lifetime.
    event(
        &view,
        cx,
        "result=nil;local m=require('mmv');m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)result=r end})",
    );
    outcome(cx, window, &view);
    assert_lua(&view, cx, "assert(result.ok)");
    view.read_with(cx, |v, cx| assert!(v.document.distance(cx) > 3000.));
}

#[gpui_kit::test]
fn superseding_an_old_result_preserves_a_newer_position_serial(cx: &mut TestAppContext) {
    let source = format!(
        "[Old](#bottom)\n\n{}\n# Bottom\n\nend",
        "paragraph\n\n".repeat(100)
    );
    let (window, view) = super::super::tests::open(cx, source);
    ready(cx, window, &view);
    view.update(cx, |v, cx| {
        // Do not yield a host turn before replacing the pending serial.
        v.lua.lua().load("local m=require('mmv');m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)first=r end})").exec().unwrap();
        v.pump_lua(cx);
        let old = v.app.position_request().unwrap().clone();
        assert!(matches!(
            v.app.operate(
                DocumentOperation::Heading {
                    path: None,
                    id: "bottom".into()
                },
                OperationOrigin::External,
                v.document.distance(cx),
                None
            ),
            OperationResult::Complete {
                outcome: Ok(()),
                ..
            }
        ));
        let newer = v.app.position_request().unwrap().clone();
        assert_ne!(old.serial, newer.serial);
        assert_eq!(old.generation, newer.generation);
        v.supersede_lua_navigation(v.next_operation_order + 1);
        assert_eq!(v.app.position_request(), Some(&newer));
        v.pump_lua(cx);
    });
    ready(cx, window, &view);
    assert_lua(&view, cx, "assert(first.code=='superseded')");
    view.read_with(cx, |v, cx| assert!(v.document.distance(cx) > 3000.));
}

#[gpui_kit::test]
fn superseded_unapplied_heading_cannot_emit_a_late_gui_warning(cx: &mut TestAppContext) {
    let (window, view) =
        super::super::tests::open(cx, "[Missing](#missing) [Next](next.md)".into());
    ready(cx, window, &view);
    event(
        &view,
        cx,
        r#"
        local m=require('mmv'); local links=m.document.current():links()
        m.navigation.open(links[1].target,{on_complete=function(r)first=r end})
        m.navigation.open(links[2].target,{on_complete=function(r)result=r end})
    "#,
    );
    frame(cx, window);
    frame(cx, window);
    assert_lua(&view, cx, "assert(first.code=='superseded')");
    view.read_with(cx, |v, _| assert!(v.message.is_none()));
}

#[gpui_kit::test]
fn reload_adoption_notifies_but_requests_failures_and_scroll_do_not(cx: &mut TestAppContext) {
    let (window, view) = super::super::tests::open(cx, "[Self](#here)\n\n# Here".into());
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    event(
        &view,
        cx,
        "local m=require('mmv'); replacements=0; w=m.ui.float.open{width=100,height=100,render=function()return m.ui.text{text='x'}end}; m.document.on_replaced({owner=w},function()replacements=replacements+1 end)",
    );
    for succeeds in [false, true] {
        view.update(cx, |v, cx| v.reload(cx));
        cx.run_until_parked();
        assert_lua(&view, cx, "assert(replacements==0 and w:is_open())");
        let request = reads.borrow_mut().pop().unwrap();
        let content = if succeeds {
            Ok(crate::app::LoadedDocument {
                key: crate::app::DocumentKey(request.path.clone()),
                source: "# Updated".into(),
            })
        } else {
            Err("Unable to read file".into())
        };
        view.update(cx, |v, cx| {
            v.complete(crate::app::ReadResult { request, content }, cx)
        });
        ready(cx, window, &view);
    }
    assert_lua(&view, cx, "assert(replacements==1 and w:is_open())");
    view.update(cx, |v, cx| {
        v.request_external(
            crate::app::ExternalOperation::Top,
            crate::app::ExternalRequestId("scroll".into()),
            cx,
        );
        v.document.text.update(cx, |_, cx| cx.notify());
    });
    frame(cx, window);
    assert_lua(&view, cx, "assert(replacements==1 and w:is_open())");
}

#[gpui_kit::test]
fn reload_replacing_an_unapplied_navigation_position_completes_it_as_superseded(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::super::tests::open(cx, "[Next](next.md#one)".into());
    ready(cx, window, &view);
    let reads = Rc::new(RefCell::new(Vec::<crate::app::ReadRequest>::new()));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    event(
        &view,
        cx,
        "local m=require('mmv'); calls=0; events=''; w=m.ui.float.open{width=100,height=100,render=function()return m.ui.text{text='x'}end}; m.document.on_replaced({owner=w},function()events=events..'R' end); m.navigation.open(m.document.current():links()[1].target,{on_complete=function(r)calls=calls+1;result=r;events=events..'C' end})",
    );
    cx.run_until_parked();
    let request = reads.borrow_mut().pop().unwrap();
    view.update(cx, |v, cx| {
        v.complete(
            crate::app::ReadResult {
                request: request.clone(),
                content: Ok(crate::app::LoadedDocument {
                    key: crate::app::DocumentKey(request.path),
                    source: "# One".into(),
                }),
            },
            cx,
        );
        assert!(v.app.position_request().is_some());
        // Deliver the reload before yielding to any render/layout callback.
        let OperationResult::Pending(reload) = v.request_operation(
            DocumentOperation::Reload,
            OperationOrigin::Automatic,
            None,
            cx,
        ) else {
            panic!("reload must read");
        };
        v.complete(
            crate::app::ReadResult {
                request: reload.clone(),
                content: Ok(crate::app::LoadedDocument {
                    key: crate::app::DocumentKey(reload.path),
                    source: "# One updated".into(),
                }),
            },
            cx,
        );
    });
    ready(cx, window, &view);
    frame(cx, window);
    assert_lua(
        &view,
        cx,
        "assert(result and result.code=='superseded' and calls==1 and events=='RRC')",
    );
}

#[gpui_kit::test]
fn notify_keeps_search_and_command_input_and_validates_all_levels(cx: &mut TestAppContext) {
    let (window, view) = super::super::tests::open(cx, "body".into());
    ready(cx, window, &view);
    for (level, group) in [
        ("info", crate::config::Group::CommandLine),
        ("warning", crate::config::Group::Warning),
        ("error", crate::config::Group::Error),
    ] {
        event(
            &view,
            cx,
            &format!("assert(require('mmv').notify('利用者message',{{level='{level}'}}))"),
        );
        view.read_with(cx, |v, _| {
            assert_eq!(v.message.as_deref(), Some("利用者message"));
            assert_eq!(v.status_severity(), Some(group));
        });
    }
    // The real command-line renderer protects input; the notification does not
    // replace or edit the Prompt entity in either mode.
    for (key, mode) in [
        ("/", crate::app::Mode::Search),
        (":", crate::app::Mode::Command),
    ] {
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.input("保持入力", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let prompt = view.read_with(cx, |v, _| v.input.state.clone());
        event(&view, cx, "require('mmv').notify('notice')");
        frame(cx, window);
        view.read_with(cx, |v, cx| {
            assert_eq!(v.app.mode, mode);
            assert_eq!(v.input.state, prompt);
            assert_eq!(v.input.state.read(cx).value().as_ref(), "保持入力");
        });
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
    }
}
