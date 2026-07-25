use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref};
use skiff_artifact_model::{
    AssemblyIdentity, BoundaryCallbackContract, BoundaryCancellationContract,
    BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, CanonicalPackageLinkPlan, ContractTypeRef,
    PackageArtifact, PackageBinding, PackageCallableId, PackageCodeSlot, PackageRefIr,
    PackageRequirementKey, PackageTypeRequirement, RuntimeAssembly, TestEffectOutcomeIr, TypeRefIr,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    compile_contract, CompilerPlatformSources, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ServiceContractPointer};
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_model::request_heap::RequestHeap;
use skiff_test_runner::{
    canonical_fixture::discover_package_test_cases,
    canonical_package::compile_package_project,
    canonical_std_seed::seed_canonical_std,
    test_overlay::{compile_package_test_overlay, PublishedPackageTestOverlay},
};

use crate::{Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

use super::{activation_context, execution_context, test_runtime, TestResolver};

const ERROR_PACKAGE_ID: &str = "example.com/typed-effect-errors";
const ERROR_PACKAGE_VERSION: &str = "1.0.0";
const ERROR_STABLE_SCHEMA_KEY: &str = "Failure";
const SERVICE_ID: &str = "example.com/typed-effect-payments";
const SERVICE_VERSION: &str = "1.0.0";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn source_inline_service_effect_sequence_typed_throw_is_caught_then_responds() {
    let fixture = TempFixture::new("source-inline-service-typed-throw");
    let platform_sources = repository_platform_sources();
    let artifacts = fixture.child("artifacts");
    seed_canonical_std(&platform_sources, &artifacts).expect("canonical std seed");

    let error_package = fixture.child("errors");
    write_error_package(&error_package);
    build_authoring_object(
        &platform_sources,
        AuthoringObject::Package,
        &error_package,
        &artifacts,
        "dev",
        true,
    )
    .expect("error package publication");

    let store = CanonicalArtifactStore::open(&artifacts).expect("canonical store");
    let error_pointer = store
        .read_package_artifact_pointer(ERROR_PACKAGE_ID, ERROR_PACKAGE_VERSION)
        .expect("error package pointer read")
        .expect("error package pointer");
    let error_artifact = store
        .read_package_artifact(&error_pointer.artifact)
        .expect("error package artifact");
    let error_schema = store
        .resolve_package_artifact_schema(&error_artifact)
        .expect("error package schema");
    let failure_entry = error_schema
        .index
        .types
        .get(ERROR_STABLE_SCHEMA_KEY)
        .expect("public Failure package schema");

    publish_open_error_service_contract(&store, failure_entry.package_schema_type_id.clone());

    let consumer = fixture.child("consumer");
    write_consumer_package(&consumer);
    let project = compile_package_project(&platform_sources, &consumer, &artifacts)
        .expect("consumer source package compile");
    let cases = discover_package_test_cases(&consumer, &consumer, false).expect("test discovery");
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources, &consumer, &artifacts, &project, &cases)
            .expect("source test overlay compile and lower");
    assert_throw_lowered_to_exact_package_symbol(&overlay);
    execute_overlay_case(&store, &overlay, &project.dependency_packages).await;
}

#[tokio::test]
async fn source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable() {
    let fixture = TempFixture::new("source-inline-compiler-owned-std");
    let platform_sources = repository_platform_sources();
    let artifacts = fixture.child("artifacts");
    seed_canonical_std(&platform_sources, &artifacts).expect("canonical std seed");

    let consumer = fixture.child("consumer");
    write_std_effect_consumer_package(&consumer);
    let project = compile_package_project(&platform_sources, &consumer, &artifacts)
        .expect("std effect consumer source package compile");
    let cases = discover_package_test_cases(&consumer, &consumer, false).expect("test discovery");
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources, &consumer, &artifacts, &project, &cases)
            .expect("compiler-owned std effect overlay compile and lower");

    let request_callable = PackageCallableId::new("pkg-callable:skiff.run/std:std.http.request");
    let std_calls = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.expressions)
        .filter(|expression| {
            matches!(
                expression,
                skiff_artifact_model::ExprIr::Call {
                    call:
                        skiff_artifact_model::CallIr {
                            target:
                                skiff_artifact_model::CallTargetIr::PackageCallable {
                                    package_ref:
                                        PackageRefIr::Dependency { dependency_ref },
                                    package_callable_id,
                                },
                            ..
                        },
                } if dependency_ref == "std" && package_callable_id == &request_callable
            )
        })
        .count();
    assert_eq!(
        std_calls, 1,
        "the production call must use the exact std package callable"
    );
    let registrations = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.statements)
        .filter(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::TestEffectRegister {
                    target:
                        skiff_artifact_model::TestEffectRegisterTargetIr::PackageCallable {
                            package_ref: PackageRefIr::Dependency { dependency_ref },
                            callable_id,
                        },
                    ..
                } if dependency_ref == "std" && callable_id == &request_callable
            )
        })
        .count();
    assert_eq!(
        registrations, 1,
        "the setup must register the same exact std package callable"
    );

    let store = CanonicalArtifactStore::open(&artifacts).expect("canonical store");
    execute_overlay_case(&store, &overlay, &project.dependency_packages).await;
}

