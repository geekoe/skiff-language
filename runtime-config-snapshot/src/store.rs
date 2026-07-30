use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use skiff_artifact_model::RuntimeConfigSnapshotRef;
use uuid::Uuid;

use crate::{
    error::{invalid, io_error},
    strict_json::StrictJsonValue,
    RuntimeConfigSnapshot, RuntimeConfigSnapshotError, RuntimeConfigSnapshotResolver,
    RuntimeConfigSnapshotResult, MAX_CONFIG_SNAPSHOT_BYTES,
};

const STORE_DIRECTORY_MODE: u32 = 0o700;
const SNAPSHOT_FILE_MODE: u32 = 0o600;

#[derive(Debug, Clone)]
pub struct RuntimeConfigSnapshotStore {
    root: PathBuf,
    snapshots: PathBuf,
}

impl RuntimeConfigSnapshotStore {
    pub fn create(root: impl AsRef<Path>) -> RuntimeConfigSnapshotResult<Self> {
        let root = root.as_ref();
        match fs::symlink_metadata(root) {
            Ok(metadata) => validate_directory(root, &metadata)?,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(STORE_DIRECTORY_MODE)
                    .create(root)
                    .map_err(|source| io_error("create snapshot root", root, source))?;
            }
            Err(source) => return Err(io_error("inspect snapshot root", root, source)),
        }
        Self::open(root)
    }

    pub fn open(root: impl AsRef<Path>) -> RuntimeConfigSnapshotResult<Self> {
        let root = root.as_ref();
        let metadata = fs::symlink_metadata(root)
            .map_err(|source| io_error("inspect snapshot root", root, source))?;
        validate_directory(root, &metadata)?;
        let root = fs::canonicalize(root)
            .map_err(|source| io_error("resolve snapshot root", root, source))?;
        let snapshots = root.join("snapshots");
        match fs::symlink_metadata(&snapshots) {
            Ok(metadata) => validate_directory(&snapshots, &metadata)?,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(STORE_DIRECTORY_MODE)
                    .create(&snapshots)
                    .map_err(|source| {
                        io_error("create snapshot records directory", &snapshots, source)
                    })?;
                sync_directory(&root)?;
            }
            Err(source) => {
                return Err(io_error(
                    "inspect snapshot records directory",
                    &snapshots,
                    source,
                ))
            }
        }
        Ok(Self { root, snapshots })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn publish(
        &self,
        snapshot: &RuntimeConfigSnapshot,
    ) -> RuntimeConfigSnapshotResult<PathBuf> {
        snapshot.validate(Path::new("<publish>"))?;
        let bytes = skiff_canonical_json::canonical_json_bytes(snapshot).map_err(|source| {
            RuntimeConfigSnapshotError::Json {
                path: PathBuf::from("<publish>"),
                source,
            }
        })?;
        if bytes.len() as u64 > MAX_CONFIG_SNAPSHOT_BYTES {
            return Err(invalid(
                "<publish>",
                format!("encoded snapshot exceeds {MAX_CONFIG_SNAPSHOT_BYTES} byte limit"),
            ));
        }
        let destination = self.path_for(snapshot.snapshot_ref());
        reject_existing_target(&destination)?;
        let temporary = self
            .snapshots
            .join(format!(".{}.tmp", Uuid::new_v4().simple()));
        let result = (|| {
            let mut file = secure_create_new(&temporary)?;
            file.write_all(&bytes)
                .map_err(|source| io_error("write snapshot temporary", &temporary, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync snapshot temporary", &temporary, source))?;
            match fs::hard_link(&temporary, &destination) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(RuntimeConfigSnapshotError::AlreadyExists {
                        path: destination.clone(),
                    });
                }
                Err(source) => {
                    return Err(io_error(
                        "atomically publish snapshot",
                        &destination,
                        source,
                    ))
                }
            }
            fs::remove_file(&temporary)
                .map_err(|source| io_error("remove snapshot temporary", &temporary, source))?;
            sync_directory(&self.snapshots)?;
            Ok(destination.clone())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    pub fn read(
        &self,
        reference: &RuntimeConfigSnapshotRef,
    ) -> RuntimeConfigSnapshotResult<RuntimeConfigSnapshot> {
        skiff_artifact_model::validate_runtime_config_snapshot_ref(reference)
            .map_err(|message| invalid("<snapshot-ref>", message))?;
        let path = self.path_for(reference);
        let mut file = secure_open_existing(&path)?;
        let metadata = file
            .metadata()
            .map_err(|source| io_error("inspect snapshot", &path, source))?;
        validate_snapshot_file(&path, &metadata)?;
        if metadata.len() > MAX_CONFIG_SNAPSHOT_BYTES {
            return Err(invalid(
                &path,
                format!("encoded snapshot exceeds {MAX_CONFIG_SNAPSHOT_BYTES} byte limit"),
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes)
            .map_err(|source| io_error("read snapshot", &path, source))?;
        let value = serde_json::from_slice::<StrictJsonValue>(&bytes).map_err(|source| {
            RuntimeConfigSnapshotError::Json {
                path: path.clone(),
                source,
            }
        })?;
        let snapshot = serde_json::from_value::<RuntimeConfigSnapshot>(value.into_inner())
            .map_err(|source| RuntimeConfigSnapshotError::Json {
                path: path.clone(),
                source,
            })?;
        snapshot.validate(&path)?;
        if snapshot.snapshot_ref() != reference {
            return Err(invalid(&path, "snapshot id/path mismatch"));
        }
        let canonical =
            skiff_canonical_json::canonical_json_bytes(&snapshot).map_err(|source| {
                RuntimeConfigSnapshotError::Json {
                    path: path.clone(),
                    source,
                }
            })?;
        if canonical != bytes {
            return Err(invalid(&path, "snapshot bytes are not canonical JSON"));
        }
        Ok(snapshot)
    }

    fn path_for(&self, reference: &RuntimeConfigSnapshotRef) -> PathBuf {
        self.snapshots
            .join(format!("{}.json", reference.snapshot_id.random_suffix()))
    }
}

impl RuntimeConfigSnapshotResolver for RuntimeConfigSnapshotStore {
    type Error = RuntimeConfigSnapshotError;

    fn resolve(
        &self,
        reference: &RuntimeConfigSnapshotRef,
    ) -> Result<RuntimeConfigSnapshot, Self::Error> {
        self.read(reference)
    }
}

fn secure_create_new(path: &Path) -> RuntimeConfigSnapshotResult<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(SNAPSHOT_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| io_error("create snapshot temporary", path, source))
}

fn secure_open_existing(path: &Path) -> RuntimeConfigSnapshotResult<File> {
    match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(RuntimeConfigSnapshotError::NotFound {
                path: path.to_path_buf(),
            })
        }
        Err(source) => Err(io_error("open snapshot", path, source)),
    }
}

fn reject_existing_target(path: &Path) -> RuntimeConfigSnapshotResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(RuntimeConfigSnapshotError::AlreadyExists {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("inspect snapshot destination", path, source)),
    }
}

fn validate_directory(path: &Path, metadata: &fs::Metadata) -> RuntimeConfigSnapshotResult<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(
            path,
            "snapshot directory must be a real directory, not a symlink",
        ));
    }
    if metadata.permissions().mode() & 0o777 != STORE_DIRECTORY_MODE {
        return Err(invalid(
            path,
            format!("snapshot directory permissions must be {STORE_DIRECTORY_MODE:o}"),
        ));
    }
    Ok(())
}

fn validate_snapshot_file(path: &Path, metadata: &fs::Metadata) -> RuntimeConfigSnapshotResult<()> {
    if !metadata.is_file() {
        return Err(invalid(path, "snapshot path must be a regular file"));
    }
    if metadata.permissions().mode() & 0o777 != SNAPSHOT_FILE_MODE {
        return Err(invalid(
            path,
            format!("snapshot file permissions must be {SNAPSHOT_FILE_MODE:o}"),
        ));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> RuntimeConfigSnapshotResult<()> {
    let directory = File::open(path)
        .map_err(|source| io_error("open snapshot directory for sync", path, source))?;
    directory
        .sync_all()
        .map_err(|source| io_error("sync snapshot directory", path, source))
}
