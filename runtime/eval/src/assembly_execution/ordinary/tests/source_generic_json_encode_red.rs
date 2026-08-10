use crate::heap_access::HeapAccess;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::{
    AssemblyIdentity, CallTargetIr, CanonicalPackageLinkPlan, ExprIr, PackageArtifact,
    PackageBinding, PackageCodeSlot, PackageRequirementKey, PackageSchemaTypeId,
    PackageSchemaTypeRecord, RuntimeAssembly, TypeRefIr, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler::{
    authoring::publish_package_artifact_records, CompilerPlatformSources, PublishedPackageArtifact,
};
use skiff_deployment::storage::{CanonicalArtifactStore, PackageArtifactPointer};
use skiff_runtime_activation::{ActivationContext, RequestActivationContext};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, HydratedPackageCode, LinkedCallTarget, LinkedExprIr,
    LinkedTypeRef, PublicationResourceTable,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use skiff_test_runner::{
    canonical_package::{compile_package_project, CanonicalPackageProject},
    canonical_std_seed::seed_canonical_std,
};

use super::{activation_context, execution_context_with_trace, test_runtime, TestResolver};
use crate::{Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

const CONSUMER_ID: &str = "example.com/generic-json-encode-red";
const MODEL_ID: &str = "example.com/generic-json-encode-model";

#[test]
fn compiler_linked_generic_std_json_encode_closes_the_concrete_runtime_plan() {
    std::thread::Builder::new()
        .name("generic-json-encode-red".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("generic JSON Tokio runtime")
                .block_on(run_compiler_linked_generic_std_json_encode_red());
        })
        .expect("generic JSON test thread")
        .join()
        .expect("generic JSON test thread must finish");
}

async fn run_compiler_linked_generic_std_json_encode_red() {
    let fixture = SourceFixture::new();
    let platform_sources = repository_platform_sources();
    let store =
        CanonicalArtifactStore::create(&fixture.artifact_root).expect("isolated artifact store");
    seed_canonical_std(&platform_sources, &fixture.artifact_root).expect("canonical std seed");

    write_model_package(&fixture.model_root);
    let model = compile_package_project(
        &platform_sources,
        &fixture.model_root,
        &fixture.artifact_root,
    )
    .expect("model package must compile before its consumer");
    let model_receipt = publish_package_artifact_records(store.root(), &model.package)
        .expect("model package canonical records");
    let model_pointer =
        PackageArtifactPointer::new(model_receipt.artifact).expect("model package pointer");
    store
        .compare_and_swap_package_artifact_pointer(None, &model_pointer)
        .expect("model package pointer publication");

    write_consumer_package(&fixture.consumer_root);
    let project = compile_package_project(
        &platform_sources,
        &fixture.consumer_root,
        &fixture.artifact_root,
    )
    .expect("generic JSON consumer must compile through the production package pipeline");
    assert_compiler_generic_chain(&project);

    let linked = LinkedFixture::new(&store, &project);
    assert_eq!(
        linked.execute("concreteEncodeControl").await.unwrap(),
        RuntimeValue::String("concrete".to_string()),
        "direct concrete std.json.encode must remain a GREEN control",
    );
    assert_eq!(
        linked.execute("genericDecodeControl").await.unwrap(),
        RuntimeValue::String("decoded".to_string()),
        "the existing generic std.json.decode substitution path must remain a GREEN control",
    );
    assert_eq!(
        linked.execute("resourceCatch").await.unwrap(),
        RuntimeValue::String("missing-package-resource.txt".to_string()),
        "PackageDirect std.resource.text must materialize and catch the exact public ResourceError",
    );
    assert_eq!(
        linked.execute("leftValue").await.unwrap(),
        RuntimeValue::String("left".to_string()),
        "a zero-argument public-instance method must receive its exact const receiver",
    );
    assert_eq!(
        linked.execute("rightValue").await.unwrap(),
        RuntimeValue::String("right".to_string()),
        "two public consts sharing one impl must not share receiver values",
    );
    assert_eq!(
        linked.execute("leftTag").await.unwrap(),
        RuntimeValue::String("left-tag".to_string()),
        "one public instance must dispatch methods from each listed interface",
    );
    assert_eq!(
        linked.execute("leftItems").await.unwrap(),
        RuntimeValue::String("leftleft".to_string()),
        "a generic package-direct public-instance stream must inject self and complete normally",
    );
    assert_eq!(
        linked.execute("leftItemsDeferred").await.unwrap(),
        RuntimeValue::String("leftleft".to_string()),
        "an assembly dynamic-self stream must detach from-values before deferred consumption",
    );

    let mut failures = Vec::new();
    for symbol in ["encodeLocal", "encodePackage", "encodeNested"] {
        if let Err(error) = linked.execute(symbol).await {
            failures.push(format!("{symbol}: {error}"));
        }
    }
    assert!(
        failures.is_empty(),
        "compiler-linked generic std.json.encode must close T before native dispatch; \
         observed failures:\n{}",
        failures.join("\n")
    );
}

fn assert_compiler_generic_chain(project: &CanonicalPackageProject) {
    let main = project
        .package
        .file_ir_units
        .iter()
        .find(|file| file.unit.module_path == "main")
        .expect("compiled main File IR");
    let wrapper = main
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol.ends_with(".encodeJson"))
        .expect("compiled generic encodeJson wrapper");
    assert_eq!(wrapper.type_params, ["T"]);
    let native_encode = wrapper
        .body
        .expressions
        .iter()
        .find_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            let is_encode = match &call.target {
                CallTargetIr::Native { target } => {
                    target.binding_key.as_deref() == Some("std.json.encode")
                }
                CallTargetIr::PackageCallable {
                    package_callable_id,
                    ..
                } => package_callable_id.as_str()
                    == "pkg-callable:skiff.run/std:std.json.encode",
                _ => false,
            };
            is_encode.then_some(call)
        })
        .unwrap_or_else(|| {
            panic!(
                "generic wrapper must contain the compiler-owned std.json.encode native call: {:#?}",
                wrapper.body.expressions
            )
        });
    assert_eq!(
        native_encode.type_args.get("T0"),
        Some(&TypeRefIr::TypeParam {
            name: "T".to_string()
        }),
        "File IR must preserve the wrapper type parameter for runtime substitution",
    );

    let package_entry = main
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol.ends_with(".encodePackage"))
        .expect("compiled package-symbol entry");
    let package_type_arg = package_entry
        .body
        .expressions
        .iter()
        .filter_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            call.type_args.get("T0")
        })
        .find(|ty| {
            matches!(
                ty,
                TypeRefIr::PackageSymbol { .. } | TypeRefIr::PackageSchema { .. }
            )
        })
        .expect("package entry must pass an exact package-owned nominal T");
    assert!(
        matches!(package_type_arg, TypeRefIr::PackageSymbol { .. }),
        "the generic caller must preserve the exact package symbol T; explicit codec \
         canonicalization happens only after runtime substitution",
    );
}

