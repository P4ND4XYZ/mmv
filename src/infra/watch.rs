use std::{
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use notify::{Event, EventKind, RecursiveMode, Watcher as _};

use crate::app::WatchTarget;

const QUIET: Duration = Duration::from_millis(80);

pub enum WatchNotice {
    Ready(WatchTarget, Result<(), String>),
    Changed(WatchTarget),
}

enum Message {
    Target(WatchTarget),
    Event(WatchTarget, notify::Result<Event>),
    Stop,
}

/// All filesystem work and notify teardown belong to this worker, not the UI.
pub struct FileWatch {
    sender: Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl FileWatch {
    pub fn new() -> (Self, Receiver<WatchNotice>) {
        let (sender, messages) = mpsc::channel();
        let (notices, receiver) = mpsc::channel();
        let events = sender.clone();
        let worker = thread::spawn(move || run(messages, events, notices));
        (
            Self {
                sender,
                worker: Some(worker),
            },
            receiver,
        )
    }

    pub fn set_target(&self, target: WatchTarget) {
        let _ = self.sender.send(Message::Target(target));
    }

    /// Signal immediately; the caller joins on a background executor.
    pub fn stop(mut self) -> JoinHandle<()> {
        let _ = self.sender.send(Message::Stop);
        self.worker.take().expect("watch worker")
    }
}

impl Drop for FileWatch {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Stop);
    }
}

#[derive(Default)]
struct Debounce(Option<Instant>);

impl Debounce {
    fn mark(&mut self, now: Instant) {
        self.0 = Some(now + QUIET);
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.0.is_some_and(|deadline| now >= deadline) {
            self.0 = None;
            true
        } else {
            false
        }
    }
}

fn matches_path(path: &Path, target: &Path) -> bool {
    // Use the same canonical identity as document reads whenever the file exists.
    if let Ok(key) = std::fs::canonicalize(path) {
        return key == target;
    }
    // Removed/renamed paths no longer canonicalize. Resolve the parent instead,
    // retaining the watched pathname so deletion followed by recreation works.
    let (Some(parent), Some(name), Some(target_name)) =
        (path.parent(), path.file_name(), target.file_name())
    else {
        return false;
    };
    let Ok(parent) = std::fs::canonicalize(parent) else {
        return false;
    };
    let same_name = if cfg!(windows) {
        name.to_string_lossy().to_lowercase() == target_name.to_string_lossy().to_lowercase()
    } else {
        name == target_name
    };
    same_name && Some(parent.as_path()) == target.parent()
}

