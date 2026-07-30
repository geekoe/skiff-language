mod common;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    CallableEffectSummary, ContractTypeRef, PackageConfigAccess,
};
use skiff_compiler::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_package_project_with_contract_dependencies,
    },
    TestDir,
};

#[test]
fn type_import_file_ir_lane_exposes_compiled_types_and_effects() {
    let temp = TestDir::new("skiff-compiler", "shared-fixture-type-lane");
    write_representative_package_project(&temp);

    let project = compile_package_project(temp.path()).expect("package project should compile");
    let main = module_artifact(&project.package, "main");

    assert!(main.unit.declarations.types.contains_key("Result"));
    assert!(main
        .unit
        .external_refs
        .package_symbols
        .iter()
        .any(|symbol| {
            symbol.symbol_path == "Message"
                && matches!(
                    &symbol.package,
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                        if dependency_ref == "dep"
                )
        }));
    assert!(project
        .package
        .artifact
        .callable_semantic_facts
        .values()
        .any(|facts| matches!(facts.effects, CallableEffectSummary::Analyzed { .. })));
    assert!(project
        .dependency("example.com/probe-dependency", "1.0.0")
        .is_some());
}

#[test]
fn config_db_resource_lane_exposes_package_artifact_projection() {
    let temp = TestDir::new("skiff-compiler", "shared-fixture-runtime-lane");
    write_representative_package_project(&temp);

    let project = compile_package_project(temp.path()).expect("package project should compile");
    let artifact = &project.package.artifact;
    let main = module_artifact(&project.package, "main");

    assert!(artifact
        .runtime_requirements
        .config
        .iter()
        .any(|requirement| {
            requirement.path == "probe.token"
                && matches!(requirement.access, PackageConfigAccess::Required { .. })
        }));
    assert_eq!(
        main.unit
            .declarations
            .db
            .get("Record")
            .expect("logical DB schema should reach typed File IR")
            .key
            .name,
        "id"
    );
    let resource_ref = artifact
        .static_resources
        .iter()
        .find(|resource| resource.path == "prompts/probe.txt")
        .expect("static resource should reach PackageArtifact");
    let blob_path = resource_ref
        .artifact_path
        .as_deref()
        .expect("materialized resource should carry its artifact path");
    assert!(blob_path.starts_with("records/package-artifacts/"));
    let blob = project
        .package
        .resource_blobs
        .iter()
        .find(|blob| blob.sha256 == resource_ref.sha256 && blob.byte_len == resource_ref.byte_len)
        .expect("resource blob by canonical content identity");
    assert_eq!(blob.bytes, b"probe resource\n");
}

#[test]
fn explicit_contract_lane_compiles_without_provider_source() {
    let temp = TestDir::new("skiff-compiler", "shared-fixture-contract-lane");
    write_representative_package_project(&temp);

    let operation = BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "input".to_string(),
            ty: ContractTypeRef::builtin("string"),
            value_plan: linkable(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("string"),
            value_plan: linkable(BoundaryValueOwner::Provider),
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    };
    let contract = compile_service_contract(ServiceContractDefinition {
        service_id: "example.probe".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([("echo".to_string(), operation)]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Probe".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("code-free service contract should compile");

    let dependency = package_contract_dependency("probe_contract", contract);
    let expected_requirement = dependency.requirement.clone();
    let contract_dependencies = BTreeMap::from([(
        ("example.com/probe-app".to_string(), "1.0.0".to_string()),
        vec![dependency],
    )]);

    let project =
        compile_package_project_with_contract_dependencies(temp.path(), &contract_dependencies)
            .expect("package should compile with an unused typed contract dependency");

    assert_eq!(
        project.package.artifact.contract_requirements,
        vec![expected_requirement]
    );
    assert!(project.package.artifact.service_requirements.is_empty());
    assert!(project.package.artifact.service_call_refs.is_empty());
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn write_representative_package_project(temp: &TestDir) {
    temp.write(
        "package.yml",
        r#"id: example.com/probe-app
version: 1.0.0
packages:
  - id: example.com/probe-dependency
    version: 1.0.0
    alias: dep
resources:
  - prompts/probe.txt
"#,
    );
    temp.write(
        "api.yml",
        "Result: main.Result\nrun: main.run\nconfigured: main.configured\n",
    );
    temp.write(
        "main.skiff",
        r#"import dep

type Result { message: dep.Message }
type Record { id: string, value: string }

db object Record {
  primary key(id)
}

function run(input: dep.Message) -> Result {
  return Result { message: input }
}

function configured() -> string {
  return config.require<string>("probe.token")
}
"#,
    );
    temp.write("prompts/probe.txt", "probe resource\n");

    temp.write(
        ".skiff-packages/example~com~~probe-dependency/1.0.0/package.yml",
        "id: example.com/probe-dependency\nversion: 1.0.0\n",
    );
    temp.write(
        ".skiff-packages/example~com~~probe-dependency/1.0.0/api.yml",
        "Message: dep.Message\n",
    );
    temp.write(
        ".skiff-packages/example~com~~probe-dependency/1.0.0/dep.skiff",
        "type Message { text: string }\n",
    );
}
