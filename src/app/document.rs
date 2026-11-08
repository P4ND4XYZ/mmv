use std::{collections::BTreeMap, path::PathBuf};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRequest {
    pub id: ReadRequestId,
    pub path: PathBuf,
    pub response_to: Option<ExternalRequestId>,
    pub origin: OperationOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOrigin {
    Initial,
    Keyboard,
    Link,
    External,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "File watching is connected in stage 3.")
    )]
    Automatic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentOperation {
    Initial(PathBuf),
    Open(PathBuf),
    Reload,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Heading links are connected in stage 4; app transitions are tested now."
        )
    )]
    Heading {
        path: Option<PathBuf>,
        id: String,
    },
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "History keys and external commands are connected in stages 4/5."
        )
    )]
    Back,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "History keys and external commands are connected in stages 4/5."
        )
    )]
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

#[derive(Clone, Debug, PartialEq)]
pub struct PositionRequest {
    pub generation: Generation,
    pub target: PositionTarget,
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
}

pub struct Document {
    pub generation: Generation,
    pub path: PathBuf,
    pub key: DocumentKey,
    pub source: String,
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
    ) -> ReadRequest {
        self.next_request += 1;
        let request = ReadRequest {
            id: ReadRequestId(self.next_request),
            path,
            response_to,
            origin,
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
        };
        if current {
            self.pending = None;
            match result.content {
                Ok(loaded) => {
                    self.next_generation += 1;
                    self.current = Some(Document {
                        generation: Generation(self.next_generation),
                        path: request.path,
                        key: loaded.key,
                        source: loaded.source,
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
