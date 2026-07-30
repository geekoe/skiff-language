//! Canonical package/service test infrastructure.
//!
//! A `kind: test` service compiles into an ordinary immutable `PackageArtifact`.
//! All selected cases for that service share the compile, resolved config and dependency
//! graph, then become roots of one multi-root `RuntimeAssembly` committed by one activation
//! transaction. The assembly and activation generation are service-run scoped; an individual
//! case does not own a separate assembly or generation.
//!
//! Each case still receives its own synthetic `ServiceDeployment`, `ServiceContract`, gateway
//! entry, ingress binding, generated service identity, config snapshot partition, heap, effect
//! registry and execution nonce. Sharing an assembly does not share a deployment or mutable state.
//! Every root dispatch receives a new opaque `testCaseCapability`; direct and recursive spawn
//! requests inherit that exact capability instead of creating or borrowing one from another root.

use std::{
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler::CompilerPlatformSources;
use thiserror::Error;

pub mod canonical_fixture;
pub mod canonical_package;
pub mod canonical_std_seed;
pub mod canonical_store;
mod canonical_test_gateway;
mod inline_effects;
pub mod package_service_host_fixture;
pub mod runtime_execution;
pub mod test_discovery;
pub mod test_service_fixture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkiffTestSummary {
    pub passed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub results: Vec<SkiffTestResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkiffTestResult {
    pub module_path: String,
    pub name: String,
    pub passed: bool,
    pub skipped: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SkiffTestOptions {
    pub live: bool,
    pub artifact_root: Option<PathBuf>,
    /// The single validated platform trust owner for every compile in this run.
    pub platform_sources: CompilerPlatformSources,
    /// Harness-owned writable canonical root. It has no public CLI spelling.
    pub runtime_artifact_root: Option<PathBuf>,
    pub base_assembly: Option<String>,
    pub base_config_snapshot: Option<String>,
    pub activation_url: Option<String>,
    pub ingress_url: Option<String>,
    /// Router/Runtime activation target; it never selects a test service config profile.
    pub target_environment: String,
    pub expected_generation: u64,
}

#[derive(Debug, Error)]
pub enum SkiffTestError {
    #[error("failed to inspect input {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("canonical package compile failed: {0}")]
    PackageCompile(#[from] canonical_package::CanonicalPackageProjectError),
    #[error("canonical test fixture failed: {0}")]
    Fixture(#[source] Box<canonical_fixture::CanonicalFixtureError>),
    #[error("input {path} is not inside a package source root")]
    MissingPackageRoot { path: String },
    #[error("canonical execution requires --activation-url, --ingress-url and --artifact-root")]
    MissingCanonicalRuntime,
    #[error("live tests require an explicit file and the complete canonical runtime target")]
    InvalidLiveOptions,
    #[error(
        "non-live tests require a harness-owned runtime artifact root outside --artifact-root"
    )]
    MissingIsolatedRuntimeRoot,
}

impl From<canonical_fixture::CanonicalFixtureError> for SkiffTestError {
    fn from(source: canonical_fixture::CanonicalFixtureError) -> Self {
        Self::Fixture(Box::new(source))
    }
}

pub fn validate_activation_url(value: &str) -> Result<(), String> {
    let Some(rest) = value.strip_prefix("http://") else {
        return Err("activation URL must use http://".to_string());
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return Err("activation URL must include /__skiff/activate-assembly".to_string());
    };
    if !is_canonical_http_authority(authority) || path != "__skiff/activate-assembly" {
        return Err("activation URL must point exactly to /__skiff/activate-assembly".to_string());
    }
    Ok(())
}

pub fn validate_ingress_url(value: &str) -> Result<(), String> {
    let authority = value
        .strip_prefix("http://")
        .ok_or_else(|| "ingress URL must use http://".to_string())?
        .strip_suffix('/')
        .unwrap_or_else(|| value.strip_prefix("http://").expect("prefix was checked"));
    if !is_canonical_http_authority(authority) {
        return Err(
            "ingress URL must be an http:// origin without path, query or fragment".to_string(),
        );
    }
    Ok(())
}

fn is_canonical_http_authority(authority: &str) -> bool {
    !authority.is_empty()
        && authority.trim() == authority
        && !authority
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'?' | b'#' | b'@' | b'\\'))
}

pub fn run_skiff_tests_with_options(
    input: &Path,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    let metadata = fs::metadata(input).map_err(|source| SkiffTestError::Metadata {
        path: input.display().to_string(),
        source,
    })?;
    if options.live && !metadata.is_file() {
        return Err(SkiffTestError::InvalidLiveOptions);
    }
    let package_root =
        canonical_package::find_package_root(input, metadata.is_file()).ok_or_else(|| {
            SkiffTestError::MissingPackageRoot {
                path: input.display().to_string(),
            }
        })?;
    let artifact_root = options
        .artifact_root
        .as_deref()
        .ok_or(SkiffTestError::MissingCanonicalRuntime)?;
    let project = canonical_package::compile_package_project_for_test(
        &options.platform_sources,
        &package_root,
        artifact_root,
    )?;
    let cases =
        canonical_fixture::discover_test_service_cases(input, &package_root, metadata.is_file())?;
    if cases.is_empty() {
        return Ok(SkiffTestSummary {
            passed: 0,
            skipped: 0,
            failed: 0,
            results: Vec::new(),
        });
    }
    if project.test_service_profile.is_none() {
        return Err(canonical_fixture::CanonicalFixtureError::InvalidInput(
            "test execution requires service.yml kind: test; package test overlays are unsupported"
                .to_string(),
        )
        .into());
    }

    // Execution is deliberately all-or-nothing: a source compile is not reported as a passed
    // runtime test. The isolated runtime owner supplies both inputs after publishing the fixture.
    let runtime_artifact_root = if options.live {
        artifact_root
    } else {
        let runtime_artifact_root = options
            .runtime_artifact_root
            .as_deref()
            .ok_or(SkiffTestError::MissingIsolatedRuntimeRoot)?;
        let source_location =
            fs::canonicalize(artifact_root).unwrap_or_else(|_| artifact_root.to_path_buf());
        let runtime_location = fs::canonicalize(runtime_artifact_root)
            .unwrap_or_else(|_| runtime_artifact_root.to_path_buf());
        if runtime_location == source_location || runtime_location.starts_with(&source_location) {
            return Err(SkiffTestError::MissingIsolatedRuntimeRoot);
        }
        runtime_artifact_root
    };
    let (Some(activation_url), Some(ingress_url)) = (
        options.activation_url.as_deref(),
        options.ingress_url.as_deref(),
    ) else {
        return Err(SkiffTestError::MissingCanonicalRuntime);
    };
    validate_activation_url(activation_url)
        .map_err(canonical_fixture::CanonicalFixtureError::InvalidInput)?;
    validate_ingress_url(ingress_url)
        .map_err(canonical_fixture::CanonicalFixtureError::InvalidInput)?;
    Ok(canonical_fixture::run_package_cases(
        &package_root,
        project,
        cases,
        artifact_root,
        runtime_artifact_root,
        activation_url,
        options,
    )?)
}
