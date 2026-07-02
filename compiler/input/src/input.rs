use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::{package_config::PACKAGE_CONFIG_FILE, service_config::SERVICE_CONFIG_FILE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationInputKind {
    Package,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRootManifest {
    root: PathBuf,
    kind: PublicationInputKind,
    manifest_path: PathBuf,
}

impl PublicationRootManifest {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind(&self) -> PublicationInputKind {
        self.kind
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}

#[derive(Debug)]
pub enum PublicationInputError {
    AmbiguousRoot {
        root: PathBuf,
        package_manifest_path: PathBuf,
        service_manifest_path: PathBuf,
    },
    MissingRootManifest {
        root: PathBuf,
    },
    InspectRootManifest {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for PublicationInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousRoot { root, .. } => write!(
                formatter,
                "{}: publication root is ambiguous; found both {} and {}",
                root.display(),
                PACKAGE_CONFIG_FILE,
                SERVICE_CONFIG_FILE
            ),
            Self::MissingRootManifest { root } => write!(
                formatter,
                "{}: missing publication root manifest; expected {} or {}",
                root.display(),
                PACKAGE_CONFIG_FILE,
                SERVICE_CONFIG_FILE
            ),
            Self::InspectRootManifest { path, source } => write!(
                formatter,
                "failed to inspect publication root manifest {}: {}",
                path.display(),
                source
            ),
        }
    }
}

impl std::error::Error for PublicationInputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InspectRootManifest { source, .. } => Some(source),
            Self::AmbiguousRoot { .. } | Self::MissingRootManifest { .. } => None,
        }
    }
}

pub fn classify_publication_root(
    root: &Path,
) -> Result<PublicationRootManifest, PublicationInputError> {
    let package_manifest_path = root.join(PACKAGE_CONFIG_FILE);
    let service_manifest_path = root.join(SERVICE_CONFIG_FILE);
    let has_package_manifest = manifest_path_is_file(&package_manifest_path)?;
    let has_service_manifest = manifest_path_is_file(&service_manifest_path)?;

    match (has_package_manifest, has_service_manifest) {
        (true, false) => Ok(PublicationRootManifest {
            root: root.to_path_buf(),
            kind: PublicationInputKind::Package,
            manifest_path: package_manifest_path,
        }),
        (false, true) => Ok(PublicationRootManifest {
            root: root.to_path_buf(),
            kind: PublicationInputKind::Service,
            manifest_path: service_manifest_path,
        }),
        (true, true) => Err(PublicationInputError::AmbiguousRoot {
            root: root.to_path_buf(),
            package_manifest_path,
            service_manifest_path,
        }),
        (false, false) => Err(PublicationInputError::MissingRootManifest {
            root: root.to_path_buf(),
        }),
    }
}

fn manifest_path_is_file(path: &Path) -> Result<bool, PublicationInputError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PublicationInputError::InspectRootManifest {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{package_config::PACKAGE_CONFIG_FILE, service_config::SERVICE_CONFIG_FILE};

    use super::{
        classify_publication_root, PublicationInputError, PublicationInputKind,
        PublicationRootManifest,
    };

    static NEXT_TEST_ROOT_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn classifies_package_manifest_root() {
        let root = TestRoot::new("package");
        root.write_manifest(PACKAGE_CONFIG_FILE);

        assert_root_manifest(
            classify_publication_root(root.path()).expect("package root should classify"),
            root.path(),
            PublicationInputKind::Package,
            PACKAGE_CONFIG_FILE,
        );
    }

    #[test]
    fn classifies_service_manifest_root() {
        let root = TestRoot::new("service");
        root.write_manifest(SERVICE_CONFIG_FILE);

        assert_root_manifest(
            classify_publication_root(root.path()).expect("service root should classify"),
            root.path(),
            PublicationInputKind::Service,
            SERVICE_CONFIG_FILE,
        );
    }

    #[test]
    fn rejects_root_without_publication_manifest() {
        let root = TestRoot::new("missing");

        let error =
            classify_publication_root(root.path()).expect_err("missing root should be rejected");
        let PublicationInputError::MissingRootManifest { root: error_root } = error else {
            panic!("expected missing manifest error, got {error:?}");
        };
        assert_eq!(error_root, root.path());
    }

    #[test]
    fn rejects_ambiguous_publication_root() {
        let root = TestRoot::new("ambiguous");
        root.write_manifest(PACKAGE_CONFIG_FILE);
        root.write_manifest(SERVICE_CONFIG_FILE);

        let error =
            classify_publication_root(root.path()).expect_err("ambiguous root should be rejected");
        let PublicationInputError::AmbiguousRoot {
            root: error_root,
            package_manifest_path,
            service_manifest_path,
        } = error
        else {
            panic!("expected ambiguous root error, got {error:?}");
        };
        assert_eq!(error_root, root.path());
        assert_eq!(package_manifest_path, root.path().join(PACKAGE_CONFIG_FILE));
        assert_eq!(service_manifest_path, root.path().join(SERVICE_CONFIG_FILE));
    }

    fn assert_root_manifest(
        manifest: PublicationRootManifest,
        root: &Path,
        kind: PublicationInputKind,
        manifest_file_name: &str,
    ) {
        assert_eq!(manifest.root(), root);
        assert_eq!(manifest.kind(), kind);
        assert_eq!(manifest.manifest_path(), root.join(manifest_file_name));
    }

    struct TestRoot {
        path: PathBuf,
    }

    impl TestRoot {
        fn new(name: &str) -> Self {
            let id = NEXT_TEST_ROOT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "skiff-publication-input-{name}-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).expect("test root should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write_manifest(&self, file_name: &str) {
            fs::write(self.path.join(file_name), "{}\n").expect("manifest should be written");
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
