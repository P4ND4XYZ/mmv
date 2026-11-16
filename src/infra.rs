pub(crate) mod browser;
mod config;
mod document;
pub mod file_tree;
mod lock;
mod logging;
mod mermaid;
mod paths;
mod watch;

mod deadline;
mod instance;
mod ipc;
#[cfg(windows)]
mod security;

#[cfg(all(test, feature = "ui-tests"))]
pub use test_support::BlockingPipeClient as PipeClient;

#[cfg(all(test, feature = "ui-tests"))]
pub(crate) mod test_support {
    use super::{Deadline, Namespace};

    // GPUI test workers are synchronous. Keep the real client's reactor alive
    // across connect/request without changing those tests' UI/domain behavior.
    pub struct BlockingPipeClient {
        runtime: tokio::runtime::Runtime,
        client: super::ipc::PipeClient,
    }
    impl BlockingPipeClient {
        pub fn connect(namespace: &Namespace, deadline: Deadline) -> std::io::Result<Self> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let client = runtime.block_on(super::ipc::PipeClient::connect(namespace, deadline))?;
            Ok(Self { runtime, client })
        }
        pub fn request(
            self,
            request: super::Request,
            deadline: Deadline,
        ) -> std::io::Result<super::Response> {
            self.runtime
                .block_on(self.client.request(request, deadline))
        }
    }

    /// Only test executables can select an isolated process-probe namespace.
    pub fn process_namespace() -> Option<Namespace> {
        let directory = std::path::PathBuf::from(std::env::var_os("MMV_TEST_GUI_DIRECTORY")?);
        assert!(directory.is_absolute());
        assert!(directory.file_name()?.to_str()?.starts_with("mmv-test-"));
        let port = std::env::var("MMV_TEST_GUI_PORT")
            .ok()?
            .parse::<std::num::NonZeroU16>()
            .ok()?
            .get();
        Some(Namespace { directory, port })
    }

    pub fn namespace() -> Namespace {
        Namespace::isolated()
    }

    pub fn deadline(duration: std::time::Duration) -> Deadline {
        Deadline::after(duration)
    }
}
pub use {
    deadline::Deadline,
    instance::{INTERNAL_GUI_ARGUMENT, InstanceGuard, connect_or_start},
    ipc::{PipeServer, Reply, Request, Response},
    paths::Namespace,
};

pub use config::load_config;
#[cfg(test)]
pub use document::read_document;
pub(crate) use document::{DocumentRead, ReadFailureKind, read_document_input_classified};
pub use logging::{log_config_failure, log_diagram_failure, log_document_failure};
pub use mermaid::{cleanup_mermaid_scratch, generate_diagram};
pub use watch::{FileWatch, WatchNotice};
