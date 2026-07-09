use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_compiler_core::artifact::{
    ConfigAndEffectMetadata, PackageDependencyConstraint, PackageUnit,
};
use skiff_compiler_emission::package_test_artifacts::{
    build_package_test_artifacts, PackageTestArtifactBuildError, PackageTestArtifactBuildInput,
    PackageTestDependencyPackageInput, PackageTestEntrypointInput as EmissionEntrypointInput,
    PackageTestFileIrArtifact as EmissionFileIrArtifact,
};
use thiserror::Error;

use skiff_compiler_core::id::PublicationId;
use skiff_compiler_emission::artifact::PublishedFileIrArtifact;

#[derive(Debug, Clone)]
pub struct TestPackageTestArtifactInput {
    pub artifact_root: PathBuf,
    pub package_id: String,
    pub package_version: String,
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    pub production_package_unit: Option<PackageUnit>,
    pub production_config_and_effect_metadata: ConfigAndEffectMetadata,
    pub package_test_config_and_effect_metadata: ConfigAndEffectMetadata,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub dependency_packages: Vec<TestPackageTestDependencyPackageInput>,
    pub test_files: Vec<TestPackageTestFileIrArtifact>,
    pub entrypoints: Vec<TestPackageTestEntrypointInput>,
}

#[derive(Debug, Clone)]
pub struct TestPackageTestDependencyPackageInput {
    pub package_id: String,
    pub package_version: String,
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub package_unit: Option<PackageUnit>,
}

