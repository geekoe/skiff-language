use crate::heap_access::HeapAccess;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, ExprIr, PackageCodeSlot, PackageSchemaTypeId,
    PackageSchemaTypeRecord, RuntimeAssembly, TypeRefIr, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, HydratedPackageCode, LinkedExprIr, LinkedTypeRef,
    PublicationResourceTable, UnitAddr,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use skiff_test_runner::canonical_package::compile_package_project;

use super::{activation_context, execution_context, test_runtime, TestResolver};
use crate::{Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

#[test]
fn compiler_wrap_continues_through_file_ir_linking_and_eval() {
    let fixture = CombinedSourceFixture::new();
    let repository_root = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("eval manifest has a runtime parent")
            .parent()
            .expect("runtime has a repository parent"),
    )
    .expect("repository root");
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    CanonicalArtifactStore::create(&fixture.artifact_root).expect("isolated artifact root");
    let project = compile_package_project(
        &platform_sources,
        &fixture.package_root,
        &fixture.artifact_root,
    )
    .expect("representation source should compile through the production package pipeline");
    let published = project.package;

    let source_file = published
        .file_ir_units
        .iter()
        .find(|file| file.unit.module_path == "main")
        .expect("compiled main File IR");
    let source_executable = source_file
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol.ends_with(".make"))
        .expect("compiled make executable");
    let source_wraps = source_executable
        .body
        .expressions
        .iter()
        .enumerate()
        .filter_map(|(index, expression)| match expression {
            ExprIr::RepresentationWrap { value, type_ref } => {
                Some((index, *value, type_ref.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        source_wraps.len(),
        1,
        "one explicit source constructor must produce one File IR wrap"
    );
    let (source_wrap_index, source_payload, source_target) = &source_wraps[0];
    assert!(matches!(
        source_executable
            .body
            .expressions
            .get(source_payload.expression as usize),
        Some(ExprIr::Call { .. })
    ));
    assert_eq!(
        source_executable
            .body
            .expressions
            .iter()
            .filter(|expression| matches!(expression, ExprIr::Call { .. }))
            .count(),
        1,
        "the constructor payload call must occur exactly once"
    );
    assert!(
        (source_payload.expression as usize) < *source_wrap_index,
        "the wrap must consume the already-lowered payload"
    );

    let package_ref =
        package_artifact_ref(&published.artifact).expect("compiled package reference");
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("representation-combined-probe"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let schema_records = published
        .resolved_package_schema_type_records
        .iter()
        .map(|(identity, record)| (identity.clone(), Arc::new(record.clone())))
        .collect::<BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>>();
    let hydrated = HydratedPackageCode::new(
        Arc::new(published.artifact.clone()),
        published
            .file_ir_units
            .iter()
            .map(|file| Arc::new(file.unit.clone()))
            .collect(),
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(published.package_schema_index.clone()))
    .with_schema_records(schema_records);
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, [hydrated])
            .expect("compiler-produced File IR should link through the canonical package image");

    let linked_code = image
        .code_by_build(&published.artifact.package_build_id)
        .expect("compiled package has one linked code slot");
    let linked_file_index = linked_code
        .files()
        .iter()
        .position(|file| file.file_ir_identity == source_file.unit.file_ir_identity)
        .expect("linked main file");
    let linked_file = &linked_code.files()[linked_file_index];
    let linked_executable_index = linked_file
        .executables
        .iter()
        .position(|executable| executable.symbol.ends_with(".make"))
        .expect("linked make executable");
    let linked_executable = &linked_file.executables[linked_executable_index];
    let linked_wraps = linked_executable
        .body
        .expressions
        .iter()
        .filter_map(|expression| match expression {
            LinkedExprIr::RepresentationWrap { value, type_ref } => {
                Some((*value, type_ref.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(linked_wraps.len(), 1);
    assert_eq!(
        linked_wraps[0].0.expression, source_payload.expression,
        "linking must preserve the exact child reference"
    );
    let TypeRefIr::LocalType { type_index } = source_target else {
        panic!("same-file compiler target must remain an exact local nominal: {source_target:#?}");
    };
    let LinkedTypeRef::Address {
        addr: linked_target,
    } = &linked_wraps[0].1
    else {
        panic!(
            "linking must resolve the local nominal to one exact address: {:#?}",
            linked_wraps[0].1
        );
    };
    assert_eq!(linked_target.unit, UnitAddr::Package(0));
    assert_eq!(linked_target.file, FileAddr::loaded_file(linked_file_index));
    assert_eq!(linked_target.type_index, *type_index as usize);

    let activation = activation_context(
        assembly.assembly_identity.clone(),
        published.artifact.package_build_id.clone(),
    );
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("combined probe request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(Arc::clone(&image), request, resolver)
        .expect("linked image and activation should form an eval target");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, eval_target);
    let mut heap = RequestHeap::default();
    let addr = ExecutableAddr::package(0, linked_file_index, linked_executable_index);
    let value = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("combined probe Tokio runtime")
        .block_on(interpreter.execute_runtime_assembly_addr(
            context,
            &mut HeapAccess::Exclusive(&mut heap),
            &addr,
            Vec::new(),
        ))
        .expect("compiler-produced linked wrap should evaluate");

    assert_eq!(
        value,
        RuntimeValue::String("payload".to_string()),
        "eval must preserve the raw payload value"
    );
}

struct CombinedSourceFixture {
    root: PathBuf,
    package_root: PathBuf,
    artifact_root: PathBuf,
}

impl CombinedSourceFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-representation-combined-{}-{nonce}",
            std::process::id()
        ));
        let package_root = root.join("package");
        let artifact_root = root.join("artifacts");
        fs::create_dir_all(&package_root).expect("combined probe package root");
        fs::write(
            package_root.join("package.yml"),
            "id: example.com/representation-combined-probe\nversion: 1.0.0\n",
        )
        .expect("combined probe manifest");
        fs::write(
            package_root.join("api.yml"),
            "Token: main.Token\nmake: main.make\n",
        )
        .expect("combined probe API");
        fs::write(
            package_root.join("main.skiff"),
            r#"
type Token = string

function payload(value: string) -> string {
  return value
}

function make() -> Token {
  return Token(payload("payload"))
}
"#,
        )
        .expect("combined probe source");
        Self {
            root,
            package_root,
            artifact_root,
        }
    }
}

impl Drop for CombinedSourceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
