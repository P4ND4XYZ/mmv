use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub distance: f32,
}

#[derive(Default)]
pub(super) struct History {
    pub entries: Vec<Entry>,
    pub current: Option<usize>,
}

impl History {
    pub fn target(&self, forward: bool) -> Option<(usize, Entry)> {
        let current = self.current?;
        let target = if forward {
            current.checked_add(1)?
        } else {
            current.checked_sub(1)?
        };
        Some((target, self.entries.get(target)?.clone()))
    }

    fn save_departure(&mut self, distance: f32) {
        if let Some(current) = self.current {
            self.entries[current].distance = distance;
        }
    }

    /// Called only when a navigation succeeds (including synchronous headings).
    pub fn push(&mut self, path: PathBuf, departure: f32) {
        self.save_departure(departure);
        self.entries
            .truncate(self.current.map_or(0, |index| index + 1));
        self.entries.push(Entry { path, distance: 0. });
        self.current = Some(self.entries.len() - 1);
    }

    pub fn travel(&mut self, target: usize, departure: f32) {
        self.save_departure(departure);
        self.current = Some(target);
    }
}
