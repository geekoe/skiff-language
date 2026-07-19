#![allow(dead_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(prefix: &str, name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{prefix}-{name}-{}-{timestamp}-{sequence}",
            process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, relative_path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let relative_path = relative_path.as_ref();
        assert!(
            relative_path.is_relative(),
            "fixture file path must be relative: {}",
            relative_path.display()
        );
        let path = self.path.join(relative_path);
        let parent = path
            .parent()
            .expect("fixture file path should have a parent directory");
        fs::create_dir_all(parent).unwrap_or_else(|error| {
            panic!(
                "failed to create fixture parent directory {}: {error}",
                parent.display()
            )
        });
        fs::write(&path, contents).unwrap_or_else(|error| {
            panic!("failed to write fixture file {}: {error}", path.display())
        });
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
