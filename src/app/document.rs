use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation(u64);

#[derive(Clone, Debug)]
pub struct ReadRequest {
    pub generation: Generation,
    pub path: PathBuf,
}

pub struct ReadResult {
    pub request: ReadRequest,
    pub content: Result<String, String>,
}

pub struct Document {
    pub generation: Generation,
    pub path: PathBuf,
    pub source: String,
}

#[derive(Default)]
pub(super) struct DocumentState {
    next_generation: u64,
    pending: Option<ReadRequest>,
    pub current: Option<Document>,
    pub error: Option<String>,
}

impl DocumentState {
    pub fn open(&mut self, path: PathBuf) -> ReadRequest {
        self.next_generation += 1;
        let request = ReadRequest {
            generation: Generation(self.next_generation),
            path,
        };
        self.pending = Some(request.clone());
        self.error = None;
        request
    }

    /// Returns true only when the current request successfully replaces the document.
    pub fn complete(&mut self, result: ReadResult) -> bool {
        if self.pending.as_ref().is_none_or(|pending| {
            pending.generation != result.request.generation || pending.path != result.request.path
        }) {
            return false;
        }
        self.pending = None;
        match result.content {
            Ok(source) => {
                self.current = Some(Document {
                    generation: result.request.generation,
                    path: result.request.path,
                    source,
                });
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }
}
