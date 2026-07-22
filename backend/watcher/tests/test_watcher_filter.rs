/// Integration tests for the filesystem event filter.
///
/// Run with:
///   cargo test --test test_watcher_filter
use tuxdrive_watcher::EventFilter;
use std::path::{Path, PathBuf};

fn filter() -> EventFilter {
    EventFilter::new(PathBuf::from("/sync"))
}

fn p(rel: &str) -> PathBuf {
    PathBuf::from("/sync").join(rel)
}

#[test]
fn normal_file_not_ignored() {
    assert!(!filter().should_ignore(&p("documents/report.pdf")));
}

#[test]
fn tuxdrive_prefix_ignored() {
    assert!(filter().should_ignore(&p(".tuxdrive-tmp/something")));
    assert!(filter().should_ignore(&p("dir/.tuxdrive-conflict.xyz")));
}

#[test]
fn tmp_extension_ignored() {
    assert!(filter().should_ignore(&p("downloads/file.tmp")));
    assert!(filter().should_ignore(&p("downloads/file.tuxdrive-tmp")));
    assert!(filter().should_ignore(&p("downloads/file.crdownload")));
    assert!(filter().should_ignore(&p("downloads/file.part")));
}

#[test]
fn hidden_files_ignored() {
    assert!(filter().should_ignore(&p(".hiddenfile")));
    assert!(filter().should_ignore(&p("dir/.hidden")));
}

#[test]
fn office_temp_ignored() {
    assert!(filter().should_ignore(&p("docs/~$report.docx")));
    assert!(filter().should_ignore(&p("~$spreadsheet.xlsx")));
}

#[test]
fn trash_dir_ignored() {
    assert!(filter().should_ignore(&p(".Trash/file.txt")));
    assert!(filter().should_ignore(&p(".trash/file.txt")));
}

#[test]
fn nested_normal_file_not_ignored() {
    assert!(!filter().should_ignore(&p("a/b/c/normal.rs")));
}

#[test]
fn root_dot_ignored() {
    assert!(filter().should_ignore(Path::new(".")));
}
