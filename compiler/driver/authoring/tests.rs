use std::{cell::Cell, fs, path::PathBuf};

use serde_json::json;
use skiff_artifact_identity::package_schema_index_identity;
use skiff_artifact_model::{PackageArtifact, PackageLocalAbiIdentity, PackageRequirement};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::prelude_registry::{
    prelude_registry, PreludeRegistryInitializationError,
};

use super::{
    build_authoring_object, resolve_reachable_package_closure, run_after_platform_context_guard,
    AuthoringObject,
};

#[test]
fn p5_f18b_authoring_mismatch_zero_source_reads() {
    let platform_sources = repository_platform_sources();
    run_after_platform_context_guard(&platform_sources, || Ok(())).unwrap();

    let same_root_reads = Cell::new(0);
    run_after_platform_context_guard(&platform_sources, || {
        let _ = prelude_registry();
        same_root_reads.set(same_root_reads.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(same_root_reads.get(), 1);

    let different_root = MinimalPlatformFixture::new("authoring-mismatch");
    let mismatch_reads = Cell::new(0);
    let error = run_after_platform_context_guard(&different_root.context(), || {
        mismatch_reads.set(mismatch_reads.get() + 1);
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PreludeRegistryInitializationError>(),
        Some(PreludeRegistryInitializationError::DifferentPlatformRoot { .. })
    ));
    assert_eq!(mismatch_reads.get(), 0);

    let hostile_store = different_root.root.join("hostile-authoring-store");
    let authoring_error = build_authoring_object(
        &different_root.context(),
        AuthoringObject::Package,
        &different_root.root.join("missing-package"),
        &hostile_store,
        "dev",
        false,
    )
    .unwrap_err();
    assert!(matches!(
        authoring_error.downcast_ref::<PreludeRegistryInitializationError>(),
        Some(PreludeRegistryInitializationError::DifferentPlatformRoot { .. })
    ));
    assert!(!hostile_store.exists());
}

#[test]
fn p5_f149_reachable_package_closure_is_transitive_and_excludes_unused_candidates() {
    let leaf = package("example.com/leaf", "1.0.0", "leaf-abi", []);
    let middle = package(
        "example.com/middle",
        "1.0.0",
        "middle-abi",
        [requirement("leaf", &leaf)],
    );
    let implementation = package(
        "example.com/service",
        "1.0.0",
        "service-abi",
        [requirement("middle", &middle)],
    );
    let unused = package("example.com/unused", "1.0.0", "unused-abi", []);
    let mut resolutions = Vec::new();

    let closure = resolve_reachable_package_closure(
        &implementation,
        &[middle.clone(), unused],
        |id, version| {
            resolutions.push((id.to_string(), version.to_string()));
            if id == leaf.package_id && version == leaf.package_version {
                Ok(leaf.clone())
            } else {
                Err(test_error("unexpected package resolution"))
            }
        },
    )
    .unwrap();

    assert_eq!(
        closure
            .iter()
            .map(|artifact| artifact.package_id.as_str())
            .collect::<Vec<_>>(),
        ["example.com/leaf", "example.com/middle"]
    );
    assert_eq!(
        resolutions,
        [("example.com/leaf".to_string(), "1.0.0".to_string())]
    );
}

#[test]
fn p5_f149_reachable_package_closure_fails_closed_on_each_exact_edge() {
    let expected = package("example.com/provider", "1.0.0", "expected-abi", []);
    let implementation = package(
        "example.com/service",
        "1.0.0",
        "service-abi",
        [requirement("provider", &expected)],
    );
    let wrong_abi = package("example.com/provider", "1.0.0", "wrong-abi", []);
    let error = resolve_reachable_package_closure(&implementation, &[wrong_abi], |_, _| {
        panic!("a loaded coordinate with the wrong ABI must fail before store resolution")
    })
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("expected local ABI expected-abi"));

    let wrong_coordinate = package("example.com/other", "1.0.0", "expected-abi", []);
    let error =
        resolve_reachable_package_closure(
            &implementation,
            &[],
            |_, _| Ok(wrong_coordinate.clone()),
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("resolved to example.com/other@1.0.0"));

    let error = resolve_reachable_package_closure(&implementation, &[], |id, version| {
        Err(test_error(format!(
            "missing pointer/record for {id}@{version}"
        )))
    })
    .unwrap_err();
    assert!(error.to_string().contains("missing pointer/record"));
}

fn package(
    id: &str,
    version: &str,
    local_abi: &str,
    requirements: impl IntoIterator<Item = PackageRequirement>,
) -> PackageArtifact {
    let package_schema_index_identity =
        package_schema_index_identity(id, &Default::default()).unwrap();
    serde_json::from_value(json!({
        "schemaVersion": "skiff-package-artifact-v2",
        "packageId": id,
        "packageVersion": version,
        "packageBuildId": format!("build:{id}:{version}:{local_abi}"),
        "files": [],
        "staticResources": [],
        "packageLocalAbi": {
            "localAbiIdentity": local_abi,
            "publicSymbols": {}
        },
        "packageSchemaIndex": {
            "packageId": id,
            "packageSchemaIndexIdentity": package_schema_index_identity
        },
        "packageSchemaTypeRecords": {},
        "implementationLinks": {},
        "callableLinks": {},
        "packageRequirements": requirements.into_iter().collect::<Vec<_>>(),
        "contractRequirements": [],
        "serviceRequirements": [],
        "runtimeRequirements": {
            "config": [],
            "state": [],
            "resources": [],
            "runtimeCapabilities": []
        },
        "callableSemanticFacts": {},
        "boundaryProjections": {},
        "serviceCallRoots": [],
        "serviceCallRefs": []
    }))
    .unwrap()
}

fn requirement(alias: &str, package: &PackageArtifact) -> PackageRequirement {
    PackageRequirement {
        alias: alias.to_string(),
        package_id: package.package_id.clone(),
        exact_version: package.package_version.clone(),
        expected_local_abi: PackageLocalAbiIdentity::new(
            package.package_local_abi.local_abi_identity.as_ref(),
        ),
        expected_package_build: None,
    }
}

fn test_error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()).into()
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    CompilerPlatformSources::new(&root).unwrap()
}

pub(super) struct MinimalPlatformFixture {
    root: PathBuf,
}

impl MinimalPlatformFixture {
    pub(super) fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skiff-p5-f18b-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("std")).unwrap();
        fs::create_dir_all(root.join("prelude")).unwrap();
        fs::write(
            root.join("std/registry.yml"),
            "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/std\n    path: .\n",
        )
        .unwrap();
        fs::write(
            root.join("std/package.yml"),
            "id: skiff.run/std\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(root.join("prelude/error.skiff"), "").unwrap();
        Self { root }
    }

    pub(super) fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }
}

impl Drop for MinimalPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
