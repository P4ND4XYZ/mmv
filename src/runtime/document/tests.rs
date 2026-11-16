use super::*;
use crate::app::{
    DocumentOperation, ExternalRequestId, OperationOrigin, OperationResult, ReadResolution,
};
use std::path::{Path, PathBuf};

// Deterministically delay delivery only after the production I/O and real parser.
// This test-only gate makes reverse completion and late-result cleanup reproducible.
#[cfg(feature = "ui-tests")]
struct Gate {
    open: std::sync::atomic::AtomicBool,
    reached: std::sync::atomic::AtomicBool,
    waker: std::sync::Mutex<Option<std::task::Waker>>,
}
#[cfg(feature = "ui-tests")]
static PREPARE_GATES: std::sync::Mutex<std::collections::BTreeMap<PathBuf, Arc<Gate>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

#[cfg(feature = "ui-tests")]
pub(crate) struct PrepareGateGuard {
    path: PathBuf,
    gate: Arc<Gate>,
}
#[cfg(feature = "ui-tests")]
impl PrepareGateGuard {
    pub(crate) fn reached(&self) -> bool {
        self.gate.reached.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(crate) fn release(&self) {
        self.gate
            .open
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(waker) = self.gate.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}
#[cfg(feature = "ui-tests")]
impl Drop for PrepareGateGuard {
    fn drop(&mut self) {
        self.release();
        PREPARE_GATES.lock().unwrap().remove(&self.path);
    }
}
#[cfg(feature = "ui-tests")]
pub(crate) fn pause_after_prepare(path: PathBuf) -> PrepareGateGuard {
    let gate = Arc::new(Gate {
        open: std::sync::atomic::AtomicBool::new(false),
        reached: std::sync::atomic::AtomicBool::new(false),
        waker: std::sync::Mutex::new(None),
    });
    let mut active = PREPARE_GATES.lock().unwrap();
    assert!(
        active.insert(path.clone(), gate.clone()).is_none(),
        "unique test path"
    );
    PrepareGateGuard { path, gate }
}
#[cfg(feature = "ui-tests")]
pub(super) async fn after_prepare(request: &app::ReadRequest) {
    let gate = PREPARE_GATES
        .lock()
        .unwrap()
        .get(&request.path)
        .filter(|_| request.phase == app::ReadPhase::Load)
        .cloned();
    if let Some(gate) = gate {
        gate.reached
            .store(true, std::sync::atomic::Ordering::SeqCst);
        std::future::poll_fn(|cx| {
            let mut waker = gate.waker.lock().unwrap();
            if gate.open.load(std::sync::atomic::Ordering::SeqCst) {
                std::task::Poll::Ready(())
            } else {
                *waker = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await;
    }
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mmv-pdf-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn begin(app: &mut app::App, path: &Path, external: bool) -> app::ReadRequest {
    let OperationResult::Pending(request) = app.operate(
        DocumentOperation::ExternalOpen {
            path: path.into(),
            line: None,
        },
        OperationOrigin::External,
        0.,
        external.then(|| ExternalRequestId(path.display().to_string())),
    ) else {
        panic!("pending identity");
    };
    request
}
fn load(app: &mut app::App, resolve: app::ReadRequest) -> app::ReadRequest {
    let (app::ReadResponse::Resolved { request, key }, None) =
        test_read(resolve).into_parts().unwrap()
    else {
        panic!("identity only");
    };
    let ReadResolution::Continue(request) = app.resolve_at(request, key, 0.).unwrap() else {
        panic!("load continuation");
    };
    request
}
fn loaded(result: PreparedRead) -> (app::ReadResult, Option<pdf::PreparedPdf>) {
    let (app::ReadResponse::Loaded(result), prepared) = result.into_parts().unwrap() else {
        panic!("body result");
    };
    (result, prepared)
}

#[test]
fn actual_io_prepare_envelope_shares_bytes_and_requires_parse_before_adoption() {
    let files = Files::new();
    let path = files.write("valid.PDF", &pdf::tests::fixture("3 0 R 3 0 R", 2, false));
    let mut app = app::App::default();
    let before = pdf::prepare_calls();
    let resolve = begin(&mut app, &path, true);
    let request = load(&mut app, resolve);
    assert!(app.document().is_none());
    assert_eq!(pdf::prepare_calls(), before);
    let (result, prepared) = loaded(test_read(request));
    assert_eq!(pdf::prepare_calls(), before + 1);
    assert!(prepared.is_some());
    let app::DocumentContent::Pdf { bytes } = &result.content.as_ref().unwrap().content else {
        panic!("PDF content");
    };
    let bytes = bytes.clone();
    let weak = Arc::downgrade(&bytes);
    let completion = app.complete_at(result, 0.).unwrap();
    assert!(completion.adopted && completion.outcome.is_ok());
    let app::DocumentContent::Pdf {
        bytes: adopted_bytes,
    } = &app.document().unwrap().content
    else {
        panic!("PDF content");
    };
    assert!(Arc::ptr_eq(adopted_bytes, &bytes));
    assert_eq!(pdf::prepare_calls(), before + 1);
    drop(prepared);
    drop(bytes);
    drop(app);
    assert!(weak.upgrade().is_none());
}

#[test]
fn actual_invalid_and_locked_prepare_keep_previous_document_position_and_mode() {
    let files = Files::new();
    let markdown = files.write("old.md", b"confirmed old body");
    let mut app = app::App::default();
    let initial = app.open(markdown);
    app.complete(infra::read_document(initial)).unwrap();
    app.begin_document_search().unwrap();
    let generation = app.document().unwrap().generation;
    let position = app.position_request().cloned();
    for (name, bytes, expected) in [
        (
            "invalid.pdf",
            Arc::new(b"not PDF".to_vec()),
            "Unable to parse PDF",
        ),
        (
            "locked.pdf",
            pdf::tests::fixture("3 0 R", 1, true),
            "PDF requires a password",
        ),
    ] {
        let path = files.write(name, &bytes);
        let resolve = begin(&mut app, &path, true);
        let request = load(&mut app, resolve);
        let (result, prepared) = loaded(test_read(request));
        assert!(prepared.is_none());
        assert_eq!(
            app.complete_at(result, 12.).unwrap().outcome,
            Err(expected.into())
        );
        assert_eq!(app.document().unwrap().generation, generation);
        assert_eq!(app.position_request(), position.as_ref());
        assert_eq!(app.mode, app::Mode::Search);
    }
}

#[test]
fn stale_external_pdf_has_real_outcome_once_and_old_resolve_cannot_steal_load() {
    let files = Files::new();
    let a = files.write("a.pdf", &pdf::tests::fixture("3 0 R", 1, false));
    let b = files.write("b.pdf", &pdf::tests::fixture("3 0 R", 1, false));
    let mut app = app::App::default();
    let old_resolve = begin(&mut app, &a, true);
    let old = load(&mut app, old_resolve.clone());
    let newer_resolve = begin(&mut app, &b, true);
    let newer = load(&mut app, newer_resolve);
    assert!(
        app.resolve_at(old_resolve, Ok(app::DocumentKey(a.clone())), 0.)
            .is_none()
    );
    assert_eq!(app.read_request_status(&old), Some(false));
    let (newer_result, newer_prepared) = loaded(test_read(newer));
    assert!(app.complete_at(newer_result, 0.).unwrap().adopted);
    let (old_result, old_prepared) = loaded(test_read(old.clone()));
    let duplicate = app::ReadResult {
        request: old.clone(),
        content: old_result
            .content
            .as_ref()
            .map(|doc| app::LoadedDocument {
                key: doc.key.clone(),
                content: doc.content.clone(),
            })
            .map_err(Clone::clone),
    };
    let completion = app.complete_at(old_result, 0.).unwrap();
    assert!(!completion.adopted && !completion.current);
    assert_eq!(completion.response_to, old.response_to);
    assert_eq!(completion.outcome, Ok(()));
    assert!(app.complete_at(duplicate, 0.).is_none());
    assert_eq!(app.document().unwrap().path, b);
    drop((old_prepared, newer_prepared));
}

#[test]
fn internal_cancel_removes_both_app_records_without_completion_or_state_change() {
    let files = Files::new();
    let path = files.write("cancel.pdf", &pdf::tests::fixture("3 0 R", 1, false));
    let mut app = app::App::default();
    let resolve = begin(&mut app, &path, false);
    let old = load(&mut app, resolve);
    let token = pdf::PrepareCancellation::default();
    token.cancel();
    let count = pdf::prepare_calls();
    let result = read_and_prepare(old.clone(), &token);
    assert!(result.is_abandoned());
    assert_eq!(pdf::prepare_calls(), count);
    assert!(!app.finish_internal(&old)); // current must never be cancelled this way
    let current = begin(&mut app, &path, false);
    let position = app.position_request().cloned();
    assert!(app.finish_internal(&old));
    assert!(!app.finish_internal(&old));
    assert_eq!(app.read_request_status(&old), None);
    assert_eq!(app.read_request_status(&current), Some(true));
    assert_eq!(app.position_request(), position.as_ref());
    assert!(app.document().is_none() && app.error().is_none());
    assert_eq!(app.read_record_counts(), (1, 1));
}

#[test]
fn registry_full_identity_phase_and_duplicate_guards_preserve_current_load() {
    let mut app = app::App::default();
    let resolve = begin(&mut app, Path::new("guard.pdf"), true);
    let mut registry = registry::ReadRegistry::default();
    registry.begin(resolve.clone()).unwrap();
    assert!(registry.begin(resolve.clone()).is_none());
    assert!(registry.ready(&resolve));
    let ReadResolution::Continue(load) = app
        .resolve_at(
            resolve.clone(),
            Ok(app::DocumentKey("guard.pdf".into())),
            0.,
        )
        .unwrap()
    else {
        panic!();
    };
    let mut altered = load.clone();
    altered.path = "different.pdf".into();
    assert!(registry.begin(altered).is_none());
    registry.begin(load.clone()).unwrap();
    assert!(!registry.ready(&resolve));
    registry.retire(&resolve);
    assert_eq!(registry.pending_count(), 1);
    assert!(registry.ready(&load));
    assert!(!registry.ready(&load));
    registry.retire(&load);
    assert_eq!(registry.pending_count(), 0);
    registry.stop();
    assert!(registry.begin(load).is_none());
}

#[test]
fn stopped_registry_retires_ready_results_but_collects_in_progress_jobs() {
    let mut app = app::App::default();
    let ready = begin(&mut app, Path::new("ready.pdf"), true);
    let processing = begin(&mut app, Path::new("processing.pdf"), true);
    let mut registry = registry::ReadRegistry::default();
    registry.begin(ready.clone()).unwrap();
    registry.ready(&ready);
    let token = registry.begin(processing.clone()).unwrap();
    registry.stop();
    assert!(token.is_cancelled());
    assert_eq!(registry.pending_count(), 1);
    assert!(!registry.ready(&processing));
    registry.retire(&processing);
    assert_eq!(registry.pending_count(), 0);
}

#[test]
fn shutdown_drains_request_records_without_replacing_document_or_input() {
    let mut app = app::App::default();
    let initial = app.open("old.md".into());
    app.complete(app::ReadResult {
        request: initial,
        content: Ok(app::LoadedDocument::markdown(
            app::DocumentKey("old.md".into()),
            "old".into(),
        )),
    });
    app.begin_document_search().unwrap();
    app.preview_search("draft".into());
    let generation = app.document().unwrap().generation;
    let position = app.position_request().cloned();
    let pending = begin(&mut app, Path::new("pending.pdf"), true);
    app.shutdown_reads();
    assert_eq!(app.read_request_status(&pending), None);
    assert!(!app.is_loading());
    assert_eq!(app.document().unwrap().generation, generation);
    assert_eq!(app.position_request(), position.as_ref());
    assert_eq!(app.mode, app::Mode::Search);
    assert_eq!(app.read_record_counts(), (0, 0));
    let result = app.operate(
        DocumentOperation::ExternalOpen {
            path: "later.pdf".into(),
            line: None,
        },
        OperationOrigin::External,
        0.,
        Some(ExternalRequestId("closed".into())),
    );
    assert!(matches!(
        result,
        OperationResult::Complete {
            outcome: Err(app::OperationError::DocumentUnavailable),
            ..
        }
    ));
    assert_eq!(app.read_record_counts(), (0, 0));
}

#[test]
fn raw_pdf_compatibility_response_cannot_fabricate_prepared_success() {
    let mut app = app::App::default();
    let request = begin(&mut app, Path::new("raw.pdf"), true);
    let result = PreparedRead::from(app::ReadResult {
        request,
        content: Ok(app::LoadedDocument {
            key: app::DocumentKey("raw.pdf".into()),
            content: app::DocumentContent::Pdf {
                bytes: Arc::new(vec![1, 2, 3]),
            },
        }),
    });
    let (result, prepared) = loaded(result);
    assert!(prepared.is_none());
    assert_eq!(
        result.content.err().unwrap(),
        "PDF success requires actual preparation"
    );
}