async fn execute_overlay_case(
    store: &CanonicalArtifactStore,
    overlay: &PublishedPackageTestOverlay,
    dependencies: &[PackageArtifact],
) {
    let mut packages = vec![overlay.overlay.artifact.clone()];
    packages.extend(dependencies.iter().cloned());
    let assembly = package_link_fixture(&packages);
    let overlay_ref =
        package_artifact_ref(&overlay.overlay.artifact).expect("overlay package reference");
    let binding = overlay.bindings.first().expect("one test binding");
    let callable = overlay
        .overlay
        .artifact
        .callable_links
        .get(&binding.callable_id)
        .expect("test callable link");
    let hydrated = hydrate_packages(store, overlay, dependencies);
    let image = crate::test_support::link_package_fixture(assembly.clone(), hydrated);
    let caller_addr = image
        .shared_packages()
        .code_by_build(&overlay_ref.package_build_id)
        .expect("overlay code slot")
        .executable_addr(&callable.target)
        .expect("test callable executable address");

    let activation = activation_context(
        assembly.assembly_identity,
        overlay_ref.package_build_id.clone(),
    );
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request =
        RequestActivationContext::begin(activation).expect("test request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("linked source overlay should form an eval target");
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context = execution_context(&interpreter, eval_target);
    let mut heap = RequestHeap::default();

    interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &caller_addr, Vec::new())
        .await
        .expect("the first typed throw must be caught and the second response must be returned");
    interpreter
        .finalize_test_case()
        .expect("both ordered service effect outcomes must be consumed");
}

fn publish_open_error_service_contract(
    store: &CanonicalArtifactStore,
    failure_type_id: skiff_artifact_model::PackageSchemaTypeId,
) {
    let contract = compile_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: SERVICE_VERSION.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "value".to_string(),
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                stream: BoundaryStreamContract::Unary,
                cancellation: BoundaryCancellationContract::Cooperative,
                callbacks: BoundaryCallbackContract::None,
                may_suspend: true,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        )]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: ERROR_PACKAGE_ID.to_string(),
            required_type_ids: vec![failure_type_id],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "open error effect payments".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("open service error channel contract compile");
    let reference =
        service_contract_ref(&contract).expect("open service error channel contract reference");
    store
        .write_service_contract(&contract)
        .expect("open service error channel contract record");
    let pointer = ServiceContractPointer::new(reference)
        .expect("open service error channel contract pointer");
    store
        .compare_and_swap_service_contract_pointer(None, &pointer)
        .expect("open service error channel contract pointer publication");
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn assert_throw_lowered_to_exact_package_symbol(overlay: &PublishedPackageTestOverlay) {
    let payload_types = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.statements)
        .filter_map(|statement| {
            let skiff_artifact_model::StmtIr::TestEffectRegister {
                outcome: TestEffectOutcomeIr::Throw { payload_type, .. },
                ..
            } = statement
            else {
                return None;
            };
            Some(payload_type)
        })
        .collect::<Vec<_>>();
    assert_eq!(payload_types.len(), 1);
    assert!(matches!(
        payload_types[0],
        TypeRefIr::PackageSymbol { symbol }
            if symbol.package
                == (PackageRefIr::PackageId {
                    package_id: ERROR_PACKAGE_ID.to_string(),
                })
                && symbol.symbol_path == ERROR_STABLE_SCHEMA_KEY
    ));
}

