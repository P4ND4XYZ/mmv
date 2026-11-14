//! Lua host adapter: existing app operations and actual position acknowledgements.
use super::Viewer;
use crate::{
    app::{
        DocumentOperation, OperationOrigin, OperationResult, PositionRequest, ReadCompletion,
        ReadRequestId, ReadResponse,
    },
    lua::{
        document::DocumentHandle,
        navigation::{Delivery, OpenRequest, Outcome},
        notify::{Level, Notice},
    },
};
use gpui_kit::Context;

#[derive(Default)]
pub(super) struct NavigationState {
    pending: Option<Navigation>,
    suppressed_positions: std::collections::BTreeSet<u64>,
}
struct Navigation {
    request: OpenRequest,
    order: u64,
    stage: Stage,
    failure: Option<crate::infra::ReadFailureKind>,
}
enum Stage {
    Waiting,
    Reading(ReadRequestId),
    Position(PositionRequest),
}

impl Viewer {
    pub(super) fn show_lua_notice(&mut self, notice: Notice) {
        crate::infra::log_document_failure(
            std::path::Path::new("[lua notification]"),
            &notice.message,
        );
        let group = match notice.level {
            Level::Info => crate::config::Group::CommandLine,
            Level::Warning => crate::config::Group::Warning,
            Level::Error => crate::config::Group::Error,
        };
        self.message_level = Some((notice.message.clone(), group));
        self.message = Some(notice.message);
        self.message_is_error = notice.level == Level::Error;
    }

    fn deliver_navigation(&mut self, delivery: Delivery, outcome: Outcome) {
        if let Some(detail) = &outcome.detail {
            crate::infra::log_document_failure(
                std::path::Path::new("[lua navigation]"),
                &format!("{}: {}", detail.code, detail.message),
            );
            if !delivery.handler && detail.code != "superseded" {
                self.show_lua_notice(Notice {
                    message: detail.message.clone(),
                    level: if outcome.ok {
                        Level::Warning
                    } else {
                        Level::Error
                    },
                });
            }
        }
        if let Some(callback) = delivery.callback {
            match outcome.value(self.lua.lua()).and_then(|result| {
                self.lua
                    .context()
                    .enqueue(self.lua.lua(), callback, (result,))
            }) {
                Ok(_) => (),
                Err(error) => self.lua.context().report_error(
                    None,
                    crate::lua::ErrorInfo::new(error, crate::lua::Phase::Event, None),
                ),
            }
        }
    }

    fn finish_lua_navigation(&mut self, outcome: Outcome) {
        if let Some(pending) = self.lua_navigation.pending.take() {
            self.deliver_navigation(pending.request.delivery, outcome);
        }
    }

    pub(super) fn supersede_lua_navigation(&mut self, order: u64) {
        let current = self.app.position_request().map(|position| position.serial);
        self.lua_navigation
            .suppressed_positions
            .retain(|serial| Some(*serial) == current);
        if self
            .lua_navigation
            .pending
            .as_ref()
            .is_some_and(|pending| pending.order < order)
        {
            if let Some(Navigation {
                stage: Stage::Position(position),
                ..
            }) = &self.lua_navigation.pending
            {
                // Consume only this navigation's still-pending position before
                // the successor can settle it. Full generation/serial equality
                // preserves a newer position and never rolls back applied work.
                self.app.acknowledge_position(position);
                self.lua_navigation
                    .suppressed_positions
                    .insert(position.serial);
            }
            self.finish_lua_navigation(Outcome::failure("superseded", "Navigation was superseded"));
        }
    }

    pub(super) fn open_lua_navigation(&mut self, request: OpenRequest, cx: &mut Context<Self>) {
        if request.stale {
            self.deliver_navigation(
                request.delivery,
                Outcome::failure(
                    "stale_target",
                    "Navigation target belongs to a replaced document",
                ),
            );
            return;
        }
        // Browser launch does not invalidate the app's independent local read
        // or position intent, matching the existing Web-link operation.
        if let crate::app::LinkTarget::Web(url) = &request.target.target {
            let outcome = match (self.browser_launcher)(url) {
                Ok(()) => Outcome::success(),
                Err(error) => {
                    crate::infra::log_document_failure(
                        std::path::Path::new("[browser]"),
                        &error.to_string(),
                    );
                    Outcome::failure("browser_open_failed", "Unable to open URL in browser")
                }
            };
            self.deliver_navigation(request.delivery, outcome);
            return;
        }
        self.next_operation_order += 1;
        let order = self.next_operation_order;
        self.supersede_lua_navigation(order);
        self.lua_navigation.pending = Some(Navigation {
            request,
            order,
            stage: Stage::Waiting,
            failure: None,
        });
        self.advance_lua_navigation(cx);
        cx.notify();
    }

