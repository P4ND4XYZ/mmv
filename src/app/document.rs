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
    pub phase: ReadPhase,
    pub skip_same: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadPhase {
    Resolve,
    Load,
}

/// Tree-only preflight and identity check, performed by infra before any read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeOpen {
    pub root: PathBuf,
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
    ExternalOpen {
        path: PathBuf,
        line: Option<std::num::NonZeroU64>,
    },
    TreeOpen {
        path: PathBuf,
        root: PathBuf,
    },
    Reload,
    Heading {
        path: Option<PathBuf>,
        id: String,
    },
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
    SourceLine(std::num::NonZeroU64),
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

/// Identity-only results never fabricate an empty source document.
pub enum ReadResponse {
    Resolved {
        request: ReadRequest,
        key: Result<DocumentKey, String>,
    },
    Loaded(ReadResult),
}

impl From<ReadResult> for ReadResponse {
    fn from(result: ReadResult) -> Self {
        Self::Loaded(result)
    }
}

pub enum ReadResolution {
    Continue(ReadRequest),
    Complete(ReadCompletion),
}

/// Completion of one outstanding read, independently of display adoption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCompletion {
    pub request_id: ReadRequestId,
    pub response_to: Option<ExternalRequestId>,
    pub outcome: Result<(), String>,
    pub adopted: bool,
    pub current: bool,
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
    outstanding: BTreeMap<ReadRequestId, (ReadRequest, Option<u64>)>,
    pending: Option<ReadRequestId>,
    reload: Option<ReadRequestId>,
    period: u64,
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
        skip_same: bool,
        reload: bool,
    ) -> ReadRequest {
        self.next_request += 1;
        let request = ReadRequest {
            id: ReadRequestId(self.next_request),
            path,
            response_to,
            origin,
            tree,
            phase: if skip_same {
                ReadPhase::Resolve
            } else {
                ReadPhase::Load
            },
            skip_same,
        };
        self.outstanding
            .insert(request.id, (request.clone(), reload.then_some(self.period)));
        if reload {
            self.reload = Some(request.id);
        } else {
            self.pending = Some(request.id);
        }
        self.error = None;
        request
    }

    /// Unknown, altered, and already completed requests produce no second completion.
    pub fn complete(&mut self, result: ReadResult) -> Option<ReadCompletion> {
        let (request, _) = self.outstanding.get(&result.request.id)?;
        if request != &result.request {
            return None;
        }
        let (request, period) = self.outstanding.remove(&result.request.id)?;
        let current = self.pending == Some(request.id)
            || (self.reload == Some(request.id) && period == Some(self.period));
        let mut completion = ReadCompletion {
            request_id: request.id,
            response_to: request.response_to,
            outcome: result.content.as_ref().map(|_| ()).map_err(Clone::clone),
            adopted: false,
            current,
            tree_open_succeeded: current && request.tree.is_some() && result.content.is_ok(),
        };
        if current {
            if self.pending == Some(request.id) {
                self.pending = None;
            }
            if self.reload == Some(request.id) {
                self.reload = None;
            }
            match result.content {
                Ok(loaded)
                    if request.skip_same
                        && self
                            .current
                            .as_ref()
                            .is_some_and(|doc| doc.key == loaded.key) => {}
                Ok(loaded) => {
                    // A newer actual body adoption supersedes older reloads.
                    // Identity-only same-file opens above leave them intact.
                    if self.reload.is_some_and(|reload| reload < request.id) {
                        self.reload = None;
                    }
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|doc| doc.key != loaded.key)
                    {
                        self.period += 1;
                        self.reload = None;
                    }
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

    pub fn resolve(
        &mut self,
        request: ReadRequest,
        key: Result<DocumentKey, String>,
    ) -> Option<ReadResolution> {
        let (saved, _) = self.outstanding.get(&request.id)?;
        if saved != &request || request.phase != ReadPhase::Resolve {
            return None;
        }
        match key {
            Ok(key) if self.current.as_ref().is_none_or(|doc| doc.key != key) => {
                let mut next = request;
                next.phase = ReadPhase::Load;
                self.outstanding.get_mut(&next.id)?.0 = next.clone();
                Some(ReadResolution::Continue(next))
            }
            result => {
                let (request, _) = self.outstanding.remove(&request.id)?;
                let current = self.pending == Some(request.id);
                if current {
                    self.pending = None;
                    if let Err(error) = &result {
                        self.error = Some(error.clone());
                    }
                }
                let tree_open_succeeded = current && request.tree.is_some() && result.is_ok();
                Some(ReadResolution::Complete(ReadCompletion {
                    request_id: request.id,
                    response_to: request.response_to,
                    outcome: result.map(|_| ()),
                    adopted: false,
                    current,
                    tree_open_succeeded,
                }))
            }
        }
    }

    /// A newer synchronous navigation supersedes display adoption, not read responses.
    pub fn supersede(&mut self) {
        self.pending = None;
        self.error = None;
    }

    pub fn is_loading(&self) -> bool {
        self.reload.is_some()
            || self.pending.is_some_and(|id| {
                self.outstanding
                    .get(&id)
                    .is_some_and(|(request, _)| request.phase == ReadPhase::Load)
            })
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }
}