fn run(messages: Receiver<Message>, events: Sender<Message>, notices: Sender<WatchNotice>) {
    let mut active: Option<WatchTarget> = None;
    let mut watcher = None;
    let mut debounce = Debounce::default();
    loop {
        let timeout = debounce.0.map_or(Duration::from_secs(3600), |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        match messages.recv_timeout(timeout) {
            Ok(Message::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Message::Target(target)) => {
                if active.as_ref() == Some(&target) {
                    continue;
                }
                watcher.take();
                debounce = Debounce::default();
                let token = target.clone();
                let sender = events.clone();
                let result = (|| {
                    let mut next = notify::recommended_watcher(move |event| {
                        let _ = sender.send(Message::Event(token.clone(), event));
                    })?;
                    let parent = target
                        .key
                        .0
                        .parent()
                        .ok_or_else(|| notify::Error::generic("watch target has no parent"))?;
                    next.watch(parent, RecursiveMode::NonRecursive)?;
                    Ok::<_, notify::Error>(next)
                })();
                let outcome = match result {
                    Ok(next) => {
                        watcher = Some(next);
                        Ok(())
                    }
                    Err(error) => Err(error.to_string()),
                };
                active = Some(target.clone());
                let _ = notices.send(WatchNotice::Ready(target, outcome));
            }
            Ok(Message::Event(target, Ok(event))) if active.as_ref() == Some(&target) => {
                if matches!(
                    event.kind,
                    EventKind::Create(_)
                        | EventKind::Modify(_)
                        | EventKind::Remove(_)
                        | EventKind::Any
                ) && event
                    .paths
                    .iter()
                    .any(|path| matches_path(path, &target.key.0))
                {
                    debounce.mark(Instant::now());
                }
            }
            Ok(Message::Event(_, _)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if debounce.take_due(Instant::now())
            && let Some(target) = &active
            && notices.send(WatchNotice::Changed(target.clone())).is_err()
        {
            break;
        }
    }
    drop(watcher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DocumentKey;

    #[test]
    fn trailing_debounce_uses_explicit_time_and_emits_once() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        debounce.mark(start);
        assert!(!debounce.take_due(start + QUIET / 2));
        debounce.mark(start + QUIET / 2);
        assert!(!debounce.take_due(start + QUIET));
        assert!(debounce.take_due(start + QUIET + QUIET / 2));
        assert!(!debounce.take_due(start + QUIET * 3));
    }

    #[test]
    fn real_notify_tracks_overwrite_replace_recreate_switch_and_shutdown() {
        let dir = std::env::temp_dir().join(format!(
            "mmv-watch-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("MixedCase 日本語.md");
        let b = dir.join("b.md");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let target = |serial, path: &Path| WatchTarget {
            serial,
            key: DocumentKey(std::fs::canonicalize(path).unwrap()),
        };
        let a_target = target(1, &a);
        let b_target = target(2, &b);
        let (watch, notices) = FileWatch::new();
        let ready =
            |expected: &WatchTarget| match notices.recv_timeout(Duration::from_secs(5)).unwrap() {
                WatchNotice::Ready(actual, result) => {
                    assert_eq!(&actual, expected);
                    result.unwrap();
                }
                _ => panic!("expected watcher readiness"),
            };
        let changed =
            |expected: &WatchTarget| match notices.recv_timeout(Duration::from_secs(5)).unwrap() {
                WatchNotice::Changed(actual) => assert_eq!(&actual, expected),
                _ => panic!("expected file change"),
            };
        watch.set_target(a_target.clone());
        ready(&a_target);
        std::fs::write(&b, "unrelated").unwrap();
        assert!(matches!(
            notices.recv_timeout(QUIET * 3),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        std::fs::write(&a, "overwrite").unwrap();
        changed(&a_target);
        let replacement = dir.join("temporary.md");
        std::fs::write(&replacement, "replacement").unwrap();
        std::fs::remove_file(&a).unwrap();
        std::fs::rename(replacement, &a).unwrap();
        changed(&a_target);
        std::fs::remove_file(&a).unwrap();
        changed(&a_target);
        #[cfg(windows)]
        assert!(matches_path(
            &std::path::PathBuf::from(a.to_str().unwrap().to_uppercase().replace('\\', "/")),
            &a_target.key.0
        ));
        std::fs::write(&a, "recreated").unwrap();
        changed(&a_target);
        watch.set_target(b_target.clone());
        ready(&b_target);
        std::fs::write(&a, "old target").unwrap();
        assert!(matches!(
            notices.recv_timeout(QUIET * 3),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        std::fs::write(&b, "current target").unwrap();
        changed(&b_target);
        let returned_a = target(3, &a);
        watch.set_target(returned_a.clone());
        ready(&returned_a);
        // A callback already queued by the first A subscription must not become
        // current again merely because its pathname matches after A → B → A.
        watch
            .sender
            .send(Message::Event(
                a_target,
                Ok(Event::new(EventKind::Any).add_path(a.clone())),
            ))
            .unwrap();
        assert!(matches!(
            notices.recv_timeout(QUIET * 3),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        std::fs::write(&a, "returned target").unwrap();
        changed(&returned_a);
        let worker = watch.stop();
        let (done, stopped) = mpsc::channel();
        thread::spawn(move || {
            worker.join().unwrap();
            done.send(()).unwrap();
        });
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            notices.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
