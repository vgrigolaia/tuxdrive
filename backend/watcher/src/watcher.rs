use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use notify::event::{ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::error::WatcherError;
use crate::event::{EventKind, LocalEvent};
use crate::filter::EventFilter;

pub struct FsWatcher {
    sync_root: PathBuf,
    filter: EventFilter,
    tx: mpsc::Sender<LocalEvent>,
    /// Holds the debouncer alive for as long as the watcher is running.
    _debouncer: Option<Debouncer<notify::INotifyWatcher, FileIdMap>>,
}

impl FsWatcher {
    /// Create a new watcher. Returns the watcher and the receiver end of the event channel.
    pub fn new(sync_root: PathBuf) -> (Self, mpsc::Receiver<LocalEvent>) {
        let (tx, rx) = mpsc::channel(1024);
        let filter = EventFilter::new(sync_root.clone());
        let watcher = Self {
            sync_root,
            filter,
            tx,
            _debouncer: None,
        };
        (watcher, rx)
    }

    /// Start watching. Spawns a background thread for the debouncer callback.
    pub fn start(&mut self) -> Result<(), WatcherError> {
        if self._debouncer.is_some() {
            return Err(WatcherError::AlreadyRunning);
        }

        let sync_root = self.sync_root.clone();
        let filter = self.filter.clone();
        let tx = self.tx.clone();

        let callback = move |result: DebounceEventResult| {
            let events = match result {
                Ok(events) => events,
                Err(errors) => {
                    for e in errors {
                        warn!("Debouncer error: {e}");
                    }
                    return;
                }
            };

            for debounced in events {
                let notify_event = debounced.event;
                let timestamp = SystemTime::now();

                let local_events = convert_event(&notify_event, &sync_root, &filter, timestamp);
                for local_event in local_events {
                    debug!(
                        "FS event {:?} → {:?}",
                        local_event.kind, local_event.relative_path
                    );
                    // blocking_send is fine here: notify runs its callback on a
                    // dedicated thread, not inside a tokio runtime.
                    if tx.blocking_send(local_event).is_err() {
                        // Receiver was dropped — watcher is shutting down, stop quietly.
                        return;
                    }
                }
            }
        };

        let mut debouncer = new_debouncer(Duration::from_millis(500), None, callback)
            .map_err(WatcherError::Notify)?;

        debouncer
            .watcher()
            .watch(self.sync_root.as_ref(), RecursiveMode::Recursive)
            .map_err(WatcherError::Notify)?;

        self._debouncer = Some(debouncer);
        Ok(())
    }

    /// Stop watching.
    pub fn stop(&mut self) -> Result<(), WatcherError> {
        if self._debouncer.is_none() {
            return Err(WatcherError::NotRunning);
        }
        // Dropping the debouncer unregisters all watches and joins the notify thread.
        self._debouncer = None;
        Ok(())
    }
}

/// Convert a single `notify::Event` into zero or more `LocalEvent`s.
fn convert_event(
    event: &notify::Event,
    sync_root: &PathBuf,
    filter: &EventFilter,
    timestamp: SystemTime,
) -> Vec<LocalEvent> {
    use notify::EventKind as NK;

    match &event.kind {
        NK::Create(_) => {
            make_events_single(event, EventKind::Created, sync_root, filter, timestamp)
        }

        NK::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if event.paths.len() == 2 {
                let from = event.paths[0].clone();
                let to = event.paths[1].clone();

                // Filter: if either path is ignored, skip the rename entirely
                if filter.should_ignore(&from) || filter.should_ignore(&to) {
                    return vec![];
                }

                let rel_from = match from.strip_prefix(sync_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => return vec![],
                };
                let rel_to = match to.strip_prefix(sync_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => return vec![],
                };

                // Use `to` as the canonical absolute_path / relative_path for the event
                vec![LocalEvent {
                    relative_path: rel_to,
                    absolute_path: to.clone(),
                    kind: EventKind::Renamed {
                        from: rel_from,
                        to: to,
                    },
                    timestamp,
                }]
            } else if event.paths.len() == 1 {
                // Incomplete rename pair: treat as Delete + Create
                let path = &event.paths[0];
                if filter.should_ignore(path) {
                    return vec![];
                }
                let rel = match path.strip_prefix(sync_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => return vec![],
                };
                vec![
                    LocalEvent {
                        relative_path: rel.clone(),
                        absolute_path: path.clone(),
                        kind: EventKind::Deleted,
                        timestamp,
                    },
                    LocalEvent {
                        relative_path: rel,
                        absolute_path: path.clone(),
                        kind: EventKind::Created,
                        timestamp,
                    },
                ]
            } else {
                vec![]
            }
        }

        NK::Modify(ModifyKind::Data(_)) | NK::Modify(ModifyKind::Any) => {
            make_events_single(event, EventKind::Modified, sync_root, filter, timestamp)
        }

        NK::Modify(_) => {
            make_events_single(event, EventKind::Modified, sync_root, filter, timestamp)
        }

        NK::Remove(_) => {
            make_events_single(event, EventKind::Deleted, sync_root, filter, timestamp)
        }

        // Access, Other, Any — not interesting for sync
        _ => vec![],
    }
}

/// Build one `LocalEvent` per path in `event.paths`, filtering out ignored paths.
fn make_events_single(
    event: &notify::Event,
    kind: EventKind,
    sync_root: &PathBuf,
    filter: &EventFilter,
    timestamp: SystemTime,
) -> Vec<LocalEvent> {
    let mut out = Vec::new();
    for path in &event.paths {
        if filter.should_ignore(path) {
            continue;
        }
        let relative_path = match path.strip_prefix(sync_root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        out.push(LocalEvent {
            relative_path,
            absolute_path: path.clone(),
            kind: kind.clone(),
            timestamp,
        });
    }
    out
}