struct LinkedFixture {
    image: Arc<AssemblyExecutionImage>,
    activation: Arc<ActivationContext>,
    package_build_id: skiff_artifact_model::PackageBuildId,
}

impl LinkedFixture {
    fn new(store: &CanonicalArtifactStore, project: &CanonicalPackageProject) -> Self {
        let mut packages = vec![project.package.artifact.clone()];
        packages.extend(project.dependency_packages.iter().cloned());
        let assembly = package_link_fixture(&packages);
        let hydrated = std::iter::once(hydrate_published_package(&project.package))
            .chain(
                project
                    .dependency_packages
                    .iter()
                    .map(|package| hydrate_package(store, package)),
            )
            .collect::<Vec<_>>();
        let image =
            skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
                .expect("compiler-produced package closure must link");
        let package_build_id = project.package.artifact.package_build_id.clone();
        assert_linked_runtime_chain(&image, &package_build_id);
        let activation =
            activation_context(assembly.assembly_identity.clone(), package_build_id.clone());
        Self {
            image,
            activation,
            package_build_id,
        }
    }

    async fn execute(&self, symbol: &str) -> crate::error::Result<RuntimeValue> {
        let code = self
            .image
            .code_by_build(&self.package_build_id)
            .expect("consumer linked code slot");
        let (file_index, executable_index) = code
            .files()
            .iter()
            .enumerate()
            .find_map(|(file_index, file)| {
                file.executables
                    .iter()
                    .position(|executable| executable.symbol.ends_with(&format!(".{symbol}")))
                    .map(|executable_index| (file_index, executable_index))
            })
            .unwrap_or_else(|| panic!("linked executable {symbol}"));
        let request = RequestActivationContext::begin(Arc::clone(&self.activation))
            .expect("generic JSON request generation");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
            activation: Arc::clone(&self.activation),
        });
        let target = RuntimeAssemblyEvalTarget::new(Arc::clone(&self.image), request, resolver)
            .expect("linked generic JSON image and activation");
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let context =
            execution_context_with_trace(&interpreter, target, "trace:resource-package-direct");
        let heap = RequestHeap::default();
        interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut HeapAccess::private(heap),
                &ExecutableAddr::package(0, file_index, executable_index),
                Vec::new(),
            )
            .await
    }
}

