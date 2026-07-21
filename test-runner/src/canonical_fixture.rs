//! Compatibility-free facade over focused package-test fixture owners.

use skiff_deployment::storage::EcosystemStorageError;
use skiff_syntax::error::CompileError;
use thiserror::Error;

pub use crate::canonical_store::{CanonicalBaseAssembly, CanonicalTestRecords};
pub use crate::package_test_assembly::{
    assemble_package_test_fixture, CanonicalPackageTestEntrypoint, CanonicalPackageTestFixture,
};
pub use crate::runtime_execution::run_package_cases;
pub use crate::test_discovery::{discover_package_test_cases, PackageTestCase};

#[derive(Debug, Error)]
pub enum CanonicalFixtureError {
    #[error("failed to access {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: CompileError,
    },
    #[error(transparent)]
    Storage(#[from] EcosystemStorageError),
    #[error(transparent)]
    Overlay(Box<crate::test_overlay::PackageTestOverlayError>),
    #[error("invalid canonical fixture: {0}")]
    InvalidInput(String),
}

impl From<crate::test_overlay::PackageTestOverlayError> for CanonicalFixtureError {
    fn from(source: crate::test_overlay::PackageTestOverlayError) -> Self {
        Self::Overlay(Box::new(source))
    }
}
