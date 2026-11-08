use std::{collections::BTreeMap, path::PathBuf};

use super::{Document, Generation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagramRequest {
    pub generation: Generation,
    pub path: PathBuf,
    /// Full-document UTF-8 byte offset of the fenced code block.
    pub start: usize,
    pub source: String,
}

pub struct DiagramResult {
    pub request: DiagramRequest,
    pub png: Result<Vec<u8>, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramStatus {
    Pending,
    Running,
    Ready,
    Failed,
}

pub struct Diagram {
    pub source: String,
    pub status: DiagramStatus,
}

#[derive(Default)]
pub(super) struct MermaidState {
    pub diagrams: BTreeMap<usize, Diagram>,
    // Kept across document replacement to serialize external processes.
    diagram_in_flight: Option<DiagramRequest>,
}

impl MermaidState {
    pub fn replace_document(&mut self) {
        self.diagrams.clear();
    }

    pub fn discover_diagrams(&mut self, sources: BTreeMap<usize, String>) {
        for (start, source) in sources {
            self.diagrams.entry(start).or_insert(Diagram {
                source,
                status: DiagramStatus::Pending,
            });
        }
    }

    pub fn next_diagram(
        &mut self,
        document: Option<&Document>,
        loading: bool,
    ) -> Option<DiagramRequest> {
        if self.diagram_in_flight.is_some() || loading {
            return None;
        }
        let document = document?;
        let (&start, diagram) = self
            .diagrams
            .iter_mut()
            .find(|(_, diagram)| diagram.status == DiagramStatus::Pending)?;
        diagram.status = DiagramStatus::Running;
        let request = DiagramRequest {
            generation: document.generation,
            path: document.path.clone(),
            start,
            source: diagram.source.clone(),
        };
        self.diagram_in_flight = Some(request.clone());
        Some(request)
    }

    /// Releases the serial worker even for stale completions; only current results are accepted.
    pub fn complete_diagram(
        &mut self,
        result: &DiagramResult,
        document: Option<&Document>,
    ) -> bool {
        if self.diagram_in_flight.as_ref() != Some(&result.request) {
            return false;
        }
        self.diagram_in_flight = None;
        let Some(document) = document else {
            return false;
        };
        if document.generation != result.request.generation || document.path != result.request.path
        {
            return false;
        }
        let Some(diagram) = self.diagrams.get_mut(&result.request.start) else {
            return false;
        };
        if diagram.source != result.request.source {
            return false;
        }
        diagram.status = if result.png.is_ok() {
            DiagramStatus::Ready
        } else {
            DiagramStatus::Failed
        };
        true
    }
}
