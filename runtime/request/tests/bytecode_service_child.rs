#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, OnceLock,
        },
    };

    use skiff_artifact_model::{
        PackageBuildId, ServiceContractRef, ServiceProtocolIdentity, ServiceRequirementKey,
    };
    use skiff_compiler::{
        authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
        CompilerPlatformSources,
    };
    use skiff_runtime_deployment_image::ServiceDependencySlot;
    use skiff_runtime_linker::{
        link_deployment_execution_image, DeploymentExecutionImage, LinkLimits,
    };
    use skiff_runtime_loader::{
        DeploymentBytecodeLoader, FilesystemDeploymentBytecodeContentResolver,
    };
    use skiff_runtime_request::{
        BytecodeRequestChildComposition, BytecodeServiceChildError, BytecodeServiceResolver,
        FailClosedServiceChildThrowMaterializer, ServiceChildThrowMaterializer,
    };

    static NEXT_PROVIDER_TEMP: AtomicU64 = AtomicU64::new(0);
    static PROVIDER_IMAGE: OnceLock<Arc<DeploymentExecutionImage>> = OnceLock::new();

    fn provider_image() -> Arc<DeploymentExecutionImage> {
        Arc::clone(PROVIDER_IMAGE.get_or_init(|| {
            let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("request crate must be under the repository root")
                .to_path_buf();
            let fixture_root = repository_root.join(
                "runtime/host/src/host/request_entry/phase_4_proof_support/fixtures/vcp4-sleep",
            );
            let artifact_root = std::env::temp_dir().join(format!(
                "skiff-request-service-child-provider-{}-{}",
                std::process::id(),
                NEXT_PROVIDER_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&artifact_root).unwrap();
            let sources = CompilerPlatformSources::new(&repository_root)
                .expect("open repository platform sources");
            seed_official_std_package(&sources, &artifact_root)
                .expect("seed canonical std into provider fixture store");
            let receipt = build_authoring_object(
                &sources,
                AuthoringObject::Package,
                &fixture_root,
                &artifact_root,
                "skiff-test",
                true,
            )
            .expect("provider fixture publishes through production authoring");
            let deployment = serde_json::from_value(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("provider authoring receipt carries deployment"),
            )
            .expect("provider deployment receipt remains typed");
            let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
                .expect("open provider fixture store");
            let hydrated = DeploymentBytecodeLoader::new(&resolver)
                .load(&deployment)
                .expect("load provider fixture closure");
            let image = Arc::new(
                link_deployment_execution_image(hydrated, &provider_link_limits())
                    .expect("link provider fixture image"),
            );
            fs::remove_dir_all(&artifact_root).unwrap();
            image
        }))
    }

    fn provider_link_limits() -> LinkLimits {
        LinkLimits {
            max_packages: u64::MAX,
            max_root_specializations: u64::MAX,
            max_specializations: u64::MAX,
            max_code_words_per_function: u64::MAX,
            max_total_code_words: u64::MAX,
            max_relocations_per_function: u64::MAX,
            max_total_relocations: u64::MAX,
            max_image_table_entries: u64::MAX,
            max_total_image_table_entries: u64::MAX,
            max_total_function_table_entries: u64::MAX,
            max_type_nesting_depth: u64::MAX,
            max_expanded_type_nodes: u64::MAX,
            max_expanded_type_bytes: u64::MAX,
            max_constant_graph_nodes: u64::MAX,
            max_constant_graph_edges: u64::MAX,
        }
    }

    fn test_slot(protocol: &str) -> ServiceDependencySlot {
        ServiceDependencySlot::try_new(
            ServiceRequirementKey {
                caller_package_build_id: PackageBuildId::new("build:caller"),
                service_requirement_slot: 0,
            },
            ServiceContractRef {
                service_id: "example.com/provider".to_string(),
                contract_version: "1.0.0".to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new(protocol),
            },
            Vec::<skiff_artifact_model::ContractOperationId>::new(),
        )
        .expect("dependency slot accepts an empty operation set")
    }

    fn test_operation() -> skiff_artifact_model::ContractOperationId {
        skiff_artifact_identity::contract_operation_id("example.com/provider", "1.0.0", "run")
            .unwrap()
    }

    #[test]
    fn default_service_resolver_fails_closed_for_missing_provider() {
        let composition = BytecodeRequestChildComposition::default();
        let slot = ServiceDependencySlot::try_new(
            ServiceRequirementKey {
                caller_package_build_id: PackageBuildId::new("build:caller"),
                service_requirement_slot: 0,
            },
            ServiceContractRef {
                service_id: "example.com/provider".to_string(),
                contract_version: "1.0.0".to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            },
            Vec::<skiff_artifact_model::ContractOperationId>::new(),
        )
        .expect("dependency slot accepts an empty operation set");
        let operation =
            skiff_artifact_identity::contract_operation_id("example.com/provider", "1.0.0", "run")
                .unwrap();
        let error = composition
            .service_resolver
            .resolve_service(
                &slot,
                &operation,
                &ServiceProtocolIdentity::new("unassigned"),
            )
            .expect_err("default resolver must fail closed");
        assert!(matches!(
            error,
            BytecodeServiceChildError::ProviderMissing {
                service_id,
                contract_version,
            } if service_id == "example.com/provider" && contract_version == "1.0.0"
        ));
        let _ = Arc::new(composition);
    }

    #[test]
    fn protocol_mismatch_resolver_failure_is_typed() {
        struct ProtocolMismatchResolver;

        impl BytecodeServiceResolver for ProtocolMismatchResolver {
            fn resolve_service(
                &self,
                _slot: &ServiceDependencySlot,
                _operation: &skiff_artifact_model::ContractOperationId,
                _expected_protocol: &ServiceProtocolIdentity,
            ) -> Result<
                std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
                BytecodeServiceChildError,
            > {
                Err(BytecodeServiceChildError::ProtocolMismatch {
                    expected: ServiceProtocolIdentity::new("expected-protocol"),
                    actual: ServiceProtocolIdentity::new("actual-protocol"),
                })
            }
        }

        let mut composition = BytecodeRequestChildComposition::default();
        composition.service_resolver = Arc::new(ProtocolMismatchResolver);
        let slot = ServiceDependencySlot::try_new(
            ServiceRequirementKey {
                caller_package_build_id: PackageBuildId::new("build:caller"),
                service_requirement_slot: 0,
            },
            ServiceContractRef {
                service_id: "example.com/provider".to_string(),
                contract_version: "1.0.0".to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("expected-protocol"),
            },
            Vec::<skiff_artifact_model::ContractOperationId>::new(),
        )
        .expect("dependency slot accepts an empty operation set");
        let operation =
            skiff_artifact_identity::contract_operation_id("example.com/provider", "1.0.0", "run")
                .unwrap();
        let error = composition
            .service_resolver
            .resolve_service(
                &slot,
                &operation,
                &ServiceProtocolIdentity::new("expected-protocol"),
            )
            .expect_err("protocol mismatch resolver must fail closed");
        assert!(matches!(
            error,
            BytecodeServiceChildError::ProtocolMismatch { .. }
        ));
    }

    #[test]
    fn fail_closed_throw_materializer_is_injectable_into_composition() {
        let mut composition = BytecodeRequestChildComposition::default();
        composition.throw_materializer = Arc::new(FailClosedServiceChildThrowMaterializer);
        let _: Arc<dyn ServiceChildThrowMaterializer> = Arc::clone(&composition.throw_materializer);
    }

    #[test]
    fn provider_drift_resolver_failure_is_typed() {
        struct DriftResolver;

        impl BytecodeServiceResolver for DriftResolver {
            fn resolve_service(
                &self,
                _slot: &ServiceDependencySlot,
                _operation: &skiff_artifact_model::ContractOperationId,
                _expected_protocol: &ServiceProtocolIdentity,
            ) -> Result<std::sync::Arc<DeploymentExecutionImage>, BytecodeServiceChildError>
            {
                Err(BytecodeServiceChildError::DeploymentDrift)
            }
        }

        let mut composition = BytecodeRequestChildComposition::default();
        composition.service_resolver = Arc::new(DriftResolver);
        let error = composition
            .service_resolver
            .resolve_service(
                &test_slot("expected-protocol"),
                &test_operation(),
                &ServiceProtocolIdentity::new("expected-protocol"),
            )
            .expect_err("provider drift resolver must fail closed");
        assert!(matches!(error, BytecodeServiceChildError::DeploymentDrift));
    }

    #[test]
    fn provider_load_failure_resolver_failure_is_typed() {
        struct LoadFailureResolver;

        impl BytecodeServiceResolver for LoadFailureResolver {
            fn resolve_service(
                &self,
                _slot: &ServiceDependencySlot,
                _operation: &skiff_artifact_model::ContractOperationId,
                _expected_protocol: &ServiceProtocolIdentity,
            ) -> Result<std::sync::Arc<DeploymentExecutionImage>, BytecodeServiceChildError>
            {
                Err(BytecodeServiceChildError::Load {
                    message: "deployment execution image construction failed".to_string(),
                })
            }
        }

        let mut composition = BytecodeRequestChildComposition::default();
        composition.service_resolver = Arc::new(LoadFailureResolver);
        let error = composition
            .service_resolver
            .resolve_service(
                &test_slot("expected-protocol"),
                &test_operation(),
                &ServiceProtocolIdentity::new("expected-protocol"),
            )
            .expect_err("provider load failure resolver must fail closed");
        assert!(matches!(
            error,
            BytecodeServiceChildError::Load { message }
                if message.contains("deployment execution image construction failed")
        ));
    }

    #[test]
    fn provider_present_resolver_returns_the_loaded_image() {
        struct PresentResolver {
            image: Arc<DeploymentExecutionImage>,
        }

        impl BytecodeServiceResolver for PresentResolver {
            fn resolve_service(
                &self,
                _slot: &ServiceDependencySlot,
                _operation: &skiff_artifact_model::ContractOperationId,
                _expected_protocol: &ServiceProtocolIdentity,
            ) -> Result<std::sync::Arc<DeploymentExecutionImage>, BytecodeServiceChildError>
            {
                Ok(Arc::clone(&self.image))
            }
        }

        let image = provider_image();
        let mut composition = BytecodeRequestChildComposition::default();
        composition.service_resolver = Arc::new(PresentResolver {
            image: Arc::clone(&image),
        });
        let resolved = composition
            .service_resolver
            .resolve_service(
                &test_slot("expected-protocol"),
                &test_operation(),
                &ServiceProtocolIdentity::new("expected-protocol"),
            )
            .expect("present provider resolver must succeed");
        assert!(Arc::ptr_eq(&image, &resolved));
    }
}
