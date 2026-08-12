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
    service_deployment_ref, ArtifactIdentityError, ValidatedBytecodeArtifact,
};
use skiff_artifact_model::{
    BytecodeArtifactRef, BytecodeDecodeError, CallableEffectSummary, PackageArtifact,
    PackageArtifactRef, PendingEffectCategory, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, StructuralValidationError,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources, PackageCompileError,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_linker::{
    link_deployment, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation, LinkLimits,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};
use skiff_test_runner::canonical_package::{compile_package_project, CanonicalPackageProjectError};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn unsupported_typed_source_is_owned_by_phase_1_compiler_admission() {
    let source = r#"function run() -> string { return "disabled" }"#;
    let source_root = package_source("unsupported-source", source, false);
    let artifact_root = TempRoot::create("phase-1-contract-artifact-unsupported-source");
    let platform = CompilerPlatformSources::new(&repository_root()).expect("open platform source");
    let error = compile_package_project(&platform, source_root.path(), artifact_root.path())
        .expect_err("Phase 1 compiler admission must reject string values before emission");
    let CanonicalPackageProjectError::Compile(PackageCompileError::BytecodeEmission { source }) =
        error
    else {
        panic!("unsupported typed source must be owned by bytecode emission: {error:?}");
    };
    assert_eq!(
        format!("{source:?}"),
        "UnsupportedPhase1Capability { capability: ValueShape, module_path: \"main\", function_key: Some(\"main::run\"), location: \"return type\" }",
        "the source-owned capability category and location are part of the exact contract",
    );
}

#[test]
fn malformed_word_is_owned_by_bounded_structural_admission() {
    let fixture = PublishedService::build("malformed-structural");
    let mut artifact = fixture.bytecode.artifact().clone();
    let (function_key, function) = artifact
        .image
        .functions
        .iter_mut()
        .next()
        .expect("production compiler emits at least one function");
    let function_key = function_key.clone();
    function.words = vec![0xffff_ffff];
    function.statement_entries.clear();
    function.source_map.clear();

    let error = ValidatedBytecodeArtifact::admit(artifact)
        .expect_err("bounded artifact admission must reject an unknown opcode word");
    assert!(matches!(
        error,
        ArtifactIdentityError::InvalidBytecodeStructural(
            StructuralValidationError::Decode {
                function_key: actual_function,
                error: BytecodeDecodeError::UnknownOpcode {
                    pc: 0,
                    word: 0xffff_ffff,
                },
            },
        ) if actual_function == function_key
    ));
}

#[test]
fn bytecode_content_identity_mismatch_is_owned_by_artifact_admission() {
    let fixture = PublishedService::build("identity-mismatch");
    let mut artifact = fixture.bytecode.artifact().clone();
    let computed = artifact.bytecode_identity.clone();
    let last = artifact
        .bytecode_identity
        .pop()
        .expect("canonical identity is non-empty");
    artifact
        .bytecode_identity
        .push(if last == '0' { '1' } else { '0' });
    let declared = artifact.bytecode_identity.clone();

    let error = ValidatedBytecodeArtifact::admit(artifact)
        .expect_err("content drift must not retain the declared bytecode identity");
    assert!(matches!(
        error,
        ArtifactIdentityError::BytecodeIdentityMismatch {
            declared: actual_declared,
            computed: actual_computed,
        } if actual_declared == declared && actual_computed == computed
    ));
}

#[test]
fn reachable_pending_effect_is_rejected_by_the_link_capability_owner() {
    let fixture = PublishedService::build("reachable-effect");
    let (hydrated, package, function_key) = fixture.with_pending_effect("::run");
    let expected_detail =
        "Phase 1 capability gate rejected reachable pending effect HostEffect".to_string();

    let error = link_deployment(&hydrated, &production_sized_limits())
        .expect_err("a reachable host effect must not enter the Phase 1 executable closure");
    assert_eq!(
        error,
        BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::CallableEffectPlan,
            location: BytecodeLinkLocation::Function {
                package: Box::new(package),
                function_key,
            },
            detail: expected_detail,
        },
    );
}

#[test]
fn design_dependent_unreachable_pending_effect_is_not_a_raw_artifact_scan() {
    let fixture = PublishedService::build("unreachable-effect");
    let (hydrated, _package, unsupported_function) = fixture.with_pending_effect("::unused");

    let candidate = link_deployment(&hydrated, &production_sized_limits())
        .expect("an unreachable disabled function must not replace entry-closure admission");
    assert!(candidate.functions().iter().all(|function| {
        function.key().artifact_function_key().as_str() != unsupported_function
    }));
}

struct PublishedService {
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
    contract: Arc<ServiceContract>,
    deployment: Arc<ServiceDeployment>,
}

impl PublishedService {
    fn build(scenario: &str) -> Self {
        let source = "function run(value: number) -> number { return value }\n\
                      function unused() -> number { return 2 }\n";
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

    fn with_pending_effect(
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

fn package_source(scenario: &str, source: &str, service: bool) -> TempRoot {
    let source_root = TempRoot::create(&format!("phase-1-contract-source-{scenario}"));
    let package_id = format!("test.skiff/phase-1-contract-{scenario}");
    fs::write(
        source_root.path().join("package.yml"),
        format!("id: {package_id}\nversion: 1.0.0\n"),
    )
    .expect("write package manifest");
    fs::write(source_root.path().join("main.skiff"), source).expect("write package source");
    if service {
        fs::write(
            source_root.path().join("service.yml"),
            format!("id: {package_id}\n"),
        )
        .expect("write service manifest");
        fs::write(source_root.path().join("api.yml"), "{}\n").expect("write service API manifest");
        fs::write(
            source_root.path().join("http.yml"),
            "run:\n  method: POST\n  path: /phase-1/contract\n  kind: typedJson\n  handler: main.run\n  adapterArgs:\n    - param: value\n      source: { kind: http.body }\n",
        )
        .expect("write HTTP manifest");
    }
    source_root
}

fn production_sized_limits() -> LinkLimits {
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

#[derive(Debug)]
struct TempRoot(PathBuf);

impl TempRoot {
    fn create(prefix: &str) -> Self {
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

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/linker lives below repository root")
        .to_path_buf()
}
