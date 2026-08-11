//! Compatibility-free facade over focused test-service fixture owners.

use std::{fmt, io};

use skiff_deployment::storage::EcosystemStorageError;
use skiff_syntax::error::CompileError;
use thiserror::Error;

use crate::SkiffTestResult;

pub use crate::canonical_store::{CanonicalBaseClosure, CanonicalTestRecords};
pub use crate::runtime_execution::run_package_cases;
pub use crate::test_discovery::{discover_test_service_cases, TestServiceCase};
pub use crate::test_service_fixture::{
    assemble_test_service_fixture, CanonicalTestServiceCaseFixture, CanonicalTestServiceEntrypoint,
    CanonicalTestServiceFixture,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPhase {
    Resolve,
    Connect,
    Write,
    Read,
}

impl fmt::Display for HttpPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Resolve => "resolve",
            Self::Connect => "connect",
            Self::Write => "write",
            Self::Read => "read",
        })
    }
}

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
    #[error(
        "HTTP {phase} failed for {target}: kind={kind:?} raw_errno={raw_os_error:?} \
         elapsed={elapsed_ms}ms deadline={deadline_ms}ms: {source}"
    )]
    Http {
        phase: HttpPhase,
        target: String,
        kind: io::ErrorKind,
        raw_os_error: Option<i32>,
        elapsed_ms: u128,
        deadline_ms: u128,
        #[source]
        source: io::Error,
    },
    #[error("wire decode failed for {context}: {message}")]
    Wire { context: String, message: String },
    #[error("control request returned typed HTTP {status} {code}: {message}")]
    RemoteControl {
        status: u16,
        code: String,
        message: String,
    },
    #[error("suite execution stopped at {module_path}::{name}: {source}")]
    SuiteExecution {
        completed: Vec<SkiffTestResult>,
        module_path: String,
        name: String,
        #[source]
        source: Box<CanonicalFixtureError>,
    },
    #[error("invalid canonical fixture: {0}")]
    InvalidInput(String),
}

/// Shared guidance appended to user-facing `InvalidInput` diagnostics when a service test
/// was invoked directly without the engine test orchestrator.
pub(crate) const SERVICE_TEST_FIXTURE_GUIDANCE: &str =
    "service tests must run through the engine test orchestrator \
     (skiff test <test-root> --artifact-root <store> --sources <manifest.json> [--shards <n>]), \
     which publishes sources incrementally and supplies exact dependency baselines";
