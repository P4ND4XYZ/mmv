use crate::{app, pdf, ui};
use std::collections::BTreeMap;

struct ReadJob {
    request: app::ReadRequest,
    cancel: pdf::PrepareCancellation,
    collector: Option<gpui_kit::Task<()>>,
    ready: bool,
}

#[derive(Default)]
pub(crate) struct ReadRegistry {
    jobs: BTreeMap<app::ReadRequestId, ReadJob>,
    stopped: bool,
}

impl ReadRegistry {
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped
    }

    pub(super) fn begin(&mut self, request: app::ReadRequest) -> Option<pdf::PrepareCancellation> {
        if self.stopped {
            return None;
        }
        if let Some(old) = self.jobs.get(&request.id) {
            // Only a completed Resolve delivery may advance this ID to Load.
            if !old.ready
                || old.request.phase != app::ReadPhase::Resolve
                || request.phase != app::ReadPhase::Load
                || {
                    let mut expected = old.request.clone();
                    expected.phase = app::ReadPhase::Load;
                    expected != request
                }
            {
                return None;
            }
        }
        if let Some(mut previous) = self.jobs.remove(&request.id)
            && let Some(job) = previous.collector.take()
        {
            job.detach();
        }
        let cancel = pdf::PrepareCancellation::default();
        self.jobs.insert(
            request.id,
            ReadJob {
                request,
                cancel: cancel.clone(),
                collector: None,
                ready: false,
            },
        );
        Some(cancel)
    }

    pub(super) fn set_job(&mut self, request: &app::ReadRequest, task: gpui_kit::Task<()>) {
        if let Some(job) = self
            .jobs
            .get_mut(&request.id)
            .filter(|job| job.request == *request)
        {
            job.collector = Some(task);
        } else {
            task.detach();
        }
    }

    pub(super) fn ready(&mut self, request: &app::ReadRequest) -> bool {
        let Some(job) = self
            .jobs
            .get_mut(&request.id)
            .filter(|job| job.request == *request && !job.ready)
        else {
            return false;
        };
        job.ready = true;
        !self.stopped
    }

    pub(super) fn retire(&mut self, request: &app::ReadRequest) {
        if self
            .jobs
            .get(&request.id)
            .is_some_and(|job| job.request == *request)
        {
            self.completed(request.id);
        }
    }

    pub(super) fn completed(&mut self, id: app::ReadRequestId) {
        if let Some(mut job) = self.jobs.remove(&id)
            && let Some(task) = job.collector.take()
        {
            task.detach();
        }
    }

    pub(super) fn cancel_stale(
        &mut self,
        view: &mut ui::Viewer,
        cx: &mut gpui_kit::Context<ui::Viewer>,
    ) {
        let mut retired = Vec::new();
        for job in self.jobs.values_mut() {
            // External response duty survives stale adoption and client disconnect.
            if job.request.response_to.is_none()
                && view.read_request_status(&job.request) == Some(false)
            {
                job.cancel.cancel();
                if job.ready && view.finish_internal_read(&job.request, cx) {
                    retired.push(job.request.id);
                }
            }
        }
        for id in retired {
            self.completed(id);
        }
    }

    pub(super) fn stop(&mut self) {
        self.stopped = true;
        for job in self.jobs.values() {
            job.cancel.cancel();
        }
        // Ready results are owned by the foreground delivery queue, which shutdown drains.
        let ready: Vec<_> = self
            .jobs
            .iter()
            .filter_map(|(id, job)| job.ready.then_some(*id))
            .collect();
        for id in ready {
            self.completed(id);
        }
    }

    /// Transfer every collector to the application's asynchronous quit hook.
    pub(crate) fn stop_and_collect(&mut self) -> Vec<gpui_kit::Task<()>> {
        self.stop();
        self.jobs
            .values_mut()
            .filter_map(|job| job.collector.take())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.jobs.len()
    }
}
