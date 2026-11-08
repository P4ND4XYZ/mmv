mod config;
mod document;
mod logging;
mod mermaid;
mod watch;

#[cfg(windows)]
mod deadline;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod security;

#[cfg(all(windows, test, feature = "ui-tests"))]
pub use ipc::PipeClient;

#[cfg(all(windows, test, feature = "ui-tests"))]
pub(crate) mod test_support {
    use super::{Deadline, Namespace};

    pub fn namespace() -> Namespace {
        Namespace::isolated()
    }

    pub fn deadline(duration: std::time::Duration) -> Deadline {
        Deadline::after(duration)
    }
}
#[cfg(windows)]
pub use {
    deadline::Deadline,
    instance::{INTERNAL_GUI_ARGUMENT, InstanceGuard, connect_or_start},
    ipc::{PipeServer, Reply, Request, Response},
    security::Namespace,
};

pub use config::load_config;
pub use document::read_document;
pub use logging::{log_config_failure, log_diagram_failure};
pub use mermaid::{cleanup_mermaid_scratch, generate_diagram};
pub use watch::{FileWatch, WatchNotice};