fn assert_linked_runtime_chain(
    image: &AssemblyExecutionImage,
    package_build_id: &skiff_artifact_model::PackageBuildId,
) {
    let code = image
        .code_by_build(package_build_id)
        .expect("consumer linked code");
    let wrapper = code
        .files()
        .iter()
        .flat_map(|file| &file.executables)
        .find(|executable| executable.symbol.ends_with(".encodeJson"))
        .expect("linked generic encodeJson wrapper");
    let package_call = wrapper
        .body
        .expressions
        .iter()
        .find_map(|expression| {
            let LinkedExprIr::Call { call } = expression else {
                return None;
            };
            let LinkedCallTarget::PackageDirect { call: direct } = &call.target else {
                return None;
            };
            (direct.package_callable_id().as_str() == "pkg-callable:skiff.run/std:std.json.encode")
                .then_some((call, direct))
        })
        .expect("linker must retain the exact std package callable");
    assert_eq!(
        package_call.0.type_args.get("T0"),
        Some(&LinkedTypeRef::TypeParam {
            name: "T".to_string()
        }),
        "consumer wrapper must hand T to the std package wrapper",
    );
    let std_executable = image
        .executable_at(package_call.1.executable_addr())
        .expect("linked std encode executable");
    let std_wrapper = std_executable.executable();
    let native_call = std_wrapper
        .body
        .expressions
        .iter()
        .find_map(|expression| {
            let LinkedExprIr::Call { call } = expression else {
                return None;
            };
            let LinkedCallTarget::Native { target } = &call.target else {
                return None;
            };
            (target.binding_key.as_deref() == Some("std.json.encode")).then_some(call)
        })
        .expect("std package wrapper must dispatch the exact native target");
    assert_eq!(
        native_call.type_args.get("T0"),
        Some(&LinkedTypeRef::TypeParam {
            name: "T".to_string()
        }),
        "std native wrapper must preserve T for Eval substitution",
    );
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
                .expect("exact dependency in compiler-produced closure");
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
        assembly_identity: AssemblyIdentity::new("test:generic-json-encode-red"),
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
        gateway_ingress: Vec::new(),
    }
}

fn hydrate_package(
    store: &CanonicalArtifactStore,
    package: &PackageArtifact,
) -> HydratedPackageCode {
    let reference = package_artifact_ref(package).expect("hydrated package reference");
    let files = package
        .files
        .iter()
        .map(|file| {
            store
                .read_file_ir(&reference, file)
                .expect("compiler-produced dependency File IR")
        })
        .collect();
    let schema = store
        .resolve_package_artifact_schema(package)
        .expect("compiler-produced package schema");
    let records = schema
        .records
        .iter()
        .map(
            |(type_id, record): (&PackageSchemaTypeId, &Arc<PackageSchemaTypeRecord>)| {
                (type_id.clone(), Arc::clone(record))
            },
        )
        .collect();
    HydratedPackageCode::new(
        Arc::new(package.clone()),
        files,
        PublicationResourceTable::default(),
    )
    .with_schema_index(schema.index)
    .with_schema_records(records)
}

