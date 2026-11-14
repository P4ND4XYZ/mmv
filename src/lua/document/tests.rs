use super::*;
use crate::lua::LuaRuntime;

fn extract(path: &Path, _: &str) -> Result<Vec<LinkCandidate>, String> {
    Ok(crate::core::links::candidates(
        path,
        [
            ("日本語".into(), "#one".into(), Some(0..3)),
            ("別名".into(), "#one".into(), Some(3..6)),
        ],
    ))
}
fn runtime() -> LuaRuntime {
    LuaRuntime::from_config(Default::default())
}
fn publish(runtime: &LuaRuntime) {
    runtime
        .context()
        .replace_document(
            runtime.lua(),
            DocumentHandle::new(
                PathBuf::from(if cfg!(windows) {
                    r"C:\docs\a.md"
                } else {
                    "/docs/a.md"
                }),
                Arc::new("日本語".into()),
                extract,
            ),
        )
        .unwrap();
}

#[test]
fn snapshot_identity_readonly_copies_state_and_source_validation() {
    let runtime = runtime();
    publish(&runtime);
    runtime
        .lua()
        .load(
            r#"
        local m=require('mmv'); old=m.document.current()
        assert(old==m.document.current() and old:source()==old:source())
        a=old:links(); b=old:links()
        assert(a~=b and a[1]~=b[1] and a[1].target==b[2].target)
        assert(a[1].source_range==b[1].source_range)
        assert(a[1].source_range.start==0 and a[1].source_range.stop==3)
        assert(not pcall(function() a[1].source_range.start=7 end))
        assert(not pcall(function() a[1].target.anything=true end))
        a[1].display_name='changed'; a[1].source_range=nil
        assert(old:links()[1].display_name=='日本語')
        s=m.ui.state({doc=old,source=old:source(),target=b[1].target,range=b[1].source_range})
        local v=s:get(); assert(rawequal(v.target,s:get().target))
    "#,
        )
        .exec()
        .unwrap();
    let source: AnyUserData = runtime.lua().load("return old:source()").eval().unwrap();
    let range: AnyUserData = runtime
        .lua()
        .load("return b[1].source_range")
        .eval()
        .unwrap();
    assert_eq!(
        source.borrow::<SourceHandle>().unwrap().text().as_str(),
        "日本語"
    );
    assert_eq!(
        range
            .borrow::<RangeHandle>()
            .unwrap()
            .for_source(&source.borrow::<SourceHandle>().unwrap())
            .unwrap(),
        0..3
    );
    publish(&runtime);
    let new: AnyUserData = runtime
        .lua()
        .load("return require('mmv').document.current():source()")
        .eval()
        .unwrap();
    assert!(
        range
            .borrow::<RangeHandle>()
            .unwrap()
            .for_source(&new.borrow::<SourceHandle>().unwrap())
            .is_err()
    );
    runtime.lua().load("assert(old~=require('mmv').document.current()); assert(old:links()[1].display_name=='日本語'); assert(b[1].target~=require('mmv').document.current():links()[1].target); collectgarbage('collect'); assert(s:get().doc==old)").exec().unwrap();
}

#[test]
fn old_snapshot_bytes_are_kept_by_state_handles_and_released_after_gc() {
    let runtime = runtime();
    publish(&runtime);
    let source = Arc::downgrade(
        &runtime
            .context()
            .documents
            .borrow()
            .current
            .as_ref()
            .unwrap()
            .0
            .source,
    );
    runtime
        .lua()
        .load("local m=require('mmv'); saved=m.ui.state(m.document.current():links())")
        .exec()
        .unwrap();
    publish(&runtime);
    runtime.lua().gc_collect().unwrap();
    assert!(source.upgrade().is_some());
    runtime.lua().load("saved=nil").exec().unwrap();
    for _ in 0..3 {
        runtime.lua().gc_collect().unwrap();
        runtime.pump_one();
    }
    assert!(source.upgrade().is_none());
}

#[test]
fn replacement_subscribers_are_captured_ordered_cancelled_and_owner_scoped() {
    let runtime = runtime();
    runtime.lua().load(r#"
        local m=require('mmv'); local u=m.ui; events=''
        w=u.float.open{width=100,height=100,render=function() return u.text{text='x'} end}
        first=m.document.on_replaced({owner=w},function() events=events..'1'; second:cancel(); late=m.document.on_replaced({owner=w},function() events=events..'L' end) end)
        second=m.document.on_replaced({owner=w},function() events=events..'2' end)
    "#).exec().unwrap();
    publish(&runtime);
    for _ in 0..5 {
        runtime.pump_one();
    }
    runtime
        .lua()
        .load("assert(events=='1'); assert(first:cancel()); assert(not first:cancel())")
        .exec()
        .unwrap();
    publish(&runtime);
    runtime.lua().load("w:close(); assert(require('mmv').document.on_replaced({owner=w},function()end)==false)").exec().unwrap();
    for _ in 0..5 {
        runtime.pump_one();
    }
    runtime.lua().load("assert(events=='1')").exec().unwrap();
    assert!(runtime.context().shared.borrow().replaced.is_empty());
}

#[test]
fn link_analysis_failure_is_not_an_empty_success() {
    let runtime = runtime();
    runtime
        .context()
        .replace_document(
            runtime.lua(),
            DocumentHandle::new(PathBuf::new(), Arc::new(String::new()), |_, _| {
                Err("Unable to analyze document links".into())
            }),
        )
        .unwrap();
    runtime.lua().load("local d=require('mmv').document.current(); assert(not pcall(function() d:links() end))").exec().unwrap();
}