    /// Preserve the common link history departure barrier, without requiring a
    /// GPUI window/focus or cancelling any Search/Command input from Lua.
    pub(super) fn advance_lua_navigation(&mut self, cx: &mut Context<Self>) {
        if !self
            .lua_navigation
            .pending
            .as_ref()
            .is_some_and(|p| matches!(p.stage, Stage::Waiting))
        {
            return;
        }
        self.settle_position(cx);
        if self.document.normalization_pending() || self.app.position_request().is_some() {
            return;
        }
        let mut pending = self.lua_navigation.pending.take().unwrap();
        let operation = match &pending.request.target.target {
            crate::app::LinkTarget::File { path, fragment } => match fragment {
                Some(id) => DocumentOperation::Heading {
                    path: if self.app.document().is_some_and(|doc| doc.path == *path) {
                        None
                    } else {
                        Some(path.clone())
                    },
                    id: id.clone(),
                },
                None => DocumentOperation::Open(path.clone()),
            },
            _ => unreachable!("targets are resolved at snapshot extraction"),
        };
        let result = self.request_operation_in_order(
            operation,
            OperationOrigin::Link,
            None,
            pending.order,
            cx,
        );
        match result {
            OperationResult::Pending(read) => {
                pending.stage = Stage::Reading(read.id);
                self.lua_navigation.pending = Some(pending);
            }
            OperationResult::Complete {
                outcome: Ok(()), ..
            } => {
                if let Some(position) = self.app.position_request().cloned() {
                    pending.stage = Stage::Position(position);
                    self.lua_navigation.pending = Some(pending);
                } else {
                    self.deliver_navigation(pending.request.delivery, Outcome::success());
                }
            }
            OperationResult::Complete {
                outcome: Err(_), ..
            } => self.deliver_navigation(
                pending.request.delivery,
                Outcome::failure("navigation_failed", "Unable to navigate to document"),
            ),
        }
    }

    /// Production runtime supplies the I/O classification independently of the
    /// English GUI summary. Delayed normalization retains it with the request.
    pub(crate) fn complete_classified(
        &mut self,
        response: ReadResponse,
        failure: Option<crate::infra::ReadFailureKind>,
        cx: &mut Context<Self>,
    ) {
        let id = match &response {
            ReadResponse::Loaded(result) => result.request.id,
            ReadResponse::Resolved { request, .. } => request.id,
        };
        if let Some(pending) = self.lua_navigation.pending.as_mut()
            && matches!(pending.stage, Stage::Reading(read) if read == id)
        {
            pending.failure = failure;
        }
        self.complete(response, cx);
    }

    pub(super) fn lua_read_completed(&mut self, completion: &ReadCompletion) {
        let Some(pending) = self.lua_navigation.pending.as_ref().filter(
            |pending| matches!(pending.stage, Stage::Reading(id) if id == completion.request_id),
        ) else {
            return;
        };
        if !completion.current {
            self.finish_lua_navigation(Outcome::failure("superseded", "Navigation was superseded"));
        } else if let Err(message) = &completion.outcome {
            let code = pending
                .failure
                .map_or("navigation_failed", |failure| failure.code());
            self.app.clear_error(); // The Lua result owns the only GUI fallback.
            self.finish_lua_navigation(Outcome::failure(code, message.clone()));
        } else if let Some(position) = self.app.position_request().cloned() {
            self.lua_navigation.pending.as_mut().unwrap().stage = Stage::Position(position);
        } else {
            // Identity-only same-document success is not superseded.
            self.finish_lua_navigation(Outcome::success());
        }
    }

    pub(super) fn lua_position_owned(&self, request: &PositionRequest) -> bool {
        self.lua_navigation.pending.as_ref().is_some_and(
            |pending| matches!(&pending.stage, Stage::Position(position) if position == request),
        )
    }

    pub(super) fn lua_position_managed(&self, request: &PositionRequest) -> bool {
        self.lua_position_owned(request)
            || self
                .lua_navigation
                .suppressed_positions
                .contains(&request.serial)
    }

    pub(super) fn lua_position_completed(
        &mut self,
        request: &PositionRequest,
        warning: Option<(&'static str, String)>,
    ) {
        self.lua_navigation
            .suppressed_positions
            .remove(&request.serial);
        if self.lua_position_owned(request) {
            let outcome = warning.map_or_else(Outcome::success, |(code, message)| {
                Outcome::warning(code, message)
            });
            self.finish_lua_navigation(outcome);
        }
    }

    /// Terminal failure at the position-analysis/application boundary. The
    /// adopted body/history are intentionally retained; stale failures do nothing.
    pub(super) fn lua_position_unavailable(
        &mut self,
        request: &PositionRequest,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.app.position_request() != Some(request) || !self.lua_position_managed(request) {
            return false;
        }
        self.app.acknowledge_position(request);
        self.lua_position_completed(request, Some(("position_unavailable", message)));
        cx.notify();
        true
    }

    pub(super) fn publish_lua_document(&mut self, cx: &mut Context<Self>) {
        let document = self.app.document().expect("adopted document");
        let snapshot = DocumentHandle::new(
            document.path.clone(),
            document.source.clone(),
            super::links::extract_candidates,
        );
        if let Err(error) = self
            .lua
            .context()
            .replace_document(self.lua.lua(), snapshot)
        {
            self.lua.context().report_error(
                None,
                crate::lua::ErrorInfo::new(error, crate::lua::Phase::Event, None),
            );
        }
        // A reload may carry a heading but produces a new PositionRequest.
        // Queue its adoption subscribers before the displaced result, too.
        if self.lua_navigation.pending.as_ref().is_some_and(|pending| {
            matches!(&pending.stage, Stage::Position(position) if self.app.position_request() != Some(position))
        }) {
            self.finish_lua_navigation(Outcome::failure("superseded", "Navigation was superseded"));
        }
        // Adoption subscribers run before any position/layout completion.
        self.pump_lua(cx);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