#[derive(Debug, Clone)]
pub struct TestPackageTestFileIrArtifact {
    pub source_path: String,
    pub module_path: String,
    pub file_ir: skiff_compiler_core::artifact::FileIrUnit,
    pub explicit_const_type_annotations: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct TestPackageTestEntrypointInput {
    pub display_name: String,
    pub source_path: String,
    pub module_path: String,
    pub test_ordinal: u32,
    pub executable_index: u32,
    pub executable_local_id: String,
    pub symbol: Option<String>,
    pub default_run: bool,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPackageTestArtifactOutput {
    pub artifact_root: PathBuf,
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
    pub test_build_hash: String,
    pub package_unit_path: String,
    pub assembly_path: String,
    pub dev_pointer_path: String,
    pub runtime_visible_paths: Vec<String>,
    pub entrypoints: Vec<TestPackageTestEntrypointSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestPackageTestPointer {
    pub schema_version: String,
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
    pub package_test_assembly: TestPackageTestAssemblyPointer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestPackageTestAssemblyPointer {
    pub assembly_identity: String,
    pub assembly_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPackageTestEntrypointSummary {
    pub display_name: String,
    pub entrypoint_local_id: String,
    pub entrypoint_id: String,
}

#[derive(Debug, Error)]
pub enum TestPackageTestArtifactError {
    #[error("invalid package test artifact input: {message}")]
    InvalidInput { message: String },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize {path}: {source}")]
    SerializeJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to compute package test identity: {0}")]
    Identity(#[from] skiff_compiler_emission::identity::ArtifactIdentityError),
    #[error("failed to register runtime-visible path {path}: {message}")]
    RuntimeVisiblePathRegistration { path: String, message: String },
}

impl From<PackageTestArtifactBuildError> for TestPackageTestArtifactError {
    fn from(error: PackageTestArtifactBuildError) -> Self {
        match error {
            PackageTestArtifactBuildError::InvalidInput { message } => {
                Self::InvalidInput { message }
            }
            PackageTestArtifactBuildError::Identity(source) => Self::Identity(source),
        }
    }
}

pub fn write_package_test_artifact_root(
    input: TestPackageTestArtifactInput,
) -> Result<TestPackageTestArtifactOutput, TestPackageTestArtifactError> {
    write_package_test_artifact_root_with_runtime_path_registration(input, |_| Ok(()))
}

pub fn write_package_test_artifact_root_with_runtime_path_registration(
    input: TestPackageTestArtifactInput,
    mut register_runtime_path: impl FnMut(&str) -> Result<(), String>,
) -> Result<TestPackageTestArtifactOutput, TestPackageTestArtifactError> {
    let artifact_root = input.artifact_root;
    let built = build_package_test_artifacts(PackageTestArtifactBuildInput {
        package_id: input.package_id,
        package_version: input.package_version,
        package_dependencies: input.package_dependencies,
        production_package_unit: input.production_package_unit,
        production_config_and_effect_metadata: input.production_config_and_effect_metadata,
        package_test_config_and_effect_metadata: input.package_test_config_and_effect_metadata,
        production_files: input.production_files,
        dependency_packages: input
            .dependency_packages
            .into_iter()
            .map(|dependency| PackageTestDependencyPackageInput {
                package_id: dependency.package_id,
                package_version: dependency.package_version,
                package_dependencies: dependency.package_dependencies,
                production_files: dependency.production_files,
                package_unit: dependency.package_unit,
            })
            .collect(),
        test_files: input
            .test_files
            .into_iter()
            .map(|file| EmissionFileIrArtifact {
                source_path: file.source_path,
                module_path: file.module_path,
                file_ir: file.file_ir,
                explicit_const_type_annotations: file.explicit_const_type_annotations,
            })
            .collect(),
        entrypoints: input
            .entrypoints
            .into_iter()
            .map(|entrypoint| EmissionEntrypointInput {
                display_name: entrypoint.display_name,
                source_path: entrypoint.source_path,
                module_path: entrypoint.module_path,
                test_ordinal: entrypoint.test_ordinal,
                executable_index: entrypoint.executable_index,
                executable_local_id: entrypoint.executable_local_id,
                symbol: entrypoint.symbol,
                default_run: entrypoint.default_run,
                config_and_effect_metadata: entrypoint.config_and_effect_metadata,
            })
            .collect(),
    })?;
    let assembly_path = built.assembly.path.clone();
    let dev_pointer_path = format!(
        "dev/package-tests/{}/{}.json",
        built.package_artifact_path, built.test_build_hash
    );
    let runtime_visible_paths = vec![dev_pointer_path.clone(), assembly_path.clone()];
    for path in &runtime_visible_paths {
        register_runtime_path(path).map_err(|message| {
            TestPackageTestArtifactError::RuntimeVisiblePathRegistration {
                path: path.clone(),
                message,
            }
        })?;
    }
    let pointer = TestPackageTestPointer {
        schema_version: "skiff-package-test-dev-pointer-v1".to_string(),
        package_id: built.package_id.clone(),
        package_version: built.package_version.clone(),
        test_build_identity: built.test_build_identity.clone(),
        package_test_assembly: TestPackageTestAssemblyPointer {
            assembly_identity: built.test_build_identity.clone(),
            assembly_path: assembly_path.clone(),
        },
    };

    for file in &built.production_files {
        write_json(&artifact_root, &file.path, &file.value())?;
    }
    for dependency in &built.dependency_package_units {
        for file in &dependency.files {
            write_json(&artifact_root, &file.path, &file.value())?;
        }
        write_json(&artifact_root, &dependency.unit_path, &dependency.value)?;
    }
    for file in &built.test_files {
        write_json(&artifact_root, &file.path, &file.value())?;
    }
    write_json(
        &artifact_root,
        &built.production_package_unit.unit_path,
        &built.production_package_unit.value,
    )?;
    write_json(&artifact_root, &assembly_path, &built.assembly.value)?;
    write_json(
        &artifact_root,
        &dev_pointer_path,
        &serde_json::to_value(&pointer).expect("package test dev pointer must serialize"),
    )?;

    Ok(TestPackageTestArtifactOutput {
        artifact_root,
        package_id: built.package_id,
        package_version: built.package_version,
        test_build_identity: built.test_build_identity,
        test_build_hash: built.test_build_hash,
        package_unit_path: built.production_package_unit.unit_path,
        assembly_path,
        dev_pointer_path,
        runtime_visible_paths,
        entrypoints: built
            .entrypoints
            .into_iter()
            .map(|entrypoint| TestPackageTestEntrypointSummary {
                display_name: entrypoint.display_name,
                entrypoint_local_id: entrypoint.entrypoint_local_id,
                entrypoint_id: entrypoint.entrypoint_id,
            })
            .collect(),
    })
}

pub fn list_package_test_assemblies(
    artifact_root: &Path,
    package_id: &str,
) -> Result<Vec<TestPackageTestPointer>, TestPackageTestArtifactError> {
    let package_path = PublicationId::parse(package_id)
        .map_err(|error| TestPackageTestArtifactError::InvalidInput {
            message: format!("package id {package_id} is invalid: {error}"),
        })?
        .artifact_path();
    let dir = artifact_root
        .join("dev")
        .join("package-tests")
        .join(package_path);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut pointers: Vec<TestPackageTestPointer> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|source| TestPackageTestArtifactError::Read {
        path: dir.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| TestPackageTestArtifactError::Read {
            path: dir.display().to_string(),
            source,
        })?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(entry.path()).map_err(|source| {
            TestPackageTestArtifactError::Read {
                path: entry.path().display().to_string(),
                source,
            }
        })?;
        pointers.push(serde_json::from_str(&text).map_err(|source| {
            TestPackageTestArtifactError::ParseJson {
                path: entry.path().display().to_string(),
                source,
            }
        })?);
    }
    pointers.sort_by(|left, right| left.test_build_identity.cmp(&right.test_build_identity));
    Ok(pointers)
}

fn write_json(
    root: &Path,
    relative_path: &str,
    value: &Value,
) -> Result<(), TestPackageTestArtifactError> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| TestPackageTestArtifactError::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|source| {
        TestPackageTestArtifactError::SerializeJson {
            path: path.display().to_string(),
            source,
        }
    })?;
    let text = format!("{text}\n");
    match fs::read(&path) {
        Ok(existing) if existing == text.as_bytes() => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => {}
    }
    fs::write(&path, text.as_bytes()).map_err(|source| TestPackageTestArtifactError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use serde_json::Value;

    use super::*;
    use crate::test_support::{
        compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test,
        compile_package_ast_config_and_effect_metadata_with_dependency_publications_for_test,
        compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test,
        compile_package_ast_file_ir_artifacts_with_dependency_publications_unit_and_metadata_for_test,
        compile_package_dependency_publications_for_test,
        compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_and_metadata_for_test,
        compile_parsed_only_package_ast_file_ir_artifacts_with_dependency_publications_and_metadata_for_test,
        compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test,
        compile_source_file_ir_artifact_for_test, TestCompilerSourceFile, TestPackageApiEntry,
        TestPackageManifest, TestResolvedPackage,
    };
    use crate::PackageDependency;

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn metadata_aware_package_compile_returns_full_config_metadata() {
        let sources = vec![
            parsed_test_source(
                "api.skiff",
                "api",
                false,
                r#"
                    function publicAnswer() -> number {
                        return 42
                    }
                "#,
            ),
            parsed_test_source(
                "api.test.skiff",
                "api.__test",
                true,
                r#"
                    function packageTestEntry() -> string {
                        const secret = config.require<string>("app.secret")
                        const timeout = config.optional<number>("app.timeout")
                        const enabled = config.has("app.enabled")
                        return secret
                    }
                "#,
            ),
        ];

        let compiled = compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
            "example.com/math",
            &[],
            Path::new("/tmp/skiff-package-test-metadata"),
            &sources,
            &BTreeMap::new(),
        )
        .expect("package test compile should return metadata");

        let metadata = serde_json::to_value(&compiled.config_and_effect_metadata)
            .expect("metadata should serialize");
        let config = metadata["config"]
            .as_object()
            .expect("metadata config should be an object");
        for key in ["shape", "uses", "activation", "requirements"] {
            assert!(
                config.contains_key(key),
                "metadata config must contain {key}: {metadata}"
            );
        }
        assert_config_shape_path(&metadata, "app.secret", true);
        assert_config_shape_path(&metadata, "app.timeout", false);
        assert_config_use_path(&metadata, "app.secret", true);
        assert_config_use_path(&metadata, "app.timeout", false);
        assert_config_activation_has_path(&metadata, "app.enabled");
        assert_config_requirement(&metadata, "app.enabled", "has");
        assert_config_requirement(&metadata, "app.secret", "require");
        assert_config_requirement(&metadata, "app.timeout", "optional");
    }

    #[test]
    fn package_test_metadata_only_matches_single_case_full_compile() {
        let root = Path::new("/tmp/skiff-package-test-metadata-only-equality");
        let manifest = metadata_manifest("example.com/meta", Vec::new());
        let available = manifest_map([manifest.clone()]);
        let package_aliases = BTreeMap::new();
        let case_a = metadata_case_sources("test.caseA");
        let case_b = metadata_case_sources("test.caseB");

        let full_case_a = compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
            &manifest.id,
            &manifest.dependencies,
            root,
            &case_a,
            &package_aliases,
        )
        .expect("old full compile metadata should compile")
        .config_and_effect_metadata;
        let metadata_only_case_a =
            compile_package_ast_config_and_effect_metadata_with_dependency_publications_for_test(
                &manifest,
                root,
                &case_a,
                &package_aliases,
                &[],
                &available,
            )
            .expect("metadata-only compile should compile");
        let full_case_b = compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
            &manifest.id,
            &manifest.dependencies,
            root,
            &case_b,
            &package_aliases,
        )
        .expect("old full compile metadata should compile")
        .config_and_effect_metadata;
        let metadata_only_case_b =
            compile_package_ast_config_and_effect_metadata_with_dependency_publications_for_test(
                &manifest,
                root,
                &case_b,
                &package_aliases,
                &[],
                &available,
            )
            .expect("metadata-only compile should compile");

        let full_a = metadata_json(&full_case_a);
        let only_a = metadata_json(&metadata_only_case_a);
        let full_b = metadata_json(&full_case_b);
        let only_b = metadata_json(&metadata_only_case_b);
        assert_eq!(only_a, full_a);
        assert_eq!(only_b, full_b);
        assert!(metadata_only_case_a.effects.is_empty());
        assert!(metadata_only_case_b.effects.is_empty());
        assert_config_shape_path(&only_a, "prod.secret", true);
        assert_config_shape_path(&only_a, "test.sharedConst", true);
        assert_config_shape_path(&only_a, "test.sharedHelper", true);
        assert_config_use_path(&only_a, "test.caseA", false);
        assert_config_activation_has_path(&only_a, "test.sharedEnabled");
        assert_config_requirement(&only_a, "test.sharedEnabled", "has");
        assert!(!config_shape_has_path(&only_a, "test.caseB"));
        assert!(!config_shape_has_path(&only_b, "test.caseA"));
    }

    #[test]
    fn package_test_metadata_only_preserves_dependency_requirements() {
        let root = Path::new("/tmp/skiff-package-test-metadata-only-dependency");
        let temp = temp_artifact_root("metadata-dependency-package");
        let dependency_root = temp.path().join("dep");
        fs::create_dir_all(&dependency_root).expect("dependency package root");
        fs::write(
            dependency_root.join("package.yml"),
            "id: example.com/dep\nversion: 1.0.0\n",
        )
        .expect("dependency package manifest");
        fs::write(
            dependency_root.join("dep.skiff"),
            r#"
                function depSecret() -> string {
                    return config.require<string>("dep.secret")
                }
            "#,
        )
        .expect("dependency package source");
        let dependency = metadata_manifest_at(
            "example.com/dep",
            Vec::new(),
            dependency_root.join("package.yml"),
        );
        let mut dependency_ref = PackageDependency::id("example.com/dep");
        dependency_ref.alias = Some("dep".to_string());
        let manifest = metadata_manifest("example.com/meta", vec![dependency_ref]);
        let available = manifest_map([manifest.clone(), dependency.clone()]);
        let dependency_package = TestResolvedPackage {
            manifest: dependency.clone(),
            config: Value::Null,
        };
        let sources = vec![parsed_test_source(
            "main.skiff",
            "main",
            false,
            r#"
                function run() -> string {
                    return "ok"
                }
            "#,
        )];
        let package_aliases = BTreeMap::new();

        let full =
            compile_parsed_only_package_ast_file_ir_artifacts_with_dependency_publications_and_metadata_for_test(
                &manifest,
                root,
                &sources,
                &package_aliases,
                std::slice::from_ref(&dependency_package),
                &available,
            )
            .expect("old full compile metadata should compile")
            .config_and_effect_metadata;
        let metadata_only =
            compile_package_ast_config_and_effect_metadata_with_dependency_publications_for_test(
                &manifest,
                root,
                &sources,
                &package_aliases,
                &[dependency_package],
                &available,
            )
            .expect("metadata-only compile should compile");

        let full = metadata_json(&full);
        let metadata_only = metadata_json(&metadata_only);
        assert_eq!(metadata_only, full);
        assert_dependency_requirement(&metadata_only, "dep.secret", "example.com/dep", "dep");
    }

    #[test]
    fn package_test_metadata_reuses_compiled_dependency_publications() {
        let root = Path::new("/tmp/skiff-package-test-metadata-reuse-dependency");
        let temp = temp_artifact_root("metadata-reuse-dependency-package");
        let dependency_root = temp.path().join("dep");
        fs::create_dir_all(&dependency_root).expect("dependency package root");
        fs::write(
            dependency_root.join("package.yml"),
            "id: example.com/dep\nversion: 1.0.0\n",
        )
        .expect("dependency package manifest");
        fs::write(
            dependency_root.join("dep.skiff"),
            r#"
                function depSecret() -> string {
                    return config.require<string>("dep.secret")
                }
            "#,
        )
        .expect("dependency package source");
        let dependency = metadata_manifest_at(
            "example.com/dep",
            Vec::new(),
            dependency_root.join("package.yml"),
        );
        let mut dependency_ref = PackageDependency::id("example.com/dep");
        dependency_ref.alias = Some("dep".to_string());
        let manifest = metadata_manifest("example.com/meta", vec![dependency_ref]);
        let available = manifest_map([manifest.clone(), dependency.clone()]);
        let dependency_package = TestResolvedPackage {
            manifest: dependency,
            config: Value::Null,
        };
        let production_sources = vec![parsed_test_source(
            "main.skiff",
            "main",
            false,
            r#"
                function run() -> string {
                    return "ok"
                }
            "#,
        )];
        let case_sources = metadata_case_sources("test.caseA");
        let package_aliases = BTreeMap::new();

        let dependency_publications = compile_package_dependency_publications_for_test(
            &manifest,
            std::slice::from_ref(&dependency_package),
            &available,
        )
        .expect("dependency publications should compile once");
        let parsed_only =
            compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_and_metadata_for_test(
                &manifest,
                root,
                &case_sources,
                &package_aliases,
                &dependency_publications,
            )
            .expect("parsed-only compile should reuse dependency publications")
            .config_and_effect_metadata;
        let production =
            compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test(
                &manifest,
                root,
                &production_sources,
                &package_aliases,
                &dependency_publications,
            )
            .expect("production compile should reuse dependency publications");
        let metadata_only =
            compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test(
                &manifest,
                root,
                &case_sources,
                &package_aliases,
                &dependency_publications,
            )
            .expect("metadata-only compile should reuse dependency publications");

        let parsed_only = metadata_json(&parsed_only);
        let production = metadata_json(&production.config_and_effect_metadata);
        let metadata_only = metadata_json(&metadata_only);
        assert_eq!(metadata_only, parsed_only);
        assert_dependency_requirement(&production, "dep.secret", "example.com/dep", "dep");
        assert_dependency_requirement(&metadata_only, "dep.secret", "example.com/dep", "dep");
    }

    #[test]
    fn writer_separates_production_and_package_test_metadata_and_ignores_non_owner_test_files() {
        let artifact_root = temp_artifact_root("metadata-owner");
        let production_sources = vec![parsed_test_source(
            "api.skiff",
            "api",
            false,
            r#"
                function publicAnswer() -> string {
                    return config.require<string>("prod.secret")
                }
            "#,
        )];
        let owner_test_sources = vec![
            parsed_test_source(
                "api.skiff",
                "api",
                false,
                r#"
                    function publicAnswer() -> string {
                        return config.require<string>("prod.secret")
                    }
                "#,
            ),
            parsed_test_source(
                "api.test.skiff",
                "api.__test",
                true,
                r#"
                    function packageTestEntry() -> string {
                        return config.require<string>("test.owner")
                    }
                "#,
            ),
        ];
        let production_compiled =
            compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
                "example.com/math",
                &[],
                Path::new("/tmp/skiff-package-test-production"),
                &production_sources,
                &BTreeMap::new(),
            )
            .expect("production package should compile");
        let package_test_compiled =
            compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
                "example.com/math",
                &[],
                Path::new("/tmp/skiff-package-test-owner"),
                &owner_test_sources,
                &BTreeMap::new(),
            )
            .expect("package test package should compile");
        let owner_file = package_test_compiled
            .file_ir_artifacts
            .iter()
            .find(|file| file.source_path == "api.test.skiff")
            .expect("owner test file should be compiled")
            .clone();

