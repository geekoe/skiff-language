use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use skiff_artifact_model::{
    ConfigLiteralBinding, FileIrRef, FileIrUnit, MetadataValue, PackageArtifact,
    PackageArtifactRef, PackageSchemaIndex, PackageSchemaIndexRef, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, PublicationResourceRef, RuntimeAssemblyRef, SecretRefBinding,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{
    HydratedRuntimeAssembly, RuntimeAssemblyContentResolver, RuntimeAssemblyLoader,
    RuntimeAssemblyRecordResolver,
};

/// Production filesystem resolver for the typed canonical artifact store.
///
/// Every path comes from an exact typed reference. Raw coordinates are checked
/// before typed deserialization by the store; no legacy pointer/index or host
/// admission hook participates in hydration.
#[derive(Debug, Clone)]
pub struct FilesystemRuntimeAssemblyContentResolver {
    artifact_root: PathBuf,
    store: CanonicalArtifactStore,
}

impl FilesystemRuntimeAssemblyContentResolver {
    pub fn open(artifact_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let artifact_root = artifact_root.as_ref().to_path_buf();
        Ok(Self {
            store: CanonicalArtifactStore::open(&artifact_root)?,
            artifact_root,
        })
    }

    pub fn from_store(store: CanonicalArtifactStore) -> Self {
        let artifact_root = store.root().to_path_buf();
        Self {
            artifact_root,
            store,
        }
    }

    pub fn store(&self) -> &CanonicalArtifactStore {
        &self.store
    }

    pub fn load_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<HydratedRuntimeAssembly> {
        RuntimeAssemblyLoader::new(self).load_ref(reference)
    }
}

impl RuntimeAssemblyContentResolver for FilesystemRuntimeAssemblyContentResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        Ok(self.store.read_service_deployment(reference)?)
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        Ok(self.store.read_service_contract(reference)?)
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        Ok(self.store.read_package_schema_index(reference)?)
    }

    fn resolve_package_schema_type(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>> {
        Ok(self.store.read_package_schema_type_record(reference)?)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        Ok(self.store.read_package_artifact(reference)?)
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        Ok(self.store.read_file_ir(package, reference)?)
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        Ok(self.store.read_static_resource(package, reference)?)
    }

    fn resolve_activation_secrets(
        &self,
        environment: &str,
        deployment: &ServiceDeploymentRef,
        bindings: &[SecretRefBinding],
    ) -> anyhow::Result<Vec<ConfigLiteralBinding>> {
        if bindings.is_empty() {
            return Ok(Vec::new());
        }
        skiff_artifact_model::validate_activation_environment(environment)
            .map_err(anyhow::Error::msg)?;
        let service_storage_id = skiff_artifact_identity::publication_storage_segment(
            &deployment.service_id,
            "service id",
        )?;
        let path = self
            .artifact_root
            .join("configs")
            .join("services")
            .join(service_storage_id)
            .join(format!("config.{environment}.secret.yml"));
        let source = fs::read_to_string(&path).map_err(|error| {
            anyhow::anyhow!(
                "failed to read runtime activation secret source for service {}: {}",
                deployment.service_id,
                error.kind()
            )
        })?;
        resolve_service_secret_bindings(&source, deployment, bindings)
    }
}

impl RuntimeAssemblyRecordResolver for FilesystemRuntimeAssemblyContentResolver {
    fn resolve_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::RuntimeAssembly>> {
        Ok(self.store.read_runtime_assembly(reference)?)
    }
}

fn resolve_service_secret_bindings(
    source: &str,
    deployment: &ServiceDeploymentRef,
    bindings: &[SecretRefBinding],
) -> anyhow::Result<Vec<ConfigLiteralBinding>> {
    let document: serde_json::Value = serde_yaml::from_str(source).map_err(|_| {
        anyhow::anyhow!(
            "runtime activation secret source is invalid YAML for service {}",
            deployment.service_id
        )
    })?;
    let service = document
        .as_object()
        .and_then(|root| root.get("service"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime activation secret source has no service object for service {}",
                deployment.service_id
            )
        })?;
    let mut seen = BTreeSet::new();
    let mut resolved = Vec::with_capacity(bindings.len());
    for binding in bindings {
        if !seen.insert(binding.path.as_str()) {
            anyhow::bail!(
                "runtime activation repeats secret path {} for service {}",
                binding.path,
                deployment.service_id
            );
        }
        let value = resolve_dotted_json_path(service, &binding.path).ok_or_else(|| {
            anyhow::anyhow!(
                "runtime activation secret source is missing path {} for service {}",
                binding.path,
                deployment.service_id
            )
        })?;
        if value.is_null() {
            anyhow::bail!(
                "runtime activation secret source path {} is null for service {}",
                binding.path,
                deployment.service_id
            );
        }
        resolved.push(ConfigLiteralBinding {
            path: binding.path.clone(),
            value: MetadataValue::from_json(value.clone()),
        });
    }
    Ok(resolved)
}

fn resolve_dotted_json_path<'a>(
    root: &'a serde_json::Map<String, serde_json::Value>,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut segments = path.split('.');
    let first = segments.next()?;
    if first.is_empty() {
        return None;
    }
    let mut value = root.get(first)?;
    for segment in segments {
        if segment.is_empty() {
            return None;
        }
        value = value.as_object()?.get(segment)?;
    }
    Some(value)
}

