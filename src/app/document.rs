use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReadRequestId(u64);

/// Supplied by the connection boundary, never derived from a document generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalRequestId(pub String);

/// Filesystem identity resolved by infra, not by app or individual input sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentKey(pub PathBuf);

/// An adopted-file subscription, stable across reloads, unique across A → B → A.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchTarget {
    pub serial: u64,
    pub key: DocumentKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRequest {
    pub id: ReadRequestId,
    pub path: PathBuf,
    pub response_to: Option<ExternalRequestId>,
    pub origin: OperationOrigin,
    pub tree: Option<TreeOpen>,
}

/// Tree-only preflight and identity check, performed by infra before any read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeOpen {
    pub root: PathBuf,
    pub current: Option<DocumentKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOrigin {
    Keyboard,
    Link,
    External,
    Automatic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentOperation {
    Initial(PathBuf),
    Open(PathBuf),
    TreeOpen { path: PathBuf, root: PathBuf },
    Reload,
    Heading { path: Option<PathBuf>, id: String },
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationError {
    DocumentUnavailable,
    NoPreviousEntry,
    NoNextEntry,
}

/// Pending reads complete through ReadCompletion; synchronous outcomes carry only
/// the caller's external ID, never a fabricated read ID or document generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationResult {
    Pending(ReadRequest),
    Complete {
        response_to: Option<ExternalRequestId>,
        outcome: Result<(), OperationError>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PositionTarget {
    Top,
    Distance(f32),
    Heading(String),
}

/// Whether a new position continues or supersedes UI movement intent that has
/// been accepted but not yet applied. The app decides this with document/history
/// semantics; UI owns the actual movement and layout waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingMovementPolicy {
    Carry,
    Replace,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionRequest {
    pub generation: Generation,
    pub target: PositionTarget,
    pub pending_movements: PendingMovementPolicy,
    // Distinguishes successive synchronous movements within the same generation.
    pub serial: u64,
}

pub struct LoadedDocument {
    pub key: DocumentKey,
    pub source: String,
}

pub struct ReadResult {
    pub request: ReadRequest,
    pub content: Result<LoadedDocument, String>,
}

/// Completion of one outstanding read, independently of display adoption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCompletion {
    pub request_id: ReadRequestId,
    pub response_to: Option<ExternalRequestId>,
    pub outcome: Result<(), String>,
    pub adopted: bool,
    /// A current tree request succeeded, including an unchanged document.
    pub tree_open_succeeded: bool,
}

pub struct Document {
    pub generation: Generation,
    pub path: PathBuf,
    pub key: DocumentKey,
    pub source: Arc<String>,
}

#[derive(Default)]
pub(super) struct DocumentState {
    next_request: u64,
    next_generation: u64,
    outstanding: BTreeMap<ReadRequestId, ReadRequest>,
    pending: Option<ReadRequestId>,
    pub current: Option<Document>,
    pub error: Option<String>,
}

impl DocumentState {
    pub fn open(
        &mut self,
        path: PathBuf,
        response_to: Option<ExternalRequestId>,
        origin: OperationOrigin,
        tree: Option<TreeOpen>,
    ) -> ReadRequest {
        self.next_request += 1;
        let request = ReadRequest {
            id: ReadRequestId(self.next_request),
            path,
            response_to,
            origin,
            tree,
        };
        self.outstanding.insert(request.id, request.clone());
        self.pending = Some(request.id);
        self.error = None;
        request
    }

    /// Unknown, altered, and already completed requests produce no second completion.
    pub fn complete(&mut self, result: ReadResult) -> Option<ReadCompletion> {
        let request = self.outstanding.get(&result.request.id)?;
        if request != &result.request {
            return None;
        }
        let request = self.outstanding.remove(&result.request.id)?;
        let current = self.pending == Some(request.id);
        let mut completion = ReadCompletion {
            request_id: request.id,
            response_to: request.response_to,
            outcome: result.content.as_ref().map(|_| ()).map_err(Clone::clone),
            adopted: false,
            tree_open_succeeded: current && request.tree.is_some() && result.content.is_ok(),
        };
        if current {
            self.pending = None;
            match result.content {
                Ok(loaded)
                    if request.tree.is_some()
                        && self
                            .current
                            .as_ref()
                            .is_some_and(|doc| doc.key == loaded.key) => {}
                Ok(loaded) => {
                    self.next_generation += 1;
                    self.current = Some(Document {
                        generation: Generation(self.next_generation),
                        path: request.path,
                        key: loaded.key,
                        source: Arc::new(loaded.source),
                    });
                    completion.adopted = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        Some(completion)
    }

    /// A newer synchronous navigation supersedes display adoption, not read responses.
    pub fn supersede(&mut self) {
        self.pending = None;
        self.error = None;
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }
}