fn package_link_fixture(packages: &[PackageArtifact]) -> RuntimeAssembly {
    let references = packages
        .iter()
        .map(|package| package_artifact_ref(package).expect("package reference"))
        .collect::<Vec<_>>();
    let by_coordinate = packages
        .iter()
        .map(|package| {
            (
                (
                    package.package_id.as_str(),
                    package.package_version.as_str(),
                ),
                package,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let package_links = packages
        .iter()
        .flat_map(|caller| {
            caller
                .package_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| {
            let dependency = by_coordinate
                .get(&(
                    requirement.package_id.as_str(),
                    requirement.exact_version.as_str(),
                ))
                .expect("exact package dependency in link closure");
            assert_eq!(
                dependency.package_local_abi.local_abi_identity,
                requirement.expected_local_abi
            );
            PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: package_artifact_ref(dependency).expect("dependency package reference"),
            }
        })
        .collect();
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("test-fixture:source-inline-service-typed-throw"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: references.clone(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: references
                .into_iter()
                .map(|package| PackageCodeSlot { package })
                .collect(),
            package_links,
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    }
}

fn hydrate_packages(
    store: &CanonicalArtifactStore,
    overlay: &PublishedPackageTestOverlay,
    dependencies: &[PackageArtifact],
) -> Vec<(PackageArtifact, Vec<skiff_artifact_model::FileIrUnit>)> {
    let mut hydrated = vec![(
        overlay.overlay.artifact.clone(),
        overlay
            .overlay
            .file_ir_units
            .iter()
            .map(|file| file.unit.clone())
            .collect(),
    )];
    hydrated.extend(dependencies.iter().map(|package| {
        let reference = package_artifact_ref(package).expect("dependency package reference");
        let files = package
            .files
            .iter()
            .map(|file| {
                store
                    .read_file_ir(&reference, file)
                    .expect("dependency File IR")
                    .as_ref()
                    .clone()
            })
            .collect();
        (package.clone(), files)
    }));
    hydrated
}

fn write_error_package(root: &Path) {
    fs::create_dir_all(root).expect("error package directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {ERROR_PACKAGE_ID}\nversion: {ERROR_PACKAGE_VERSION}\n"),
    )
    .expect("error package manifest");
    fs::write(root.join("api.yml"), "Failure: main.Failure\n").expect("error package API");
    fs::write(
        root.join("main.skiff"),
        r#"type Failure {
  message: string,
}
"#,
    )
    .expect("error package source");
}

fn write_consumer_package(root: &Path) {
    fs::create_dir_all(root).expect("consumer directory");
    fs::write(
        root.join("package.yml"),
        format!(
            r#"id: example.com/typed-effect-consumer
version: 1.0.0
packages:
  - id: {ERROR_PACKAGE_ID}
    version: {ERROR_PACKAGE_VERSION}
    alias: errors
services:
  - id: {SERVICE_ID}
    version: {SERVICE_VERSION}
    alias: payments
"#
        ),
    )
    .expect("consumer manifest");
    fs::write(root.join("api.yml"), "").expect("consumer API");
    fs::write(
        root.join("main.skiff"),
        r#"import errors

function exercise() -> string {
  const first = catch<errors.Failure>(payments/echo("first"))
  if first.tag == "ok" {
    return "typed-throw-was-not-caught"
  }
  return payments/echo("second")
}
"#,
    )
    .expect("consumer source");
    fs::write(
        root.join("main.test.skiff"),
        r#"import errors

test "typed service throw is caught before sequence response" effects {
  payments/echo {
    sequence: [
      {
        expect: "first",
        throw: errors.Failure { message: "denied" },
      },
      {
        expect: "second",
        respond: "accepted",
      },
    ],
  }
} {
  assert root.main.exercise() == "accepted"
}
"#,
    )
    .expect("consumer test source");
}

fn write_std_effect_consumer_package(root: &Path) {
    fs::create_dir_all(root).expect("std effect consumer directory");
    fs::write(
        root.join("package.yml"),
        "id: example.com/std-effect-consumer\nversion: 1.0.0\n",
    )
    .expect("std effect consumer manifest");
    fs::write(root.join("api.yml"), "").expect("std effect consumer API");
    fs::write(
        root.join("main.skiff"),
        r#"import std

function fetchStatus() -> integer {
  const response = std.http.request(std.http.HttpClientRequest {
    method: "GET",
    url: "https://must-not-run.invalid/resource",
    headers: Array.empty<std.http.HttpHeader>(),
    body: null,
    timeoutMs: null,
  })
  return response.status
}
"#,
    )
    .expect("std effect consumer source");
    fs::write(
        root.join("main.test.skiff"),
        r#"import std

test "compiler-owned std request is replaced by exact package identity" effects {
  std/http.request {
    expect: {
      method: "GET",
      url: "https://must-not-run.invalid/resource",
    },
    respond: std.http.HttpClientResponse {
      status: 204,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8(""),
    },
  }
} {
  assert root.main.fetchStatus() == 204
}
"#,
    )
    .expect("std effect consumer test source");
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval must live two levels below the Skiff root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary fixture root");
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