        let write_with_other_test = |other_return: &str| {
            let other_file = compile_source_file_ir_artifact_for_test(
                format!(
                    r#"
                        function otherPackageTestEntry() -> number {{
                            return {other_return}
                        }}
                    "#
                )
                .as_str(),
                "other.test.skiff",
                "other.__test",
                "package-test",
            )
            .expect("other test file should compile");
            write_package_test_artifact_root(TestPackageTestArtifactInput {
                artifact_root: artifact_root.path_buf(),
                package_id: "example.com/math".to_string(),
                package_version: "1.0.0".to_string(),
                package_dependencies: Vec::new(),
                production_package_unit: None,
                production_config_and_effect_metadata: production_compiled
                    .config_and_effect_metadata
                    .clone(),
                package_test_config_and_effect_metadata: package_test_compiled
                    .config_and_effect_metadata
                    .clone(),
                production_files: production_compiled.file_ir_artifacts.clone(),
                dependency_packages: Vec::new(),
                test_files: vec![
                    TestPackageTestFileIrArtifact {
                        source_path: owner_file.source_path.clone(),
                        module_path: owner_file.module_path.clone(),
                        file_ir: owner_file.unit.clone(),
                        explicit_const_type_annotations: BTreeSet::new(),
                    },
                    TestPackageTestFileIrArtifact {
                        source_path: "other.test.skiff".to_string(),
                        module_path: "other.__test".to_string(),
                        file_ir: other_file.unit,
                        explicit_const_type_annotations: BTreeSet::new(),
                    },
                ],
                entrypoints: vec![TestPackageTestEntrypointInput {
                    display_name: "owner package entry runs".to_string(),
                    source_path: owner_file.source_path.clone(),
                    module_path: owner_file.module_path.clone(),
                    test_ordinal: 0,
                    executable_index: 0,
                    executable_local_id: "packageTestEntry".to_string(),
                    symbol: Some("api.__test.packageTestEntry".to_string()),
                    default_run: true,
                    config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                }],
            })
            .expect("package test artifacts should write")
        };