fn hydrate_published_package(package: &PublishedPackageArtifact) -> HydratedPackageCode {
    let records = package
        .resolved_package_schema_type_records
        .iter()
        .map(|(type_id, record)| (type_id.clone(), Arc::new(record.clone())))
        .collect();
    HydratedPackageCode::new(
        Arc::new(package.artifact.clone()),
        package
            .file_ir_units
            .iter()
            .map(|file| Arc::new(file.unit.clone()))
            .collect(),
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(package.package_schema_index.clone()))
    .with_schema_records(records)
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval lives below repository root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

fn write_model_package(root: &Path) {
    fs::create_dir_all(root).expect("model package root");
    fs::write(
        root.join("package.yml"),
        format!("id: {MODEL_ID}\nversion: 1.0.0\n"),
    )
    .expect("model package manifest");
    fs::write(
        root.join("api.yml"),
        r#"PublicPayload: main.PublicPayload
makePublic: main.makePublic
left:
  const: root.main.left
  interfaces:
    - root.main.ValueApi
    - root.main.TagApi
right:
  const: root.main.right
  interfaces:
    - root.main.ValueApi
"#,
    )
    .expect("model package API");
    fs::write(
        root.join("main.skiff"),
        r#"type PublicPayload {
  id: string,
  count: integer,
}

function makePublic() -> PublicPayload {
  return PublicPayload { id: "package", count: 2 }
}

interface ValueApi {
  function value(self: Self) -> string
  function items(self: Self) -> Stream<string>
}

interface TagApi {
  function tag(self: Self) -> string
}

type Named<T> implements ValueApi, TagApi {
  payload: T,
  valueLabel: string,
}

impl Named<T> {
  function value() -> string {
    return self.valueLabel
  }

  function items() -> Stream<string> {
    emit(self.valueLabel)
    emit(self.valueLabel)
    return null
  }

  function tag() -> string {
    return self.valueLabel.concat("-tag")
  }
}

const left: Named<integer> = Named<integer> { payload: 1, valueLabel: "left" }
const right: Named<boolean> = Named<boolean> { payload: true, valueLabel: "right" }
"#,
    )
    .expect("model package source");
}
fn write_consumer_package(root: &Path) {
    fs::create_dir_all(root).expect("consumer package root");
    fs::write(
        root.join("package.yml"),
        format!(
            "id: {CONSUMER_ID}\nversion: 1.0.0\npackages:\n  - id: {MODEL_ID}\n    version: 1.0.0\n    alias: models\n"
        ),
    )
    .expect("consumer package manifest");
    fs::write(
        root.join("api.yml"),
        "encodeLocal: main.encodeLocal\nencodePackage: main.encodePackage\nencodeNested: main.encodeNested\nconcreteEncodeControl: main.concreteEncodeControl\ngenericDecodeControl: main.genericDecodeControl\nresourceCatch: main.resourceCatch\nleftValue: main.leftValue\nrightValue: main.rightValue\nleftTag: main.leftTag\nleftItems: main.leftItems\nleftItemsDeferred: main.leftItemsDeferred\n",
    )
    .expect("consumer package API");
    fs::write(
        root.join("main.skiff"),
        r#"import models

interface LocalStreamSource {
  function items(self: Self) -> Stream<string>
}

type LocalStreamValue implements LocalStreamSource {
  label: string,
}

impl LocalStreamValue {
  function items() -> Stream<string> {
    emit(self.label)
    emit(self.label)
    return null
  }
}

type LocalPayload {
  label: string,
  count: integer,
}

function encodeJson<T>(value: T) -> Json {
  return std.json.decode<Json>(std.json.encode<T>(value))
}

function genericDecode<T>(input: string) -> T {
  return std.json.decode<T>(input)
}

function encodeLocal() -> Json {
  return encodeJson<LocalPayload>(LocalPayload { label: "local", count: 1 })
}

function encodePackage() -> Json {
  return encodeJson<models.PublicPayload>(models/makePublic())
}

function encodeNested() -> Json {
  var items = Array.empty<LocalPayload>()
  items.push(LocalPayload { label: "nested", count: 3 })
  return encodeJson<Array<LocalPayload>>(items)
}

function concreteEncodeControl() -> Json {
  return std.json.decode<Json>(std.json.encode<string>("concrete"))
}

function genericDecodeControl() -> Json {
  return genericDecode<Json>("\"decoded\"")
}

function resourceCatch() -> string {
  let result = catch<std.resource.ResourceError>(std.resource.text("missing-package-resource.txt"))
  if result.tag == "ok" {
    return "resource-error-was-not-caught"
  }
  return result.exception.error.path
}

function leftValue() -> string {
  return models/left.value()
}

function rightValue() -> string {
  return models/right.value()
}

function leftTag() -> string {
  return models/left.tag()
}

function leftItems() -> string {
  var output = ""
  for item in models/left.items() {
    output = output.concat(item)
  }
  return output
}

function leftItemsDeferred() -> string {
  let value: any LocalStreamSource = LocalStreamValue { label: "left" } as LocalStreamSource
  let source: Stream<string> = value.items()
  var output = ""
  for item in source {
    output = output.concat(item)
  }
  return output
}
"#,
    )
    .expect("consumer package source");
}

struct SourceFixture {
    root: PathBuf,
    model_root: PathBuf,
    consumer_root: PathBuf,
    artifact_root: PathBuf,
}

impl SourceFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-generic-json-encode-red-{}-{nonce}",
            std::process::id()
        ));
        Self {
            model_root: root.join("model"),
            consumer_root: root.join("consumer"),
            artifact_root: root.join("artifacts"),
            root,
        }
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
