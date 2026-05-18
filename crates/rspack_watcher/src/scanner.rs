use std::{ops::Deref, sync::Arc, time::SystemTime};

use rspack_paths::ArcPath;
use tokio::sync::mpsc::UnboundedSender;

use super::{FsEvent, FsEventKind, PathManager};
use crate::EventBatch;

// Scanner inspects registered paths at watch startup.
//
// Two responsibilities:
// 1. `scan_path_missing` — for file/directory deps that are not present on disk,
//    reclassify them into the `missing` tracker (synchronous). Downstream the
//    analyzer + `DependencyFinder` treat missing entries as paths to watch for
//    future creation — the same way watchpack handles absent `fileDependencies`.
//    No Remove event is emitted here; changes for paths served outside the real
//    fs (e.g. virtual modules) are delivered through `FsWatcher::trigger_event`
//    by the owning plugin.
// 2. `scan_path_changed` — for paths that do exist on disk with an mtime newer
//    than `start_time`, emit Change so late modifications are not missed.
pub struct Scanner {
  path_manager: Arc<PathManager>,
  tx: Option<UnboundedSender<EventBatch>>,
}

impl Scanner {
  /// Creates a new `Scanner` that will send events to the provided sender when paths are scanned.
  pub fn new(tx: UnboundedSender<EventBatch>, path_manager: Arc<PathManager>) -> Self {
    Self {
      path_manager,
      tx: Some(tx),
    }
  }

  /// Performs the startup scan. Step 1 (reclassify missing) runs synchronously
  /// so the analyzer — which runs immediately after `scan` returns — sees the
  /// updated `missing` tracker. Step 2 (change detection) is dispatched to the
  /// tokio runtime so it does not block the caller.
  pub fn scan(&self, start_time: SystemTime) {
    if let Some(tx) = self.tx.clone() {
      let (files, directories) = {
        let accessor = self.path_manager.access();
        let files = accessor
          .files()
          .1
          .iter()
          .map(|file| file.deref().clone())
          .collect::<Vec<_>>();
        let directories = accessor
          .directories()
          .1
          .iter()
          .map(|dir| dir.deref().clone())
          .collect::<Vec<_>>();
        (files, directories)
      };

      scan_path_missing(&files, &self.path_manager);
      scan_path_missing(&directories, &self.path_manager);

      tokio::spawn(async move {
        _ = scan_path_changed(&files, &start_time, &tx);
        _ = scan_path_changed(&directories, &start_time, &tx);
      });
    }
  }

  pub fn close(&mut self) {
    // Close the scanner by dropping the sender
    self.tx.take();
  }
}

/// Reclassify paths that are absent from the real filesystem as missing deps.
/// No events are emitted: the watcher waits for them to appear (either via an
/// OS event on the watched parent directory or an explicit `trigger_event`
/// from the owning plugin).
fn scan_path_missing(paths: &[ArcPath], path_manager: &PathManager) {
  for path in paths {
    if !path.exists() {
      path_manager.promote_to_missing(path.clone());
    }
  }
}

fn scan_path_changed(
  paths: &[ArcPath],
  start_time: &SystemTime,
  tx: &UnboundedSender<EventBatch>,
) -> bool {
  let changed_event = paths
    .iter()
    .filter(|path| check_path_metadata(path, start_time))
    .cloned()
    .map(|path| FsEvent {
      path,
      kind: FsEventKind::Change,
    })
    .collect::<Vec<_>>();

  if changed_event.is_empty() {
    return true;
  }
  tx.send(changed_event).is_ok()
}

fn check_path_metadata(filepath: &ArcPath, start_time: &SystemTime) -> bool {
  if let Ok(m_time) = filepath
    .metadata()
    .and_then(|metadata| metadata.modified().or_else(|_| metadata.created()))
  {
    *start_time < m_time
  } else {
    false
  }
}

#[cfg(test)]
mod tests {
  use rspack_paths::ArcPath;

  use super::*;

  #[tokio::test]
  async fn test_scan_missing_paths_are_promoted_to_missing() {
    // Paths absent from disk at scan time should be reclassified into the
    // `missing` tracker, not reported as Remove events.
    let current_dir = std::env::current_dir().expect("Failed to get current directory");
    let path_manager = PathManager::default();

    let ghost_file: ArcPath = current_dir.join("___ghost_file.txt").into();
    let ghost_dir: ArcPath = current_dir.join("___ghost_dir/a/b/c").into();

    let files = (vec![ghost_file.clone()].into_iter(), vec![].into_iter());
    let dirs = (vec![ghost_dir.clone()].into_iter(), vec![].into_iter());
    let missing = (vec![].into_iter(), vec![].into_iter());
    path_manager.update(files, dirs, missing).unwrap();

    let path_manager = Arc::new(path_manager);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut scanner = Scanner::new(tx, Arc::clone(&path_manager));

    let collector = tokio::spawn(async move {
      let mut collected = Vec::new();
      while let Some(event) = rx.recv().await {
        collected.push(event);
      }
      collected
    });

    scanner.scan(SystemTime::now());
    scanner.close();

    let collected = collector.await.unwrap();
    assert!(
      collected
        .iter()
        .flatten()
        .all(|event| event.kind != FsEventKind::Remove),
      "scan should not emit Remove for missing paths, got: {collected:?}"
    );

    let accessor = path_manager.access();
    let missing_all = accessor.missing().0;
    assert!(
      missing_all.contains(&ghost_file),
      "ghost file should be promoted to missing tracker"
    );
    assert!(
      missing_all.contains(&ghost_dir),
      "ghost directory should be promoted to missing tracker"
    );
  }

  #[tokio::test]
  async fn test_scan_change_emits_for_fresh_file() {
    // A real file whose mtime is after start_time should emit Change.
    let tmp = tempfile::TempDir::new().unwrap();
    let file_path = tmp.path().join("fresh.txt");
    std::fs::write(&file_path, "hello").unwrap();

    let start_time = SystemTime::now() - std::time::Duration::from_secs(10);

    let path_manager = PathManager::default();
    let files = (
      vec![ArcPath::from(file_path.clone())].into_iter(),
      vec![].into_iter(),
    );
    let dirs = (vec![].into_iter(), vec![].into_iter());
    let missing = (vec![].into_iter(), vec![].into_iter());
    path_manager.update(files, dirs, missing).unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut scanner = Scanner::new(tx, Arc::new(path_manager));

    let collector = tokio::spawn(async move {
      let mut collected = Vec::new();
      while let Some(event) = rx.recv().await {
        collected.push(event);
      }
      collected
    });

    scanner.scan(start_time);
    scanner.close();

    let collected = collector.await.unwrap();
    assert!(
      collected
        .iter()
        .flatten()
        .any(|event| event.kind == FsEventKind::Change
          && event.path == ArcPath::from(file_path.clone())),
      "scan should emit Change for file with mtime after start_time, got: {collected:?}"
    );
  }
}
