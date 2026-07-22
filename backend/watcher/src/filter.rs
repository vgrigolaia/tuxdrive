use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct EventFilter {
    pub sync_root: PathBuf,
    pub ignore_hidden: bool,
}

impl EventFilter {
    pub fn new(sync_root: PathBuf) -> Self {
        Self {
            sync_root,
            ignore_hidden: true,
        }
    }

    /// Returns true if this path should be IGNORED (not synced).
    pub fn should_ignore(&self, path: &Path) -> bool {
        // Ignore the path "." itself
        if path == Path::new(".") {
            return true;
        }

        // Work on the path relative to sync_root when possible, otherwise use as-is
        let check_path = path
            .strip_prefix(&self.sync_root)
            .unwrap_or(path);

        // Examine every component
        for component in check_path.components() {
            let s = match component.as_os_str().to_str() {
                Some(s) => s,
                None => continue,
            };

            // 1. Any component starting with `.tuxdrive-`
            if s.starts_with(".tuxdrive-") {
                return true;
            }

            // 4. Office temp files: any component starting with `~$`
            if s.starts_with("~$") {
                return true;
            }

            // 3. Hidden files (any component starting with `.`) — default true
            if self.ignore_hidden && s.starts_with('.') {
                return true;
            }

            // 5. Files in `.Trash` or `.trash` directories
            if s == ".Trash" || s == ".trash" {
                return true;
            }
        }

        // 2. Temp files by extension
        if let Some(ext) = check_path.extension().and_then(|e| e.to_str()) {
            match ext {
                "tmp" | "tuxdrive-tmp" | "crdownload" | "part" => return true,
                _ => {}
            }
        }

        // Also handle compound extension `.tuxdrive-tmp` (the "extension" as seen by Path
        // is only the last dot-segment, so check the full file name too)
        if let Some(name) = check_path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".tuxdrive-tmp") {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> EventFilter {
        EventFilter::new(PathBuf::from("/sync"))
    }

    #[test]
    fn ignores_dot_itself() {
        assert!(filter().should_ignore(Path::new(".")));
    }

    #[test]
    fn ignores_tuxdrive_prefix() {
        assert!(filter().should_ignore(Path::new("/sync/.tuxdrive-meta")));
        assert!(filter().should_ignore(Path::new("/sync/subdir/.tuxdrive-lock")));
    }

    #[test]
    fn ignores_tmp_extensions() {
        assert!(filter().should_ignore(Path::new("/sync/file.tmp")));
        assert!(filter().should_ignore(Path::new("/sync/file.crdownload")));
        assert!(filter().should_ignore(Path::new("/sync/file.part")));
        assert!(filter().should_ignore(Path::new("/sync/file.tuxdrive-tmp")));
    }

    #[test]
    fn ignores_hidden_files() {
        assert!(filter().should_ignore(Path::new("/sync/.hidden")));
        assert!(filter().should_ignore(Path::new("/sync/dir/.hidden")));
    }

    #[test]
    fn ignores_office_temp() {
        assert!(filter().should_ignore(Path::new("/sync/~$document.docx")));
    }

    #[test]
    fn ignores_trash() {
        assert!(filter().should_ignore(Path::new("/sync/.Trash/file.txt")));
        assert!(filter().should_ignore(Path::new("/sync/.trash/file.txt")));
    }

    #[test]
    fn allows_normal_files() {
        assert!(!filter().should_ignore(Path::new("/sync/document.txt")));
        assert!(!filter().should_ignore(Path::new("/sync/subdir/photo.jpg")));
    }
}
