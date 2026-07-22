/// Integration tests for the SyncQueue.
///
/// Run with:
///   cargo test --test test_queue
use tuxdrive_sync::{SyncDirection, SyncQueue, SyncTask};

fn make_upload(path: &str) -> SyncTask {
    SyncTask::upload(path.to_owned(), 1024, "application/octet-stream".into())
}

#[test]
fn enqueue_dequeue_single() {
    let q = SyncQueue::new();
    let task = make_upload("foo.txt");
    assert!(q.enqueue(task.clone()), "first enqueue must succeed");
    assert_eq!(q.len(), 1);
    let out = q.dequeue().expect("should have a task");
    assert_eq!(out.local_path, "foo.txt");
    assert!(q.is_empty());
}

#[test]
fn dedup_prevents_duplicate_paths() {
    let q = SyncQueue::new();
    let t1 = make_upload("dup.txt");
    let t2 = make_upload("dup.txt"); // same path
    assert!(q.enqueue(t1), "first enqueue must succeed");
    assert!(!q.enqueue(t2), "second enqueue same path must be rejected");
    assert_eq!(q.len(), 1);
}

#[test]
fn remove_path_removes_correct_task() {
    let q = SyncQueue::new();
    q.enqueue(make_upload("a.txt"));
    q.enqueue(make_upload("b.txt"));
    assert_eq!(q.len(), 2);
    q.remove_path("a.txt");
    assert_eq!(q.len(), 1);
    let task = q.dequeue().unwrap();
    assert_eq!(task.local_path, "b.txt");
}

#[test]
fn fifo_ordering() {
    let q = SyncQueue::new();
    q.enqueue(make_upload("first.txt"));
    q.enqueue(make_upload("second.txt"));
    q.enqueue(make_upload("third.txt"));
    assert_eq!(q.dequeue().unwrap().local_path, "first.txt");
    assert_eq!(q.dequeue().unwrap().local_path, "second.txt");
    assert_eq!(q.dequeue().unwrap().local_path, "third.txt");
    assert!(q.dequeue().is_none());
}

#[test]
fn download_task_has_drive_file_id() {
    let t = SyncTask::download("file.pdf".into(), "drive-id-123".into(), 5000);
    assert_eq!(t.drive_file_id, Some("drive-id-123".into()));
    assert!(matches!(t.direction, SyncDirection::Download));
}

#[test]
fn delete_remote_task() {
    let t = SyncTask::delete_remote("gone.txt".into(), "drive-id-456".into());
    assert!(matches!(t.direction, SyncDirection::Delete));
}

#[test]
fn delete_local_task() {
    let t = SyncTask::delete_local("local_only.txt".into());
    assert!(matches!(t.direction, SyncDirection::DeleteLocal));
    assert_eq!(t.local_path, "local_only.txt");
}
