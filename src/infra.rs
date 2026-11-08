mod document;
mod mermaid;
mod watch;

pub use document::read_document;
pub use mermaid::generate_diagram;
pub use watch::{FileWatch, WatchNotice};
