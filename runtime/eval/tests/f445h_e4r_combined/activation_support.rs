use super::{common::VERSION, imports::*};

static ACTIVATION_INSTRUCTION: OnceLock<ActivationRelativeServiceCall> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn linked_activation_instruction() -> ActivationRelativeServiceCall {
    ACTIVATION_INSTRUCTION
        .get_or_init(compile_activation_instruction)
        .clone()
}

fn compile_activation_instruction() -> ActivationRelativeServiceCall {
    const PROVIDER_PACKAGE: &str = "example.com/f445h-e4r-provider";
    const PROVIDER_SERVICE: &str = "example.com/f445h-e4r-provider";
    const CONSUMER_PACKAGE: &str = "example.com/f445h-e4r-consumer-package";
    const CONSUMER_SERVICE: &str = "example.com/f445h-e4r-consumer";

    let temp = TempFixture::new("f445h-e4r-activation-link");
    let artifact_root = temp.child("artifacts");
    let platform = repository_platform_sources();
    seed_canonical_std(&platform, &artifact_root).expect("combined canonical std seed");

    let provider_root = temp.child("provider");
    write_service_source(
        &provider_root,
        PROVIDER_PACKAGE,
        PROVIDER_SERVICE,
        "",
        "ready: main.ready\n",
        "serviceCalls:\n  - ready\n",
        "function ready() -> integer { return 7 }\n",
    );
    let provider = build_service(&platform, &provider_root, &artifact_root);

    let consumer_root = temp.child("consumer");
    write_service_source(
        &consumer_root,
        CONSUMER_PACKAGE,
        CONSUMER_SERVICE,
        &format!(
            "services:\n  - id: {PROVIDER_SERVICE}\n    version: {VERSION}\n    alias: provider\n"
        ),
        "{}\n",
        "serviceCalls: []\n",
        "function callProvider() -> integer { return provider/ready() }\n",
    );
    let consumer = build_service(&platform, &consumer_root, &artifact_root);

    let store = CanonicalArtifactStore::open(&artifact_root).expect("combined artifact store");
    let deployments = [&provider.deployment, &consumer.deployment]
        .iter()
        .map(|reference| {
            store
                .read_service_deployment(reference)
                .expect("combined service deployment")
        })
        .collect::<Vec<_>>();
    let contracts = [&provider.contract, &consumer.contract]
        .iter()
        .map(|reference| {
            store
                .read_service_contract(reference)
                .expect("combined service contract")
        })
        .collect::<Vec<_>>();
    let mut package_refs = BTreeMap::from([
        (
            provider.package.package_build_id.clone(),
            provider.package.clone(),
        ),
        (
            consumer.package.package_build_id.clone(),
            consumer.package.clone(),
        ),
    ]);
    for deployment in &deployments {
        for binding in &deployment.package_bindings {
            package_refs.insert(
                binding.package.package_build_id.clone(),
                binding.package.clone(),
            );
        }
    }
    let packages = package_refs
        .values()
        .map(|reference| {
            store
                .read_package_artifact(reference)
                .expect("combined package closure")
        })
        .collect::<Vec<_>>();
    let roots = vec![
        service_deployment_ref(&deployments[0]),
        service_deployment_ref(&deployments[1]),
    ];
    let deployment_values = deployments
        .iter()
        .map(|deployment| deployment.as_ref().clone())
        .collect::<Vec<_>>();
    let contract_values = contracts
        .iter()
        .map(|contract| contract.as_ref().clone())
        .collect::<Vec<_>>();
    let package_values = packages
        .iter()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let assembly = resolve_runtime_assembly(
        &roots,
        &deployment_values,
        &contract_values,
        &package_values,
    )
    .expect("combined runtime assembly");
    let hydrated = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| hydrate_package(&store, &slot.package))
        .collect::<Vec<_>>();
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
            .expect("combined linked execution image");
    let code = image
        .code_by_build(&consumer.package.package_build_id)
        .expect("combined consumer linked code");
    let mut instructions = code
        .files()
        .iter()
        .flat_map(|file| &file.executables)
        .flat_map(|executable| &executable.body.expressions)
        .filter_map(|expression| match expression {
            LinkedExprIr::Call {
                call:
                    CallIr {
                        target: LinkedCallTarget::ActivationRelativeService { instruction },
                        ..
                    },
            } => Some(instruction.clone()),
            _ => None,
        });
    let instruction = instructions
        .next()
        .expect("combined consumer activation-relative instruction");
    assert!(
        instructions.next().is_none(),
        "combined consumer fixture has one activation-relative instruction"
    );
    instruction
}

struct BuiltService {
    package: skiff_artifact_model::PackageArtifactRef,
    deployment: ServiceDeploymentRef,
    contract: ServiceContractRef,
}

fn build_service(
    platform: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
) -> BuiltService {
    let output = build_authoring_object(
        platform,
        AuthoringObject::Package,
        root,
        artifact_root,
        "dev",
        true,
    )
    .expect("combined service authoring");
    BuiltService {
        package: serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
            .expect("combined package artifact ref"),
        deployment: serde_json::from_value(
            output["serviceDeploymentReceipt"]["deployment"].clone(),
        )
        .expect("combined service deployment ref"),
        contract: serde_json::from_value(output["serviceContractReceipt"]["contract"].clone())
            .expect("combined service contract ref"),
    }
}

fn write_service_source(
    root: &Path,
    package_id: &str,
    service_id: &str,
    dependency_yaml: &str,
    api: &str,
    service_calls: &str,
    source: &str,
) {
    fs::create_dir_all(root).expect("combined service source directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {package_id}\nversion: {VERSION}\n{dependency_yaml}"),
    )
    .expect("combined package manifest");
    fs::write(root.join("api.yml"), api).expect("combined API manifest");
    fs::write(
        root.join("service.yml"),
        format!("id: {service_id}\n{service_calls}"),
    )
    .expect("combined service manifest");
    fs::write(
        root.join("config.dev.yml"),
        "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:f445h-e4r\nlifecycle: { maxConcurrency: 1 }\n",
    )
    .expect("combined service config");
    fs::write(root.join("main.skiff"), source).expect("combined Skiff source");
}

fn hydrate_package(
    store: &CanonicalArtifactStore,
    reference: &skiff_artifact_model::PackageArtifactRef,
) -> HydratedPackageCode {
    let artifact = store
        .read_package_artifact(reference)
        .expect("combined package artifact");
    let files = artifact
        .files
        .iter()
        .map(|file| {
            store
                .read_file_ir(reference, file)
                .expect("combined File IR")
        })
        .collect::<Vec<_>>();
    let schema_index = Arc::new(PackageSchemaIndex {
        package_id: artifact.package_schema_index.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    });
    HydratedPackageCode::new(artifact, files, PublicationResourceTable::default())
        .with_schema_index(schema_index)
}

fn service_deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval lives below repository root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("combined compiler platform sources")
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("combined test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("combined temp fixture root");
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
