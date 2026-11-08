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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Identity is delivered in stage A; operation classification uses it in stage B."
        )
    )]
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
    pub fn open(&mut self, path: PathBuf, response_to: Option<ExternalRequestId>) -> ReadRequest {
        self.next_request += 1;
        let request = ReadRequest {
            id: ReadRequestId(self.next_request),
            path,
            response_to,
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

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }
}
