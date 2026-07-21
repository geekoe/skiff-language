use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use skiff_artifact_identity::ArtifactRelativePath;

use super::{
    error::{io_error, EcosystemStorageError, StorageResult},
    strict_json::StrictJsonValue,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct CanonicalArtifactStore {
    root: PathBuf,
}

impl CanonicalArtifactStore {
    pub fn create(root: impl AsRef<Path>) -> StorageResult<Self> {
        let root = root.as_ref();
        fs::create_dir_all(root)
            .map_err(|source| io_error("create artifact root", root, source))?;
        Self::open(root)
    }

    pub fn open(root: impl AsRef<Path>) -> StorageResult<Self> {
        let root = root.as_ref();
        let root = fs::canonicalize(root)
            .map_err(|source| io_error("resolve artifact root", root, source))?;
        if !root.is_dir() {
            return Err(EcosystemStorageError::InvalidRecord {
                path: root,
                message: "artifact root must be a directory".to_string(),
            });
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn read_bytes(&self, path: &ArtifactRelativePath) -> StorageResult<Vec<u8>> {
        let resolved = path.resolve_existing(&self.root, "canonical ecosystem record")?;
        fs::read(&resolved).map_err(|source| io_error("read", resolved, source))
    }

    pub(crate) fn read_optional_bytes(
        &self,
        path: &ArtifactRelativePath,
    ) -> StorageResult<Option<Vec<u8>>> {
        let joined = self.root.join(path.as_path());
        match fs::symlink_metadata(&joined) {
            Ok(_) => self.read_bytes(path).map(Some),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(io_error("inspect", joined, source)),
        }
    }

    pub(crate) fn write_immutable(
        &self,
        path: &ArtifactRelativePath,
        bytes: &[u8],
    ) -> StorageResult<PathBuf> {
        let destination = self.prepare_destination(path)?;
        if let Ok(existing) = fs::read(&destination) {
            if existing == bytes {
                return Ok(destination);
            }
            return Err(EcosystemStorageError::ImmutableConflict { path: destination });
        }

        let temporary = temporary_path(&destination);
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error("create temporary record", &temporary, source))?;
            file.write_all(bytes)
                .map_err(|source| io_error("write temporary record", &temporary, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync temporary record", &temporary, source))?;
            match fs::hard_link(&temporary, &destination) {
                Ok(()) => Ok(destination.clone()),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = fs::read(&destination).map_err(|source| {
                        io_error("read concurrent record", &destination, source)
                    })?;
                    if existing == bytes {
                        Ok(destination.clone())
                    } else {
                        Err(EcosystemStorageError::ImmutableConflict {
                            path: destination.clone(),
                        })
                    }
                }
                Err(source) => Err(io_error("publish immutable record", &destination, source)),
            }
            .and_then(|published| {
                sync_parent(&published)?;
                Ok(published)
            })
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    pub(crate) fn with_exclusive_pointer_lock<T>(
        &self,
        path: &ArtifactRelativePath,
        action: impl FnOnce(&Path) -> StorageResult<T>,
    ) -> StorageResult<T> {
        let destination = self.prepare_destination(path)?;
        let lock_path = destination.with_extension("lock");
        reject_symlink_or_non_file(&lock_path, "pointer lock")?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| io_error("open pointer lock", &lock_path, source))?;
        lock.lock_exclusive()
            .map_err(|source| io_error("lock pointer", &lock_path, source))?;
        let result = action(&destination);
        let _ = FileExt::unlock(&lock);
        result
    }

    pub(crate) fn replace_locked(&self, destination: &Path, bytes: &[u8]) -> StorageResult<()> {
        let temporary = temporary_path(destination);
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error("create pointer temporary", &temporary, source))?;
            file.write_all(bytes)
                .map_err(|source| io_error("write pointer temporary", &temporary, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync pointer temporary", &temporary, source))?;
            fs::rename(&temporary, destination)
                .map_err(|source| io_error("replace pointer", destination, source))?;
            sync_parent(destination)?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    fn prepare_destination(&self, path: &ArtifactRelativePath) -> StorageResult<PathBuf> {
        let relative_parent = path
            .as_path()
            .parent()
            .expect("canonical relative paths always have a parent");
        let mut resolved_parent = self.root.clone();
        for component in relative_parent.components() {
            let Component::Normal(segment) = component else {
                return Err(EcosystemStorageError::InvalidRecord {
                    path: self.root.join(path.as_path()),
                    message: "canonical record path contains a non-normal component".to_string(),
                });
            };
            let candidate = resolved_parent.join(segment);
            match fs::symlink_metadata(&candidate) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(EcosystemStorageError::InvalidRecord {
                            path: candidate,
                            message: "record path parent must be a real directory, not a symlink"
                                .to_string(),
                        });
                    }
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&candidate) {
                        Ok(()) => sync_directory(&resolved_parent)?,
                        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                            let metadata = fs::symlink_metadata(&candidate).map_err(|source| {
                                io_error("inspect concurrent record directory", &candidate, source)
                            })?;
                            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                                return Err(EcosystemStorageError::InvalidRecord {
                                    path: candidate,
                                    message:
                                        "record path parent must be a real directory, not a symlink"
                                            .to_string(),
                                });
                            }
                        }
                        Err(source) => {
                            return Err(io_error("create record directory", &candidate, source))
                        }
                    }
                }
                Err(source) => return Err(io_error("inspect record directory", candidate, source)),
            }
            resolved_parent = fs::canonicalize(&candidate)
                .map_err(|source| io_error("resolve record directory", &candidate, source))?;
            if !resolved_parent.starts_with(&self.root) {
                return Err(EcosystemStorageError::InvalidRecord {
                    path: resolved_parent,
                    message: format!(
                        "record directory escapes artifact root {}",
                        self.root.display()
                    ),
                });
            }
        }
        let destination = resolved_parent.join(
            path.as_path()
                .file_name()
                .expect("canonical record paths always have a file name"),
        );
        reject_symlink_or_non_file(&destination, "record destination")?;
        Ok(destination)
    }
}

