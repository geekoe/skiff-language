use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::{
    assign_package_artifact_identities, assign_service_deployment_identity, package_artifact_ref,
    service_deployment_ref, ValidatedBytecodeArtifact,
};
use skiff_artifact_model::{
    BytecodeArtifactRef, CallableEffectSummary, GatewayEntryIdentity, IngressSelector,
    PackageArtifact, PackageArtifactRef, PackageLocalAbiSymbol, PendingEffectCategory,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_linker::{DeploymentExecutionLimits, LinkLimits};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub const PHASE_1_SCALAR_LOCAL_SOURCE: &str =
    "function helper(value: number) -> number { return value + 5 }\n\
     function run(value: number) -> number { final result = helper(value) if result == 7 { return result - 4 } return 0 }\n";

pub struct PublishedService {
    pub package: Arc<PackageArtifact>,
    pub bytecode: Arc<ValidatedBytecodeArtifact>,
    contract: Arc<ServiceContract>,
    deployment: Arc<ServiceDeployment>,
}

impl PublishedService {
    pub fn build(scenario: &str) -> Self {
        Self::build_from_source(
            scenario,
            "function run(value: number) -> number { return value }\n\
             function unused() -> number { return 2 }\n",
        )
    }

    pub fn build_from_source(scenario: &str, source: &str) -> Self {
        let receipt = author_package(scenario, source).expect("publish canonical fixture");
        let artifact_root = receipt.artifact_root.path();
        let output = receipt.output;
        let package_ref = serde_json::from_value::<PackageArtifactRef>(
            output
                .pointer("/packageArtifactReceipt/artifact")
                .cloned()
                .expect("authoring receipt contains the exact package"),
        )
        .expect("package receipt remains typed");
        let deployment_ref = serde_json::from_value::<ServiceDeploymentRef>(
            output
                .pointer("/serviceDeploymentReceipt/deployment")
                .cloned()
                .expect("authoring receipt contains the exact deployment"),
        )
        .expect("deployment receipt remains typed");
        let store = CanonicalArtifactStore::open(artifact_root).expect("open canonical store");
        let package = store
            .read_package_artifact(&package_ref)
            .expect("read exact package record");
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("production compiler publishes bytecode");
        let bytecode = store
            .read_package_bytecode(&package_ref, bytecode_ref)
            .expect("read through structural and identity admission");
        let deployment = store
            .read_service_deployment(&deployment_ref)
            .expect("read exact deployment record");
        let contract = store
            .read_service_contract(&deployment.contract)
            .expect("read exact contract record");
        Self {
            package,
            bytecode,
            contract,
            deployment,
        }
    }

    pub fn hydrated(&self) -> HydratedDeploymentBytecode {
        let deployment_ref = service_deployment_ref(&self.deployment);
        let resolver = ExactResolver {
            deployment_ref: deployment_ref.clone(),
            deployment: Arc::clone(&self.deployment),
            contract: Arc::clone(&self.contract),
            package_ref: package_artifact_ref(&self.package).expect("derive exact package ref"),
            package: Arc::clone(&self.package),
            bytecode: Arc::clone(&self.bytecode),
        };
        DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_ref)
            .expect("production loader admits the exact canonical publication")
    }

    pub fn http_gateway_lookup(&self) -> (IngressSelector, GatewayEntryIdentity) {
        let binding = self
            .deployment
            .ingress
            .first()
            .expect("published service has one HTTP ingress");
        let entry = self
            .deployment
            .gateway_entries
            .get(&binding.gateway_entry_key)
            .expect("HTTP ingress names its exact gateway entry");
        (
            binding.selector.clone(),
            entry.gateway_entry_identity.clone(),
        )
    }

    pub fn with_pending_effect(
        &self,
        function_suffix: &str,
    ) -> (HydratedDeploymentBytecode, PackageArtifactRef, String) {
        let mut package = self.package.as_ref().clone();
        let (function_key, owner) = self
            .bytecode
            .artifact()
            .image
            .functions
            .iter()
            .find(|(key, _)| key.ends_with(function_suffix))
            .map(|(key, function)| (key.clone(), function.effect_summary_ref.clone()))
            .unwrap_or_else(|| panic!("fixture has function suffix {function_suffix}"));
        let facts = package
            .callable_semantic_facts
            .get_mut(&owner)
            .expect("function effect owner is source-owned package fact");
        let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
            panic!("compiler must publish analyzed effects")
        };
        effects.may_pending = true;
        effects.pending_effect_categories = vec![PendingEffectCategory::HostEffect];
        for symbols in [
            &mut package.package_local_abi.public_symbols,
            &mut package.package_local_abi.implementation_symbols,
        ] {
            for symbol in symbols.values_mut() {
                if let PackageLocalAbiSymbol::Callable {
                    callable_id,
                    signature,
                } = symbol
                {
                    if callable_id == &owner {
                        signature.may_suspend = true;
                    }
                }
            }
        }
        assign_package_artifact_identities(&mut package)
            .expect("identity assignment accepts a conservative pending declaration");
        let package = Arc::new(package);
        let package_ref = package_artifact_ref(&package).expect("derive exact changed package ref");

        let mut deployment = self.deployment.as_ref().clone();
        deployment.implementation = package_ref.clone();
        assign_service_deployment_identity(&mut deployment)
            .expect("re-pin deployment to changed package identity");
        let deployment = Arc::new(deployment);
        let deployment_ref = service_deployment_ref(&deployment);
        let resolver = ExactResolver {
            deployment_ref: deployment_ref.clone(),
            deployment,
            contract: Arc::clone(&self.contract),
            package_ref: package_ref.clone(),
            package,
            bytecode: Arc::clone(&self.bytecode),
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_ref)
            .expect("production loader admits exact mutated effect owner");
        (hydrated, package_ref, function_key)
    }
}

