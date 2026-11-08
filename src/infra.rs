mod document;
mod mermaid;
mod watch;

#[cfg(windows)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Deadline API is reserved for the next product CLI wiring step"
    )
)]
mod deadline;
#[cfg(windows)]
#[expect(
    dead_code,
    reason = "Windows instance API is not wired into main until the next session"
)]
mod instance;
#[cfg(windows)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Windows transport API is not wired into GUI delivery until the next session"
    )
)]
mod ipc;
#[cfg(windows)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Production namespace is reserved for the next product startup wiring step"
    )
)]
mod security;

#[cfg(windows)]
#[expect(
    unused_imports,
    reason = "Public Windows foundation exports for the next CLI/GUI wiring step"
)]
pub use {
    deadline::Deadline,
    instance::{INTERNAL_GUI_ARGUMENT, InstanceGuard, connect_or_start, gui_is_alive, spawn_gui},
    ipc::{Incoming, PipeClient, PipeServer, Reply, Request, Response},
    security::Namespace,
};

pub use document::read_document;
pub use mermaid::generate_diagram;
pub use watch::{FileWatch, WatchNotice};
