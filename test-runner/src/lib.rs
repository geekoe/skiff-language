// Minimal host-side runner for Skiff source tests.
//
// Test-only effect doubles live in optional `skiff.test-doubles.json` files under
// the tested root. The fixture is keyed by `module.path::test name` and stable target id:
//
// ```json
// {
//   "configs": {
//     "api.client::uses fake": { "app": { "mode": "test" } }
//   },
//   "tests": {
//     "api.client::uses fake": {
//       "std.http.client.request": {
//         "expectRequest": { "url": "https://example.test" },
//         "response": { "status": 200, "headers": [], "body": { "__skiffBytesBase64": "" } }
//       }
//     }
//   }
// }
// ```
//
// A double can also use `"sequence": [{ "expectRequest": ..., "response": ... }]`
// when a single test invokes the same stable target more than once.
//
// Runtime package tests support stable target ids such as `std.http.client.request`,
// `std.http.client.sse`, and `std.http.client.stream`. Doubles are copied into a fresh test
// interpreter for each test case, so registrations cannot leak between tests.

use std::{fs, path::Path};

mod artifacts;
mod doubles;
mod package;
mod root_paths;
mod runtime_process;
mod service;
mod service_publish;
mod sources;
mod types;
mod visibility;

use types::{
    PackageDependencyArtifacts, PackageTestCase, PackageTestSource, ParsedSource,
    PrivateVisibilityScope, ProductionModuleSymbols, ProductionSymbol, ProductionSymbolKind,
    ResolvedPublicationTestInputs, RuntimeTestArtifact, SymbolUseKind, TestCase, TestLocalSymbols,
};
pub use types::{SkiffTestError, SkiffTestOptions, SkiffTestResult, SkiffTestSummary};

pub fn run_skiff_tests(
    input: &Path,
    profile: Option<&str>,
) -> Result<SkiffTestSummary, SkiffTestError> {
    run_skiff_tests_with_options(input, profile, &SkiffTestOptions::default())
}

pub fn run_skiff_tests_with_options(
    input: &Path,
    profile: Option<&str>,
    options: &SkiffTestOptions,
) -> Result<SkiffTestSummary, SkiffTestError> {
    let metadata = fs::metadata(input).map_err(|source| SkiffTestError::Metadata {
        path: input.display().to_string(),
        source,
    })?;

    let input_is_file = metadata.is_file();
    if options.live {
        if !input_is_file {
            return Err(SkiffTestError::RuntimeSetup {
                message: "--live tests must explicitly specify a test file".to_string(),
            });
        }
        if !options.allow_network {
            return Err(SkiffTestError::RuntimeSetup {
                message: "--live tests require --allow-network".to_string(),
            });
        }
        if options.config_path.is_none() {
            return Err(SkiffTestError::RuntimeSetup {
                message: "--live tests require --config <path>".to_string(),
            });
        }
    }
    if let Some(package_root) = sources::find_package_root(input, input_is_file) {
        return package::run_package_tests(input, &package_root, input_is_file, options);
    }

    service::run_service_tests(input, profile, input_is_file, options)
}

#[cfg(test)]
mod tests;