struct ExactResolver {
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    contract: Arc<ServiceContract>,
    package_ref: PackageArtifactRef,
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

impl DeploymentBytecodeContentResolver for ExactResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        anyhow::ensure!(
            reference == &self.deployment_ref,
            "deployment identity mismatch"
        );
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        anyhow::ensure!(
            reference == &self.deployment.contract,
            "contract identity mismatch"
        );
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        anyhow::ensure!(reference == &self.package_ref, "package identity mismatch");
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        package: &PackageArtifactRef,
        reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        anyhow::ensure!(package == &self.package_ref, "bytecode package mismatch");
        anyhow::ensure!(
            reference == self.bytecode.reference(),
            "bytecode identity mismatch"
        );
        Ok(Arc::clone(&self.bytecode))
    }
}

#[derive(Debug)]
struct AuthoringReceipt {
    artifact_root: TempRoot,
    output: serde_json::Value,
}

fn author_package(
    scenario: &str,
    source: &str,
) -> Result<AuthoringReceipt, Box<dyn std::error::Error + Send + Sync>> {
    let source_root = package_source(scenario, source, true);
    let artifact_root = TempRoot::create(&format!("phase-1-contract-artifact-{scenario}"));
    let platform = CompilerPlatformSources::new(&repository_root())?;
    let output = build_authoring_object(
        &platform,
        AuthoringObject::Package,
        source_root.path(),
        artifact_root.path(),
        "phase-1-contract",
        false,
    )?;
    Ok(AuthoringReceipt {
        artifact_root,
        output,
    })
}

pub fn package_source(scenario: &str, source: &str, service: bool) -> TempRoot {
    let source_root = TempRoot::create(&format!("phase-1-contract-source-{scenario}"));
    let package_id = format!("test.skiff/phase-1-contract-{scenario}");
    fs::write(
        source_root.path().join("package.yml"),
        format!("id: {package_id}\nversion: 1.0.0\n"),
    )
    .expect("write package manifest");
    fs::write(source_root.path().join("main.skiff"), source).expect("write package source");
    fs::write(source_root.path().join("api.yml"), "{}\n").expect("write package API manifest");
    if service {
        fs::write(
            source_root.path().join("service.yml"),
            format!("id: {package_id}\n"),
        )
        .expect("write service manifest");
        fs::write(
            source_root.path().join("http.yml"),
            "run:\n  method: POST\n  path: /phase-1/contract\n  kind: typedJson\n  handler: main.run\n  adapterArgs:\n    - param: value\n      source: { kind: http.body }\n",
        )
        .expect("write HTTP manifest");
    }
    source_root
}

pub fn production_sized_limits() -> LinkLimits {
    LinkLimits {
        max_packages: 8,
        max_root_specializations: 16,
        max_specializations: 64,
        max_code_words_per_function: 16_384,
        max_total_code_words: 65_536,
        max_relocations_per_function: 4_096,
        max_total_relocations: 16_384,
        max_image_table_entries: 4_096,
        max_total_image_table_entries: 32_768,
        max_total_function_table_entries: 32_768,
        max_type_nesting_depth: 64,
        max_expanded_type_nodes: 65_536,
        max_expanded_type_bytes: 4 * 1024 * 1024,
        max_constant_graph_nodes: 65_536,
        max_constant_graph_edges: 262_144,
    }
}

pub fn production_sized_execution_limits() -> DeploymentExecutionLimits {
    DeploymentExecutionLimits::new(production_sized_limits(), verification_limits())
}

fn verification_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: 64,
        max_total_instructions: 65_536,
        max_instructions_per_function: 16_384,
        max_frame_slots_per_function: 4_096,
        max_operand_depth: 4_096,
        max_control_flow_edges_per_function: 65_536,
        max_exception_regions_per_function: 4_096,
        max_switch_targets_per_function: 4_096,
        max_statement_events_per_pc: 64,
        max_statement_events_per_function: 65_536,
        max_total_statement_events: 262_144,
        max_source_map_entries_per_function: 65_536,
        max_image_table_entries: 32_768,
        max_arity: 256,
        max_callback_captures_per_callback: 4_096,
        max_type_nesting_depth: 64,
        max_value_lifecycle_nodes: 65_536,
        max_value_lifecycle_canonical_bytes: 4 * 1024 * 1024,
        max_constant_graph_edges: 262_144,
    }
}

#[derive(Debug)]
pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn create(prefix: &str) -> Self {
        let ordinal = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{ordinal}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos(),
        ));
        fs::create_dir(&path).expect("create unique temporary root");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/linker lives below repository root")
        .to_path_buf()
}