#[cfg(test)]
mod secret_tests {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{resolve_service_secret_bindings, FilesystemRuntimeAssemblyContentResolver};
    use crate::RuntimeAssemblyContentResolver;
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentRevision, SecretRefBinding, ServiceDeploymentRef,
    };

    static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "skiff-runtime-secret-resolver-test-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("create secret resolver test root");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn deployment() -> ServiceDeploymentRef {
        deployment_for("example.com/service")
    }

    fn deployment_for(service_id: &str) -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: service_id.to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("sha256-test"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(
                "skiff-deployment-artifact-v4:sha256:test",
            ),
        }
    }

    fn binding(path: &str) -> SecretRefBinding {
        SecretRefBinding {
            path: path.to_string(),
            secret_ref: format!("secret:{}", path.replace('.', "-")),
        }
    }

    fn write_secret_source(root: &TestRoot, service_id: &str, environment: &str, source: &str) {
        let service_storage_id =
            skiff_artifact_identity::publication_storage_segment(service_id, "service id")
                .expect("project service storage id");
        let service_config_dir = root
            .path()
            .join("configs")
            .join("services")
            .join(service_storage_id);
        std::fs::create_dir_all(&service_config_dir).expect("create service config directory");
        std::fs::write(
            service_config_dir.join(format!("config.{environment}.secret.yml")),
            source,
        )
        .expect("write test secret source");
    }

    #[test]
    fn resolves_only_bound_service_secret_paths() {
        let bindings = vec![binding("provider.apiKey")];
        let resolved = resolve_service_secret_bindings(
            "service:\n  provider:\n    apiKey: test-secret\n  unrelated: ignored\n",
            &deployment(),
            &bindings,
        )
        .expect("bound service secret should resolve");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, "provider.apiKey");
        assert_eq!(
            resolved[0].value,
            skiff_artifact_model::MetadataValue::String("test-secret".to_string())
        );
    }

    #[test]
    fn rejects_missing_bound_service_secret_path_without_exposing_values() {
        let bindings = vec![binding("provider.apiKey")];
        let error = resolve_service_secret_bindings(
            "service:\n  provider:\n    other: do-not-report\n",
            &deployment(),
            &bindings,
        )
        .expect_err("missing bound secret must fail closed");
        let message = error.to_string();
        assert!(message.contains("provider.apiKey"));
        assert!(!message.contains("do-not-report"));
    }

    #[test]
    fn rejects_unknown_and_null_bindings_without_exposing_source_values() {
        let source =
            "service:\n  provider:\n    apiKey: source-value-must-stay-private\n    nullKey:\n";
        for path in ["provider.unknown", "provider.nullKey"] {
            let error = resolve_service_secret_bindings(source, &deployment(), &[binding(path)])
                .expect_err("unknown and null secret bindings must fail closed");
            let message = error.to_string();
            assert!(message.contains(path));
            assert!(!message.contains("source-value-must-stay-private"));
        }
    }

    #[test]
    fn rejects_invalid_yaml_without_exposing_source_contents() {
        let error = resolve_service_secret_bindings(
            "service: [private-source-value",
            &deployment(),
            &[binding("provider.apiKey")],
        )
        .expect_err("invalid secret YAML must fail closed");
        let message = error.to_string();
        assert!(message.contains("invalid YAML"));
        assert!(!message.contains("private-source-value"));
    }

    #[test]
    fn filesystem_resolver_reads_only_exact_environment_and_service_secret_source() {
        let root = TestRoot::new();
        write_secret_source(
            &root,
            "example.com/service",
            "dev",
            "service:\n  provider:\n    apiKey: test-secret\n",
        );
        write_secret_source(
            &root,
            "example.com/service",
            "prod",
            "service:\n  provider:\n    apiKey: wrong-environment-value\n",
        );
        write_secret_source(
            &root,
            "example.com/other",
            "dev",
            "service:\n  provider:\n    apiKey: wrong-service-value\n",
        );
        let resolver = FilesystemRuntimeAssemblyContentResolver::open(root.path())
            .expect("open filesystem resolver");
        let bindings = vec![binding("provider.apiKey")];

        let resolved = resolver
            .resolve_activation_secrets("dev", &deployment(), &bindings)
            .expect("resolve exact environment service secret");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, "provider.apiKey");
        assert_eq!(
            resolved[0].value,
            skiff_artifact_model::MetadataValue::String("test-secret".to_string())
        );
    }

    #[test]
    fn filesystem_resolver_never_falls_back_to_wrong_service_or_environment() {
        let root = TestRoot::new();
        write_secret_source(
            &root,
            "example.com/service",
            "dev",
            "service:\n  provider:\n    apiKey: dev-only-value\n",
        );
        write_secret_source(
            &root,
            "example.com/other",
            "prod",
            "service:\n  provider:\n    apiKey: other-service-value\n",
        );
        let resolver = FilesystemRuntimeAssemblyContentResolver::open(root.path())
            .expect("open filesystem resolver");
        let bindings = vec![binding("provider.apiKey")];

        let wrong_environment = resolver
            .resolve_activation_secrets("prod", &deployment(), &bindings)
            .expect_err("resolver must not fall back to another environment");
        let wrong_service = resolver
            .resolve_activation_secrets("dev", &deployment_for("example.com/missing"), &bindings)
            .expect_err("resolver must not fall back to another service");

        for message in [wrong_environment.to_string(), wrong_service.to_string()] {
            assert!(message.contains("failed to read"));
            assert!(!message.contains("dev-only-value"));
            assert!(!message.contains("other-service-value"));
        }
    }

    #[test]
    fn filesystem_resolver_rejects_invalid_environment_before_path_resolution() {
        let root = TestRoot::new();
        let resolver = FilesystemRuntimeAssemblyContentResolver::open(root.path())
            .expect("open filesystem resolver");

        let error = resolver
            .resolve_activation_secrets("../prod", &deployment(), &[binding("provider.apiKey")])
            .expect_err("invalid environment must fail before filesystem lookup");

        assert!(error.to_string().contains("environment"));
    }
}
