//! A1-compiler integration: the real compiler publish path must automatically
//! produce the canonical actor routing projection record
//! (`records/actor-routing/current.json`) that the A3 Rust strict reader and
//! the A2 TS loader consume.
//!
//! The test drives the same public authoring entrypoints as the CLI
//! (`build_authoring_object` for packages), publishes the real compiler-owned
//! std artifact, and verifies each produced record with the A3-equivalent
//! strict consumption chain:
//! canonical bytes equality + typed decode through the frozen
//! `deny_unknown_fields` surface.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, PackageArtifactRef,
    ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{
        author_official_std_package, build_authoring_object_legacy,
        publish_package_artifact_records, AuthoringObject,
    },
    CompilerPlatformSources,
};
use skiff_deployment::{
    projection::actor_routing::{ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_RECORD_PATH},
    storage::{CanonicalArtifactStore, PackageArtifactPointer},
};

type ActorMethodKey = (
    ActorAbiIdentity,
    ActorImplementationIdentity,
    ActorMethodIdentity,
);

const ALPHA_SOURCE: &str = r#"import std

type Counter {
  id: string,
  count: number,
}

actor Counter {
  key(id)
  create()
}

impl Counter {
  function create() -> void {
    self.count = 0
  }

  function increment() -> string {
    self.count = self.count + 1
    return "counter-ok"
  }
}

type CreateOnly {
  id: string,
}

actor CreateOnly {
  key(id)
  create()
}

impl CreateOnly {
  function create() -> void {
  }
}

function ping() -> string {
  let counter = std.actor.get<Counter>("shared")
  return counter.increment()
}
"#;

const BETA_SOURCE: &str = r#"import std

type Thing {
  id: string,
}

actor Thing {
  key(id)
  create()
}

impl Thing {
  function create() -> void {
  }

  function read() -> string {
    return "thing"
  }

  function write(value: string) -> string {
    return value
  }
}

function ping() -> string {
  let thing = std.actor.get<Thing>("t")
  return thing.read()
}
"#;

fn platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler crate manifest directory has a repository parent")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

fn seed_std(artifact_root: &Path) {
    let platform_sources = platform_sources();
    let published = author_official_std_package(&platform_sources).expect("author official std");
    let artifact = package_artifact_ref(&published.artifact).expect("std artifact ref");
    let candidate = PackageArtifactPointer::new(artifact).expect("std pointer candidate");
    let store = CanonicalArtifactStore::create(artifact_root).expect("create artifact store");
    publish_package_artifact_records(artifact_root, &published).expect("publish std records");
    store
        .compare_and_swap_package_artifact_pointer(None, &candidate)
        .expect("install std pointer");
}

fn write_service_root(root: &Path, package_id: &str, service_id: &str, source: &str) {
    write_package_root(root, package_id, source, &[], Some(service_id));
}