        let first = write_with_other_test("7");
        let second = write_with_other_test("8");
        assert_eq!(
            first.test_build_identity, second.test_build_identity,
            "non-owner test file changes must not affect the current assembly identity"
        );

        let assembly = read_json(artifact_root.path(), &first.assembly_path);
        assert_eq!(assembly["testFiles"].as_array().unwrap().len(), 1);
        assert_eq!(assembly["testFiles"][0]["sourcePath"], "api.test.skiff");
        assert_eq!(
            assembly["linkPolicy"]["testFileScopes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            !serde_json::to_string(&assembly)
                .unwrap()
                .contains("other.test.skiff"),
            "non-owner test files must not be serialized into the assembly"
        );
        assert_config_shape_path(&assembly["configAndEffectMetadata"], "test.owner", true);
        assert_config_shape_path(&assembly["configAndEffectMetadata"], "prod.secret", true);

        let production_unit = read_json(artifact_root.path(), &first.package_unit_path);
        assert_config_shape_path(
            &production_unit["configAndEffectMetadata"],
            "prod.secret",
            true,
        );
        assert!(!config_shape_has_path(
            &production_unit["configAndEffectMetadata"],
            "test.owner"
        ));
    }

    #[test]
    fn writer_includes_multiple_owner_test_files_referenced_by_entrypoints() {
        let artifact_root = temp_artifact_root("multi-owner-package-test");
        let production = compile_file(
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
            "api.skiff",
            "api",
            "package-production",
        );
        let first_test = compile_file(
            r#"
                function firstEntry() -> number {
                    return 1
                }
            "#,
            "first.test.skiff",
            "first.__test",
            "package-test",
        );
        let second_test = compile_file(
            r#"
                function secondEntry() -> number {
                    return 2
                }
            "#,
            "second.test.skiff",
            "second.__test",
            "package-test",
        );

        let written = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/math".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: Vec::new(),
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production],
            dependency_packages: Vec::new(),
            test_files: vec![
                test_file_artifact(&first_test),
                test_file_artifact(&second_test),
            ],
            entrypoints: vec![
                TestPackageTestEntrypointInput {
                    display_name: "first owner runs".to_string(),
                    source_path: "first.test.skiff".to_string(),
                    module_path: "first.__test".to_string(),
                    test_ordinal: 0,
                    executable_index: 0,
                    executable_local_id: "firstEntry".to_string(),
                    symbol: Some("first.__test.firstEntry".to_string()),
                    default_run: true,
                    config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                },
                TestPackageTestEntrypointInput {
                    display_name: "second owner runs".to_string(),
                    source_path: "second.test.skiff".to_string(),
                    module_path: "second.__test".to_string(),
                    test_ordinal: 0,
                    executable_index: 0,
                    executable_local_id: "secondEntry".to_string(),
                    symbol: Some("second.__test.secondEntry".to_string()),
                    default_run: true,
                    config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                },
            ],
        })
        .expect("package test artifacts should allow multiple owner files");

        let assembly = read_json(artifact_root.path(), &written.assembly_path);
        assert_eq!(
            assembly["testFiles"]
                .as_array()
                .expect("testFiles should be an array")
                .iter()
                .map(|file| file["sourcePath"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["first.test.skiff", "second.test.skiff"]
        );
        assert_eq!(
            assembly["testEntrypoints"]
                .as_array()
                .expect("testEntrypoints should be an array")
                .iter()
                .map(|entrypoint| (
                    entrypoint["displayName"].as_str().unwrap(),
                    entrypoint["ownerTestFile"]["sourcePath"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("first owner runs", "first.test.skiff"),
                ("second owner runs", "second.test.skiff"),
            ]
        );
        assert_eq!(
            assembly["linkPolicy"]["testFileScopes"]
                .as_array()
                .expect("testFileScopes should be an array")
                .len(),
            2
        );
        assert_eq!(written.entrypoints.len(), 2);
    }

    #[test]
    fn writer_skips_identical_existing_package_test_artifacts() {
        let artifact_root = temp_artifact_root("write-skip-identical");
        let production = compile_file(
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
            "api.skiff",
            "api",
            "package-production",
        );
        let test_file = compile_file(
            r#"
                function packageTestEntry() -> number {
                    return 42
                }
            "#,
            "api.test.skiff",
            "api.__test",
            "package-test",
        );
        let input = package_test_artifact_input(
            artifact_root.path_buf(),
            vec![production],
            vec![test_file_artifact(&test_file)],
            entrypoint_input(),
        );

        let first = write_package_test_artifact_root(input.clone())
            .expect("initial package test artifacts should write");
        let files = collect_regular_files(artifact_root.path());
        assert!(!files.is_empty(), "initial write should create artifacts");
        set_files_readonly(&files, true);

        let second = write_package_test_artifact_root(input)
            .expect("identical package test artifacts should be skipped");
        set_files_readonly(&files, false);

        assert_eq!(second, first);
    }

    #[test]
    fn writer_overwrites_existing_package_test_artifact_when_content_differs() {
        let artifact_root = temp_artifact_root("write-skip-different");
        let relative_path = "dev/package-tests/example~com~~math/current.json";
        write_json(
            artifact_root.path(),
            relative_path,
            &serde_json::json!({ "version": 1, "items": ["old"] }),
        )
        .expect("initial artifact should write");

        write_json(
            artifact_root.path(),
            relative_path,
            &serde_json::json!({ "version": 2, "items": ["new"] }),
        )
        .expect("different content should overwrite the existing artifact");

        assert_eq!(
            read_json(artifact_root.path(), relative_path),
            serde_json::json!({ "version": 2, "items": ["new"] })
        );
    }

    #[test]
    fn writer_writes_changed_package_test_artifact_content() {
        let artifact_root = temp_artifact_root("write-skip-changed");
        let test_file = compile_file(
            r#"
                function packageTestEntry() -> number {
                    return 42
                }
            "#,
            "api.test.skiff",
            "api.__test",
            "package-test",
        );

        let write_with_answer = |answer: &str| {
            let production = compile_file(
                &format!(
                    r#"
                        function publicAnswer() -> number {{
                            return {answer}
                        }}
                    "#
                ),
                "api.skiff",
                "api",
                "package-production",
            );
            write_package_test_artifact_root(package_test_artifact_input(
                artifact_root.path_buf(),
                vec![production],
                vec![test_file_artifact(&test_file)],
                entrypoint_input(),
            ))
            .expect("package test artifacts should write")
        };

        let first = write_with_answer("42");
        let files = collect_regular_files(artifact_root.path());
        set_files_readonly(&files, true);

        let second = write_with_answer("43");
        set_files_readonly(&files, false);

        assert_ne!(
            first.package_unit_path, second.package_unit_path,
            "changed production content must produce a new package unit path"
        );
        assert_ne!(
            first.test_build_identity, second.test_build_identity,
            "changed production content must affect package-test identity"
        );
        assert!(
            artifact_root
                .path()
                .join(&second.package_unit_path)
                .is_file(),
            "changed package unit artifact should be written"
        );
        assert!(
            artifact_root.path().join(&second.assembly_path).is_file(),
            "changed package-test assembly should be written"
        );
        assert!(
            artifact_root
                .path()
                .join(&second.dev_pointer_path)
                .is_file(),
            "changed package-test dev pointer should be written"
        );
    }

    #[test]
    fn writer_rejects_entrypoint_owner_module_and_executable_mismatches() {
        let artifact_root = temp_artifact_root("owner-mismatch");
        let production = compile_file(
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
            "api.skiff",
            "api",
            "package-production",
        );
        let test_file = compile_file(
            r#"
                function packageTestEntry() -> number {
                    return 42
                }
            "#,
            "api.test.skiff",
            "api.__test",
            "package-test",
        );

        let missing_source = package_test_input(
            artifact_root.path_buf(),
            vec![production.clone()],
            vec![test_file_artifact(&test_file)],
            TestPackageTestEntrypointInput {
                display_name: "missing owner".to_string(),
                source_path: "missing.test.skiff".to_string(),
                module_path: "api.__test".to_string(),
                test_ordinal: 0,
                executable_index: 0,
                executable_local_id: "packageTestEntry".to_string(),
                symbol: Some("api.__test.packageTestEntry".to_string()),
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            },
        );
        assert_invalid_input_contains(missing_source, "does not match any test file");

        let module_mismatch = package_test_input(
            artifact_root.path_buf(),
            vec![production.clone()],
            vec![test_file_artifact(&test_file)],
            TestPackageTestEntrypointInput {
                display_name: "module mismatch".to_string(),
                source_path: "api.test.skiff".to_string(),
                module_path: "wrong.__test".to_string(),
                test_ordinal: 0,
                executable_index: 0,
                executable_local_id: "packageTestEntry".to_string(),
                symbol: Some("api.__test.packageTestEntry".to_string()),
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            },
        );
        assert_invalid_input_contains(
            module_mismatch,
            "does not match owner test file module_path",
        );

        let executable_mismatch = package_test_input(
            artifact_root.path_buf(),
            vec![production],
            vec![test_file_artifact(&test_file)],
            TestPackageTestEntrypointInput {
                display_name: "executable mismatch".to_string(),
                source_path: "api.test.skiff".to_string(),
                module_path: "api.__test".to_string(),
                test_ordinal: 0,
                executable_index: 0,
                executable_local_id: "wrongEntry".to_string(),
                symbol: Some("api.__test.packageTestEntry".to_string()),
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            },
        );
        assert_invalid_input_contains(executable_mismatch, "executable_local_id");
    }

    #[test]
    fn writer_sorts_dedupes_and_validates_dependency_slot_records() {
        let artifact_root = temp_artifact_root("dependency-slots");
        let production = compile_file(
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
            "api.skiff",
            "api",
            "package-production",
        );
        let test_file = compile_file(
            r#"
                function packageTestEntry() -> number {
                    return 42
                }
            "#,
            "api.test.skiff",
            "api.__test",
            "package-test",
        );

        let written = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/main".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: Vec::new(),
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production.clone()],
            dependency_packages: vec![
                dependency_package_input("example.com/zeta", "1.0.0", "7"),
                dependency_package_input("example.com/alpha", "1.0.0", "1"),
                dependency_package_input("example.com/alpha", "1.0.0", "1"),
            ],
            test_files: vec![test_file_artifact(&test_file)],
            entrypoints: vec![entrypoint_input()],
        })
        .expect("duplicate exact dependency slot should be deduped");
        let assembly = read_json(artifact_root.path(), &written.assembly_path);
        let refs = assembly["dependencyPackageUnits"].as_array().unwrap();
        let scopes = assembly["linkPolicy"]["dependencyPublicScopes"]
            .as_array()
            .unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(scopes.len(), 2);
        assert_eq!(refs[0]["packageId"], "example.com/alpha");
        assert_eq!(scopes[0]["packageId"], refs[0]["packageId"]);
        assert_eq!(refs[1]["packageId"], "example.com/zeta");
        assert_eq!(scopes[1]["packageId"], refs[1]["packageId"]);

        let conflicting_build_identity =
            write_package_test_artifact_root(TestPackageTestArtifactInput {
                artifact_root: artifact_root.path_buf(),
                package_id: "example.com/main".to_string(),
                package_version: "1.0.0".to_string(),
                package_dependencies: Vec::new(),
                production_package_unit: None,
                production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                production_files: vec![production.clone()],
                dependency_packages: vec![
                    dependency_package_input("example.com/alpha", "1.0.0", "1"),
                    dependency_package_input("example.com/alpha", "1.0.0", "2"),
                ],
                test_files: vec![test_file_artifact(&test_file)],
                entrypoints: vec![entrypoint_input()],
            });
        assert_invalid_input_contains(conflicting_build_identity, "multiple package slots");
    }

    #[test]
    fn writer_rejects_duplicate_aliases_but_allows_multiple_aliases_to_same_slot() {
        let artifact_root = temp_artifact_root("aliases");
        let production = compile_file(
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
            "api.skiff",
            "api",
            "package-production",
        );
        let test_file = compile_file(
            r#"
                function packageTestEntry() -> number {
                    return 42
                }
            "#,
            "api.test.skiff",
            "api.__test",
            "package-test",
        );
        let dependency = dependency_package_input("example.com/dep", "1.0.0", "1");

        let duplicate_alias = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/main".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: vec![
                dependency_constraint("example.com/dep", "1.0.0", "dep"),
                dependency_constraint("example.com/other", "1.0.0", "dep"),
            ],
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production.clone()],
            dependency_packages: vec![dependency.clone()],
            test_files: vec![test_file_artifact(&test_file)],
            entrypoints: vec![entrypoint_input()],
        });
        assert_invalid_input_contains(duplicate_alias, "dependency alias dep");

        let multi_alias = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/main".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: vec![
                dependency_constraint("example.com/dep", "1.0.0", "left"),
                dependency_constraint("example.com/dep", "1.0.0", "right"),
            ],
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production.clone()],
            dependency_packages: vec![dependency.clone()],
            test_files: vec![test_file_artifact(&test_file)],
            entrypoints: vec![entrypoint_input()],
        })
        .expect("multiple aliases to the same exact dependency slot should be allowed");
        let assembly = read_json(artifact_root.path(), &multi_alias.assembly_path);
        assert_eq!(
            assembly["dependencyPackageUnits"].as_array().unwrap().len(),
            1
        );

        let left_alias = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/main".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: vec![dependency_constraint("example.com/dep", "1.0.0", "left")],
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production.clone()],
            dependency_packages: vec![dependency.clone()],
            test_files: vec![test_file_artifact(&test_file)],
            entrypoints: vec![entrypoint_input()],
        })
        .expect("left alias build should write");
        let right_alias = write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.path_buf(),
            package_id: "example.com/main".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: vec![dependency_constraint("example.com/dep", "1.0.0", "right")],
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files: vec![production],
            dependency_packages: vec![dependency],
            test_files: vec![test_file_artifact(&test_file)],
            entrypoints: vec![entrypoint_input()],
        })
        .expect("right alias build should write");
        assert_ne!(
            left_alias.package_unit_path, right_alias.package_unit_path,
            "alias rename must change production package unit identity/path"
        );
        assert_ne!(
            left_alias.test_build_identity, right_alias.test_build_identity,
            "alias rename must affect package test identity through production unit identity"
        );
    }

    #[test]
    fn production_package_compile_helper_preserves_manifest_api_seed() {
        let manifest = TestPackageManifest {
            id: "example.com/math".to_string(),
            version: "1.0.0".to_string(),
            api: vec![TestPackageApiEntry::source(
                "publicAnswer",
                "api",
                "publicAnswer",
            )],
            dependencies: Vec::new(),
            path: PathBuf::from("/tmp/skiff-package-publication-api-seed"),
            synthetic: false,
        };
        let sources = vec![parsed_api_source(
            "api.skiff",
            "api",
            r#"
                function publicAnswer() -> number {
                    return 42
                }
            "#,
        )];

        let compiled =
            compile_package_ast_file_ir_artifacts_with_dependency_publications_unit_and_metadata_for_test(
                &manifest,
                Path::new("/tmp/skiff-package-publication-api-seed"),
                &sources,
                &BTreeMap::new(),
                &[],
                &BTreeMap::new(),
            )
            .expect("production package publication helper should compile");
        let package_unit = compiled
            .package_unit
            .expect("production helper should publish package unit metadata");

        assert!(
            package_unit
                .publication_abi
                .operation_exports
                .iter()
                .any(|operation| operation.public_path == "publicAnswer"),
            "production publication compile must preserve manifest API seed: {:?}",
            package_unit.publication_abi.operation_exports
        );
    }

    fn parsed_test_source(
        path: &str,
        module_path: &str,
        is_test_file: bool,
        text: &str,
    ) -> (TestCompilerSourceFile, skiff_syntax::ast::SourceFile) {
        parsed_source(path, module_path, false, is_test_file, text)
    }

    fn parsed_api_source(
        path: &str,
        module_path: &str,
        text: &str,
    ) -> (TestCompilerSourceFile, skiff_syntax::ast::SourceFile) {
        parsed_source(path, module_path, true, false, text)
    }

    fn parsed_source(
        path: &str,
        module_path: &str,
        is_api: bool,
        is_test_file: bool,
        text: &str,
    ) -> (TestCompilerSourceFile, skiff_syntax::ast::SourceFile) {
        (
            TestCompilerSourceFile {
                relative_path: PathBuf::from(path),
                module_path: module_path.to_string(),
                is_api,
                is_test_file,
                text: text.to_string(),
            },
            skiff_syntax::parser::parse_source(text).expect("test source should parse"),
        )
    }

    fn metadata_case_sources(
        case_path: &str,
    ) -> Vec<(TestCompilerSourceFile, skiff_syntax::ast::SourceFile)> {
        vec![
            parsed_test_source(
                "main.skiff",
                "main",
                false,
                r#"
                    function readProdSecret() -> string {
                        return config.require<string>("prod.secret")
                    }
                "#,
            ),
            parsed_test_source(
                "main.test.skiff",
                "main.__test",
                true,
                &format!(
                    r#"
                        const sharedConst: string = config.require<string>("test.sharedConst")
                        const sharedEnabled: boolean = config.has("test.sharedEnabled")

                        function sharedHelper() -> string {{
                            return config.require<string>("test.sharedHelper")
                        }}

                        function packageTestEntry() -> string {{
                            const local = config.optional<string>("{case_path}")
                            return sharedHelper()
                        }}
                    "#
                ),
            ),
        ]
    }

    fn metadata_manifest(id: &str, dependencies: Vec<PackageDependency>) -> TestPackageManifest {
        metadata_manifest_at(
            id,
            dependencies,
            PathBuf::from(format!("/tmp/{}/package.yml", id.replace('/', "-"))),
        )
    }

    fn metadata_manifest_at(
        id: &str,
        dependencies: Vec<PackageDependency>,
        path: PathBuf,
    ) -> TestPackageManifest {
        TestPackageManifest {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            api: Vec::new(),
            dependencies,
            path,
            synthetic: false,
        }
    }

    fn manifest_map(
        manifests: impl IntoIterator<Item = TestPackageManifest>,
    ) -> BTreeMap<(String, String), TestPackageManifest> {
        manifests
            .into_iter()
            .map(|manifest| ((manifest.id.clone(), manifest.version.clone()), manifest))
            .collect()
    }

    fn metadata_json(metadata: &ConfigAndEffectMetadata) -> Value {
        serde_json::to_value(metadata).expect("metadata should serialize")
    }

    fn compile_file(
        source: &str,
        source_path: &str,
        module_path: &str,
        role: &str,
    ) -> PublishedFileIrArtifact {
        compile_source_file_ir_artifact_for_test(source, source_path, module_path, role)
            .expect("test File IR should compile")
    }

    fn test_file_artifact(file: &PublishedFileIrArtifact) -> TestPackageTestFileIrArtifact {
        TestPackageTestFileIrArtifact {
            source_path: file.source_path.clone(),
            module_path: file.module_path.clone(),
            file_ir: file.unit.clone(),
            explicit_const_type_annotations: BTreeSet::new(),
        }
    }

    fn test_file_artifact_with_explicit<const N: usize>(
        file: &PublishedFileIrArtifact,
        names: [&str; N],
    ) -> TestPackageTestFileIrArtifact {
        TestPackageTestFileIrArtifact {
            source_path: file.source_path.clone(),
            module_path: file.module_path.clone(),
            file_ir: file.unit.clone(),
            explicit_const_type_annotations: names
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
        }
    }

    fn package_test_input(
        artifact_root: PathBuf,
        production_files: Vec<PublishedFileIrArtifact>,
        test_files: Vec<TestPackageTestFileIrArtifact>,
        entrypoint: TestPackageTestEntrypointInput,
    ) -> Result<TestPackageTestArtifactOutput, TestPackageTestArtifactError> {
        write_package_test_artifact_root(package_test_artifact_input(
            artifact_root,
            production_files,
            test_files,
            entrypoint,
        ))
    }

    fn package_test_artifact_input(
        artifact_root: PathBuf,
        production_files: Vec<PublishedFileIrArtifact>,
        test_files: Vec<TestPackageTestFileIrArtifact>,
        entrypoint: TestPackageTestEntrypointInput,
    ) -> TestPackageTestArtifactInput {
        TestPackageTestArtifactInput {
            artifact_root,
            package_id: "example.com/math".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: Vec::new(),
            production_package_unit: None,
            production_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            package_test_config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            production_files,
            dependency_packages: Vec::new(),
            test_files,
            entrypoints: vec![entrypoint],
        }
    }

    fn entrypoint_input() -> TestPackageTestEntrypointInput {
        TestPackageTestEntrypointInput {
            display_name: "package entry runs".to_string(),
            source_path: "api.test.skiff".to_string(),
            module_path: "api.__test".to_string(),
            test_ordinal: 0,
            executable_index: 0,
            executable_local_id: "packageTestEntry".to_string(),
            symbol: Some("api.__test.packageTestEntry".to_string()),
            default_run: true,
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
        }
    }

    fn dependency_package_input(
        package_id: &str,
        version: &str,
        answer: &str,
    ) -> TestPackageTestDependencyPackageInput {
        let file = compile_file(
            &format!(
                r#"
                    function depAnswer() -> number {{
                        return {answer}
                    }}
                "#
            ),
            "dep.skiff",
            "dep",
            "package-production",
        );
        TestPackageTestDependencyPackageInput {
            package_id: package_id.to_string(),
            package_version: version.to_string(),
            package_dependencies: Vec::new(),
            production_files: vec![file],
            package_unit: None,
        }
    }

    fn dependency_package_input_with_unit(
        package_id: &str,
        version: &str,
        file: PublishedFileIrArtifact,
        unit: PackageUnit,
    ) -> TestPackageTestDependencyPackageInput {
        TestPackageTestDependencyPackageInput {
            package_id: package_id.to_string(),
            package_version: version.to_string(),
            package_dependencies: Vec::new(),
            production_files: vec![file],
            package_unit: Some(unit),
        }
    }

    fn dependency_constraint(id: &str, version: &str, alias: &str) -> PackageDependencyConstraint {
        PackageDependencyConstraint {
            id: id.to_string(),
            version: version.to_string(),
            alias: alias.to_string(),
            config: Value::Null,
        }
    }

    struct TempArtifactRoot {
        path: PathBuf,
    }

    impl TempArtifactRoot {
        fn path(&self) -> &Path {
            &self.path
        }

        fn path_buf(&self) -> PathBuf {
            self.path.clone()
        }
    }

    impl Drop for TempArtifactRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn temp_artifact_root(name: &str) -> TempArtifactRoot {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "skiff-package-test-artifacts-{name}-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("old temp artifact root should be removed");
        }
        fs::create_dir_all(&root).expect("temp artifact root should be created");
        TempArtifactRoot { path: root }
    }

    fn read_json(root: &Path, relative_path: &str) -> Value {
        serde_json::from_str(
            &fs::read_to_string(root.join(relative_path)).expect("artifact should be readable"),
        )
        .expect("artifact should be JSON")
    }

    fn collect_regular_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_regular_files_into(root, &mut files);
        files.sort();
        files
    }

    fn collect_regular_files_into(path: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("artifact directory should be readable") {
            let entry = entry.expect("artifact directory entry should be readable");
            let file_type = entry
                .file_type()
                .expect("artifact directory entry type should be readable");
            if file_type.is_dir() {
                collect_regular_files_into(&entry.path(), files);
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }

    fn set_files_readonly(files: &[PathBuf], readonly: bool) {
        for file in files {
            let mut permissions = fs::metadata(file)
                .unwrap_or_else(|error| {
                    panic!(
                        "artifact file {} metadata should be readable: {error}",
                        file.display()
                    )
                })
                .permissions();
            permissions.set_readonly(readonly);
            fs::set_permissions(file, permissions).unwrap_or_else(|error| {
                panic!(
                    "artifact file {} permissions should be writable: {error}",
                    file.display()
                )
            });
        }
    }

    fn assert_invalid_input_contains<T: std::fmt::Debug>(
        result: Result<T, TestPackageTestArtifactError>,
        expected: &str,
    ) {
        let error = match result {
            Ok(_) => panic!("input should be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains(expected),
            "expected {expected:?} in error:\n{error}"
        );
    }

    fn assert_config_shape_path(metadata: &Value, path: &str, required: bool) {
        let entry = metadata["config"]["shape"]["entries"]
            .as_array()
            .expect("shape entries should be an array")
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("expected shape path {path} in {metadata}"));
        assert_eq!(entry["required"], required);
    }

    fn config_shape_has_path(metadata: &Value, path: &str) -> bool {
        metadata["config"]["shape"]["entries"]
            .as_array()
            .expect("shape entries should be an array")
            .iter()
            .any(|entry| entry["path"] == path)
    }

    fn assert_config_use_path(metadata: &Value, path: &str, required: bool) {
        let entry = metadata["config"]["uses"]
            .as_array()
            .expect("uses should be an array")
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("expected use path {path} in {metadata}"));
        assert_eq!(entry["required"], required);
    }

    fn assert_config_activation_has_path(metadata: &Value, path: &str) {
        let has_paths = metadata["config"]["activation"]["hasPaths"]
            .as_array()
            .expect("activation hasPaths should be an array");
        assert!(
            has_paths.iter().any(|entry| entry == path),
            "expected activation has path {path} in {metadata}"
        );
    }

    fn assert_config_requirement(metadata: &Value, path: &str, kind: &str) {
        let requirements = metadata["config"]["requirements"]["effective"]
            .as_array()
            .expect("effective requirements should be an array");
        assert!(
            requirements
                .iter()
                .any(|entry| entry["path"] == path && entry["access"]["kind"] == kind),
            "expected requirement {kind}:{path} in {metadata}"
        );
    }

    fn assert_dependency_requirement(metadata: &Value, path: &str, package_id: &str, alias: &str) {
        let requirements = metadata["config"]["requirements"]["dependency"]
            .as_array()
            .expect("dependency requirements should be an array");
        let requirement = requirements
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("expected dependency requirement {path} in {metadata}"));
        assert_eq!(
            requirement["provenance"][0]["declaringPublication"]["id"],
            package_id
        );
        assert_eq!(
            requirement["provenance"][0]["dependencyPath"][0]["alias"],
            alias
        );
        assert_config_requirement(metadata, path, "require");
    }
}
