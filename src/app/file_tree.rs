use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: OsString,
    pub directory: bool,
    pub expandable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootRequest {
    pub id: u64,
    pub document: PathBuf,
    pub markers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryRequest {
    pub id: u64,
    pub generation: u64,
    pub root: PathBuf,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Root(RootRequest),
    Directory(DirectoryRequest),
}

pub enum Completion {
    Root(RootRequest, PathBuf),
    Directory(DirectoryRequest, Result<Vec<Entry>, ()>),
}

pub struct Node {
    pub entry: Entry,
    pub expanded: bool,
    pub error: bool,
    children: Vec<PathBuf>,
}

impl Node {
    fn new(entry: Entry) -> Self {
        Self {
            entry,
            expanded: false,
            error: false,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Action {
    Next,
    Previous,
    Left,
    Right,
    Toggle,
    First,
    Last,
    Refresh,
}

#[derive(Default)]
pub struct FileTree {
    pub root: Option<PathBuf>,
    pub cursor: Option<PathBuf>,
    nodes: BTreeMap<PathBuf, Node>,
    generation: u64,
    serial: u64,
    dirty_document: Option<PathBuf>,
    root_request: Option<RootRequest>,
    pending: BTreeMap<PathBuf, DirectoryRequest>,
}

impl FileTree {
    pub fn node(&self, path: &Path) -> Option<&Node> {
        self.nodes.get(path)
    }

    pub fn resolving_root(&self) -> bool {
        self.root_request.is_some() || self.dirty_document.is_some()
    }

    /// Visibility is not a request generation: hiding never cancels a listing.
    pub fn document_changed(
        &mut self,
        document: PathBuf,
        visible: bool,
        markers: &[String],
    ) -> Vec<Request> {
        self.dirty_document = Some(document);
        self.root_request = None;
        if visible {
            self.show(markers)
        } else {
            Vec::new()
        }
    }

    pub fn show(&mut self, markers: &[String]) -> Vec<Request> {
        let Some(document) = self.dirty_document.clone() else {
            return Vec::new();
        };
        if self.root_request.is_some() {
            return Vec::new();
        }
        self.serial += 1;
        let request = RootRequest {
            id: self.serial,
            document,
            markers: markers.to_vec(),
        };
        self.root_request = Some(request.clone());
        vec![Request::Root(request)]
    }

    pub fn complete(&mut self, completion: Completion) -> Vec<Request> {
        match completion {
            Completion::Root(request, root) => {
                if self.root_request.as_ref() != Some(&request) {
                    return Vec::new();
                }
                self.root_request = None;
                self.dirty_document = None;
                if self.root.as_ref() == Some(&root) {
                    return Vec::new();
                }
                self.generation += 1;
                self.pending.clear();
                self.nodes.clear();
                let mut node = Node::new(Entry {
                    name: root.file_name().unwrap_or(root.as_os_str()).to_owned(),
                    path: root.clone(),
                    directory: true,
                    expandable: true,
                });
                node.expanded = true;
                self.nodes.insert(root.clone(), node);
                self.root = Some(root.clone());
                self.cursor = Some(root.clone());
                self.fetch(&root).into_iter().collect()
            }
            Completion::Directory(request, result) => {
                if request.generation != self.generation
                    || self.pending.get(&request.path) != Some(&request)
                    || !self.nodes.contains_key(&request.path)
                {
                    return Vec::new();
                }
                self.pending.remove(&request.path);
                let root = self.root.as_ref() == Some(&request.path);
                match result {
                    Err(()) => {
                        self.nodes.get_mut(&request.path).unwrap().error = true;
                        if root {
                            self.cursor = self.root.clone();
                        }
                    }
                    Ok(entries) => self.replace_children(&request.path, entries),
                }
                Vec::new()
            }
        }
    }

    fn fetch(&mut self, path: &Path) -> Option<Request> {
        if self.pending.contains_key(path) {
            return None;
        }
        self.serial += 1;
        let request = DirectoryRequest {
            id: self.serial,
            generation: self.generation,
            root: self.root.clone().expect("listing requires a tree root"),
            path: path.to_owned(),
        };
        self.pending.insert(path.to_owned(), request.clone());
        Some(Request::Directory(request))
    }

    pub fn rows(&self) -> Vec<(PathBuf, usize)> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.append_rows(root, 0, &mut rows);
        }
        rows
    }

    fn append_rows(&self, path: &Path, depth: usize, rows: &mut Vec<(PathBuf, usize)>) {
        let Some(node) = self.nodes.get(path) else {
            return;
        };
        rows.push((path.to_owned(), depth));
        if node.expanded && !(self.root.as_deref() == Some(path) && node.error) {
            for child in &node.children {
                self.append_rows(child, depth + 1, rows);
            }
        }
    }

    fn discard_descendants(&mut self, path: &Path) {
        self.nodes
            .retain(|key, _| key == path || !key.starts_with(path));
        self.pending.retain(|key, _| !key.starts_with(path));
        if let Some(node) = self.nodes.get_mut(path) {
            node.children.clear();
        }
    }

    fn replace_children(&mut self, path: &Path, entries: Vec<Entry>) {
        // Capture old sibling order before removing anything. Row numbers cannot
        // express the deletion fallback when inserted siblings change the order.
        let previous_siblings = self
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.parent())
            .and_then(|parent| self.nodes.get(parent))
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let children: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        let old_children = self.nodes[path].children.clone();
        for old in old_children {
            if !children.contains(&old) {
                self.discard_descendants(&old);
                self.nodes.remove(&old);
            }
        }
        for entry in entries {
            let changed_kind = self.nodes.get(&entry.path).is_some_and(|node| {
                node.entry.expandable != entry.expandable || node.entry.directory != entry.directory
            });
            if changed_kind {
                self.discard_descendants(&entry.path);
                self.nodes.remove(&entry.path);
            }
            if let Some(node) = self.nodes.get_mut(&entry.path) {
                node.entry = entry;
            } else {
                self.nodes.insert(entry.path.clone(), Node::new(entry));
            }
        }
        let node = self.nodes.get_mut(path).unwrap();
        node.children = children;
        node.error = false;
        if let Some(cursor) = self.cursor.clone() {
            self.cursor = Some(self.surviving_cursor(cursor, &previous_siblings));
        }
    }

    fn surviving_cursor(&self, cursor: PathBuf, siblings: &[PathBuf]) -> PathBuf {
        if self.nodes.contains_key(&cursor) {
            return cursor;
        }
        if let Some(index) = siblings.iter().position(|path| path == &cursor) {
            if let Some(next) = siblings[index + 1..]
                .iter()
                .find(|path| self.nodes.contains_key(*path))
            {
                return next.clone();
            }
            if let Some(previous) = siblings[..index]
                .iter()
                .rev()
                .find(|path| self.nodes.contains_key(*path))
            {
                return previous.clone();
            }
        }
        // If the parent itself disappeared, select the nearest surviving
        // ancestor, not a sibling of the deleted parent.
        cursor
            .ancestors()
            .skip(1)
            .find(|path| self.nodes.contains_key(*path))
            .expect("the root survives a successful listing")
            .to_owned()
    }

    pub fn act(&mut self, action: Action) -> Vec<Request> {
        if self.resolving_root() {
            return Vec::new();
        }
        let Some(cursor) = self.cursor.clone() else {
            return Vec::new();
        };
        if self
            .root
            .as_ref()
            .is_some_and(|root| self.nodes[root].error)
            && !matches!(action, Action::Refresh)
        {
            return Vec::new();
        }
        let rows = self.rows();
        let index = rows
            .iter()
            .position(|(path, _)| path == &cursor)
            .unwrap_or(0);
        let mut requests = Vec::new();
        match action {
            Action::Next => {
                self.cursor = rows
                    .get((index + 1).min(rows.len() - 1))
                    .map(|row| row.0.clone())
            }
            Action::Previous => {
                self.cursor = rows.get(index.saturating_sub(1)).map(|row| row.0.clone())
            }
            Action::First => self.cursor = self.root.clone(),
            Action::Last => self.cursor = rows.last().map(|row| row.0.clone()),
            Action::Left | Action::Right | Action::Toggle => {
                let node = &self.nodes[&cursor];
                let collapse = node.expanded && matches!(action, Action::Left | Action::Toggle);
                if collapse {
                    self.discard_descendants(&cursor);
                    self.nodes.get_mut(&cursor).unwrap().expanded = false;
                } else if node.entry.expandable && !node.expanded && !matches!(action, Action::Left)
                {
                    self.nodes.get_mut(&cursor).unwrap().expanded = true;
                    requests.extend(self.fetch(&cursor));
                } else if matches!(action, Action::Right) && node.expanded {
                    if let Some(child) = node.children.first() {
                        self.cursor = Some(child.clone());
                    }
                } else if matches!(action, Action::Left) && self.root.as_ref() != Some(&cursor) {
                    self.cursor = cursor.parent().map(Path::to_owned);
                }
            }
            Action::Refresh => {
                let expanded: Vec<_> = rows
                    .into_iter()
                    .filter_map(|(path, _)| {
                        (self.root.as_ref() == Some(&path) || self.nodes[&path].expanded)
                            .then_some(path)
                    })
                    .collect();
                for path in expanded {
                    requests.extend(self.fetch(&path));
                }
            }
        }
        requests
    }
}

#[cfg(test)]
mod tests;