fn write_package_root(
    root: &Path,
    package_id: &str,
    source: &str,
    dependencies: &[(&str, &str, &str)],
    service_id: Option<&str>,
) {
    fs::create_dir_all(root).expect("create service root");
    let dependency_lines = dependencies
        .iter()
        .map(|(id, version, alias)| {
            format!("  - id: {id}\n    version: {version}\n    alias: {alias}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let packages_section = if dependencies.is_empty() {
        String::new()
    } else {
        format!("\npackages:\n{dependency_lines}")
    };
    fs::write(
        root.join("package.yml"),
        format!("id: {package_id}\nversion: 1.0.0{packages_section}\n"),
    )
    .expect("write package.yml");
    if let Some(service_id) = service_id {
        fs::write(
            root.join("service.yml"),
            format!("id: {service_id}\nserviceCalls: []\n"),
        )
        .expect("write service.yml");
    }
    fs::write(root.join("api.yml"), "ping: main.ping\n").expect("write api.yml");
    fs::write(root.join("main.skiff"), source).expect("write main.skiff");
}

fn deployment_ref(receipt: &Value) -> ServiceDeploymentRef {
    serde_json::from_value(
        receipt
            .pointer("/serviceDeploymentReceipt/deployment")
            .expect("service deployment receipt")
            .clone(),
    )
    .expect("typed ServiceDeploymentRef")
}

fn package_ref(receipt: &Value) -> PackageArtifactRef {
    serde_json::from_value(
        receipt
            .pointer("/packageArtifactReceipt/artifact")
            .expect("package artifact receipt")
            .clone(),
    )
    .expect("typed PackageArtifactRef")
}

fn load_projection(artifact_root: &Path) -> ActorRoutingProjection {
    let path = artifact_root.join(ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    let bytes = fs::read(&path).expect("actor routing projection record exists");
    let projection: ActorRoutingProjection =
        serde_json::from_slice(&bytes).expect("strict typed decode");
    assert_eq!(
        skiff_canonical_json::canonical_json_bytes(&projection).expect("canonical bytes"),
        bytes,
        "projection record must be canonical JSON"
    );
    projection
}

fn method_keys(projection: &ActorRoutingProjection) -> BTreeSet<ActorMethodKey> {
    projection
        .methods
        .iter()
        .map(|method| {
            (
                method.actor.actor_abi_identity.clone(),
                method.actor_implementation_identity.clone(),
                method.method_identity.clone(),
            )
        })
        .collect()
}

fn expected_methods(
    store: &CanonicalArtifactStore,
    package_ref: &PackageArtifactRef,
) -> BTreeSet<ActorMethodKey> {
    let artifact = store
        .read_package_artifact(package_ref)
        .expect("read package artifact")
        .as_ref()
        .clone();
    let mut methods = BTreeSet::new();
    for file_ref in &artifact.files {
        let unit = store
            .read_file_ir(package_ref, file_ref)
            .expect("read File IR unit");
        for actor in &unit.actor_declarations {
            for method in actor.method_implementations.keys() {
                methods.insert((
                    actor.actor_abi_identity.clone(),
                    actor.actor_implementation_identity.clone(),
                    method.clone(),
                ));
            }
        }
    }
    methods
}

fn unique_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("skiff-{name}-{}-{unique}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_publish_paths_emit_the_canonical_actor_routing_projection() {
        let temp = unique_root("actor-routing-publish");
        let artifact_root = temp.join("artifacts");
        seed_std(&artifact_root);
        let store = CanonicalArtifactStore::open(&artifact_root).expect("open artifact store");

        let alpha_root = temp.join("alpha");
        write_service_root(
            &alpha_root,
            "example.com/alpha-package",
            "example.com/alpha",
            ALPHA_SOURCE,
        );
        let alpha_receipt = build_authoring_object_legacy(
            &platform_sources(),
            AuthoringObject::Package,
            &alpha_root,
            &artifact_root,
            "dev",
            false,
        )
        .expect("alpha package publish");
        let alpha_deployment: ServiceDeploymentRef = deployment_ref(&alpha_receipt);
        let alpha_package: PackageArtifactRef = package_ref(&alpha_receipt);

        let alpha_projection = load_projection(&artifact_root);
        let alpha_expected = expected_methods(&store, &alpha_package);
        assert_eq!(
            method_keys(&alpha_projection),
            alpha_expected,
            "package publish must project exactly the public actor methods (create-only excluded)"
        );
        for entry in &alpha_projection.methods {
            assert_eq!(
                entry.deployment, alpha_deployment,
                "exact deployment binding"
            );
            assert_eq!(entry.package, alpha_package, "exact owning package binding");
        }

        let beta_root = temp.join("beta");
        write_service_root(
            &beta_root,
            "example.com/beta-package",
            "example.com/beta",
            BETA_SOURCE,
        );
        let beta_receipt = build_authoring_object_legacy(
            &platform_sources(),
            AuthoringObject::Package,
            &beta_root,
            &artifact_root,
            "dev",
            false,
        )
        .expect("beta package publish");
        let beta_deployment: ServiceDeploymentRef = deployment_ref(&beta_receipt);
        let beta_package: PackageArtifactRef = package_ref(&beta_receipt);

        let beta_projection = load_projection(&artifact_root);
        let beta_expected = expected_methods(&store, &beta_package);
        assert_eq!(
            method_keys(&beta_projection),
            beta_expected,
            "a later package publish must replace the current projection with its own deployment facts"
        );

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn compiler_publish_deduplicates_shared_dependency_bindings() {
        let temp = unique_root("actor-routing-shared-dependency");
        let artifact_root = temp.join("artifacts");
        seed_std(&artifact_root);
        let store = CanonicalArtifactStore::open(&artifact_root).expect("open artifact store");

        let shared_root = temp.join("shared");
        write_package_root(
            &shared_root,
            "example.com/shared-package",
            ALPHA_SOURCE,
            &[],
            None,
        );
        let shared_receipt = build_authoring_object_legacy(
            &platform_sources(),
            AuthoringObject::Package,
            &shared_root,
            &artifact_root,
            "dev",
            true,
        )
        .expect("shared package publish");
        let shared_package = package_ref(&shared_receipt);

        let middle_root = temp.join("middle");
        write_package_root(
            &middle_root,
            "example.com/middle-package",
            "function ping() -> string { return \"middle\" }\n",
            &[("example.com/shared-package", "1.0.0", "shared")],
            None,
        );
        build_authoring_object_legacy(
            &platform_sources(),
            AuthoringObject::Package,
            &middle_root,
            &artifact_root,
            "dev",
            true,
        )
        .expect("middle package publish");

        let service_root = temp.join("service");
        write_package_root(
            &service_root,
            "example.com/service-package",
            "function ping() -> string { return \"ok\" }\n",
            &[
                ("example.com/middle-package", "1.0.0", "middle"),
                ("example.com/shared-package", "1.0.0", "shared"),
            ],
            Some("example.com/service"),
        );
        let service_receipt = build_authoring_object_legacy(
            &platform_sources(),
            AuthoringObject::Package,
            &service_root,
            &artifact_root,
            "dev",
            true,
        )
        .expect("service publish must project each shared dependency binding only once");
        let service_deployment = deployment_ref(&service_receipt);

        let projection = load_projection(&artifact_root);
        let shared_expected = expected_methods(&store, &shared_package);
        assert_eq!(
            method_keys(&projection),
            shared_expected,
            "shared dependency methods must appear exactly once despite duplicate bindings"
        );
        for entry in &projection.methods {
            assert_eq!(
                entry.deployment, service_deployment,
                "exact deployment binding"
            );
            assert_eq!(
                entry.package, shared_package,
                "exact owning package binding"
            );
        }

        fs::remove_dir_all(temp).unwrap();
    }
}
