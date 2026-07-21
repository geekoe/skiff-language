//! Canonical package/service test infrastructure.
//!
//! Production package source is compiled once into an immutable `PackageArtifact`.
//! Test overlays are separate package builds and canonical service tests are assembled
//! from code-free contracts, source-free deployments, and a `RuntimeAssembly`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

pub mod canonical_fixture;
pub mod canonical_package;
pub mod ecosystem_smoke_fixture;
pub mod test_overlay;

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
    pub allow_network: bool,
    pub config_path: Option<PathBuf>,
    pub package_dirs: Vec<PathBuf>,
    pub artifact_root: Option<PathBuf>,
    pub activation_url: Option<String>,
    pub ingress_url: Option<String>,
    pub environment: String,
    pub expected_generation: u64,
    pub package_test_concurrency: Option<usize>,
}

impl Default for SkiffTestOptions {
    fn default() -> Self {
        Self {
            live: false,
            allow_network: false,
            config_path: None,
            package_dirs: Vec::new(),
            artifact_root: None,
            activation_url: None,
            ingress_url: None,
            environment: "skiff-test".to_string(),
            expected_generation: 0,
            package_test_concurrency: None,
        }
    }
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
    Fixture(#[from] canonical_fixture::CanonicalFixtureError),
    #[error("input {path} is not inside a package source root")]
    MissingPackageRoot { path: String },
    #[error("canonical execution requires --activation-url, --ingress-url and --artifact-root")]
    MissingCanonicalRuntime,
    #[error("live tests require an explicit file, --allow-network, and --config")]
    InvalidLiveOptions,
}

pub fn validate_activation_url(value: &str) -> Result<(), String> {
    let Some(rest) = value.strip_prefix("http://") else {
        return Err("activation URL must use http://".to_string());
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return Err("activation URL must include /__skiff/activate-assembly".to_string());
    };
    if authority.trim().is_empty() || path != "__skiff/activate-assembly" {
        return Err("activation URL must point exactly to /__skiff/activate-assembly".to_string());
    }
    Ok(())
}

pub fn run_skiff_tests(
    input: &Path,
    profile: Option<&str>,
) -> Result<SkiffTestSummary, SkiffTestError> {
    run_skiff_tests_with_options(input, profile, &SkiffTestOptions::default())
}

pub fn run_skiff_tests_with_options(
    input: &Path,
    _profile: Option<&str>,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    let metadata = fs::metadata(input).map_err(|source| SkiffTestError::Metadata {
        path: input.display().to_string(),
        source,
    })?;
    if options.live
        && (!metadata.is_file() || !options.allow_network || options.config_path.is_none())
    {
        return Err(SkiffTestError::InvalidLiveOptions);
    }
    let package_root =
        canonical_package::find_package_root(input, metadata.is_file()).ok_or_else(|| {
            SkiffTestError::MissingPackageRoot {
                path: input.display().to_string(),
            }
        })?;
    let project = canonical_package::compile_package_project(
        &package_root,
        &options.package_dirs,
        &Default::default(),
    )?;
    let cases =
        canonical_fixture::discover_package_test_cases(input, &package_root, metadata.is_file())?;
    if cases.is_empty() {
        return Ok(SkiffTestSummary {
            passed: 0,
            skipped: 0,
            failed: 0,
            results: Vec::new(),
        });
    }

    // Execution is deliberately all-or-nothing: a source compile is not reported as a passed
    // runtime test. The isolated runtime owner supplies both inputs after publishing the fixture.
    let (Some(artifact_root), Some(activation_url), Some(_ingress_url)) = (
        options.artifact_root.as_deref(),
        options.activation_url.as_deref(),
        options.ingress_url.as_deref(),
    ) else {
        return Err(SkiffTestError::MissingCanonicalRuntime);
    };
    validate_activation_url(activation_url).map_err(|message| {
        SkiffTestError::Fixture(canonical_fixture::CanonicalFixtureError::InvalidInput(
            message,
        ))
    })?;
    Ok(canonical_fixture::run_package_cases(
        &package_root,
        project,
        cases,
        artifact_root,
        activation_url,
        options,
    )?)
}