pub(crate) fn canonical_bytes<T: Serialize>(value: &T) -> StorageResult<Vec<u8>> {
    skiff_canonical_json::canonical_json_bytes(value).map_err(|source| {
        EcosystemStorageError::Json {
            path: PathBuf::from("<canonical serialization>"),
            source,
        }
    })
}

pub(crate) fn strict_value(path: &Path, bytes: &[u8]) -> StorageResult<Value> {
    serde_json::from_slice::<StrictJsonValue>(bytes)
        .map(StrictJsonValue::into_inner)
        .map_err(|source| EcosystemStorageError::Json {
            path: path.to_path_buf(),
            source,
        })
}

pub(crate) fn typed_from_value<T: DeserializeOwned>(path: &Path, value: Value) -> StorageResult<T> {
    serde_json::from_value(value).map_err(|source| EcosystemStorageError::Json {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn read_locked_bytes(destination: &Path) -> StorageResult<Option<Vec<u8>>> {
    match File::open(destination) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|source| io_error("read locked pointer", destination, source))?;
            Ok(Some(bytes))
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error("open locked pointer", destination, source)),
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = destination
        .file_name()
        .expect("destination has file name")
        .to_string_lossy();
    destination.with_file_name(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ))
}

fn sync_parent(path: &Path) -> StorageResult<()> {
    let parent = path
        .parent()
        .expect("canonical record destinations always have a parent");
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> StorageResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync record directory", path, source))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> StorageResult<()> {
    Ok(())
}

fn reject_symlink_or_non_file(path: &Path, label: &str) -> StorageResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(EcosystemStorageError::InvalidRecord {
                path: path.to_path_buf(),
                message: format!("{label} must be a regular file, not a symlink"),
            })
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("inspect destination", path, source)),
    }
}
