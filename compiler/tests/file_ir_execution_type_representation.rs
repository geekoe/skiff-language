mod common;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ExecutableIr, PackageTypeRequirement, TypeRefIr,
};
use skiff_compiler::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_package_project_with_contract_dependencies_and_schemas,
    },
    package_schemas::{public_contract_type, resolved_package_schema},
    TestDir,
};

const PACKAGE_ID: &str = "example.com/file-ir-execution-types";
const SERVICE_ID: &str = "example.payments";
const VERSION: &str = "1.0.0";

#[test]
fn contract_typed_executables_have_one_opaque_execution_representation() {
    let without_external_symbol = compile_fixture("without-symbol", "");
    let with_external_symbol = compile_fixture("with-symbol", "type Unrelated { value: string }\n");

    let baseline = module_artifact(&without_external_symbol.package, "main");
    let with_symbol = module_artifact(&with_external_symbol.package, "main");
    for name in ["wrapper", "private_helper", "consume"] {
        assert_execution_signature_eq(
            executable(baseline.unit.executables.as_slice(), name),
            executable(with_symbol.unit.executables.as_slice(), name),
        );
    }

    let wrapper = executable(&baseline.unit.executables, "wrapper");
    assert_eq!(wrapper.params.len(), 2);
    assert_eq!(wrapper.params[0].name, "label");
    assert_eq!(wrapper.params[0].ty, TypeRefIr::builtin("string"));
    assert_eq!(wrapper.params[1].name, "request");
    assert_eq!(wrapper.params[1].ty, opaque_unknown());
    assert_eq!(wrapper.return_type, opaque_unknown());
    assert!(!wrapper.may_suspend);

    let private_helper = executable(&baseline.unit.executables, "private_helper");
    let nested = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Nullable {
            inner: Box::new(opaque_unknown()),
        }],
    };
    assert_eq!(private_helper.params[0].ty, nested);
    assert_eq!(private_helper.return_type, nested);

    let consume = executable(&baseline.unit.executables, "consume");
    assert!(consume.may_suspend);
    assert_eq!(consume.params[0].ty, opaque_unknown());
    assert_eq!(consume.return_type, opaque_unknown());

    assert!(baseline.unit.external_refs.service_symbols.is_empty());
    let contract_type_id = &without_external_symbol.package.package_schema_index.types["Request"]
        .package_schema_type_id;
    let executable_wire = serde_json::to_string(&baseline.unit.executables).unwrap();
    assert!(!executable_wire.contains(contract_type_id.as_str()));
    assert!(!executable_wire.contains("serviceSymbol"));
}

#[test]
fn impl_receiver_and_contract_parameter_keep_distinct_execution_roles() {
    let temp = TestDir::new("skiff-compiler", "file-ir-execution-impl-receiver");
    temp.write(
        "package.yml",
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n"),
    );
    temp.write("api.yml", "{}\n");
    temp.write(
        "main.skiff",
        r#"
type Adapter { label: string }

impl Adapter {
  function relay(request: payments.Request) -> payments.Request {
    return request
  }
}
"#,
    );
    let (dependencies, schemas) = contract_fixture();
    let project = compile_package_project_with_contract_dependencies_and_schemas(
        temp.path(),
        &dependencies,
        &schemas,
    )
    .expect("impl receiver fixture should compile");
    let main = module_artifact(&project.package, "main");
    let relay = executable(&main.unit.executables, "Adapter.relay");

    assert_eq!(
        relay.self_type,
        Some(TypeRefIr::LocalType { type_index: 0 })
    );
    assert_eq!(relay.params.len(), 1);
    assert_eq!(relay.params[0].name, "request");
    assert_eq!(relay.params[0].ty, opaque_unknown());
    assert_eq!(relay.return_type, opaque_unknown());
    assert!(main.unit.external_refs.service_symbols.is_empty());
}

fn compile_fixture(
    suffix: &str,
    unrelated_type: &str,
) -> common::package_project::PublishedPackageProject {
    let temp = TestDir::new(
        "skiff-compiler",
        &format!("file-ir-execution-type-{suffix}"),
    );
    temp.write(
        "package.yml",
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n"),
    );
    temp.write("api.yml", "Request: main.Request\nwrapper: main.wrapper\n");
    temp.write(
        "main.skiff",
        format!(
            r#"{unrelated_type}
type Request {{ message: string }}

function wrapper(label: string, request: payments.Request) -> payments.Request {{
  return request
}}

function private_helper(requests: Array<payments.Request?>) -> Array<payments.Request?> {{
  return requests
}}

function consume(request: payments.Request) -> payments.Request {{
  return payments/echo(request)
}}
"#
        ),
    );
    let (dependencies, schemas) = contract_fixture();
    compile_package_project_with_contract_dependencies_and_schemas(
        temp.path(),
        &dependencies,
        &schemas,
    )
    .expect("contract execution type fixture should compile")
}

fn contract_fixture() -> (
    BTreeMap<
        skiff_compiler_input::package_config::PackageManifestKey,
        Vec<skiff_compiler::PackageContractCompileDependency>,
    >,
    BTreeMap<
        skiff_compiler_input::package_config::PackageManifestKey,
        Vec<skiff_compiler::ResolvedPackageSchema>,
    >,
) {
    let seed = TestDir::new("skiff-compiler", "file-ir-execution-schema-seed");
    seed.write(
        "package.yml",
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n"),
    );
    seed.write("api.yml", "Request: main.Request\n");
    seed.write("main.skiff", "type Request { message: string }\n");
    let seed = compile_package_project(seed.path()).expect("schema seed should compile");
    let (request, request_id) = public_contract_type(&seed.package, "Request");
    let contract = compile_service_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: VERSION.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "request".to_string(),
                    ty: request.clone(),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: request,
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
            },
        )]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: PACKAGE_ID.to_string(),
            required_type_ids: vec![request_id.clone()],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Payments".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::from([(request_id, "Request".to_string())]),
        },
    })
    .unwrap();
    (
        BTreeMap::from([(
            (PACKAGE_ID.to_string(), VERSION.to_string()),
            vec![package_contract_dependency("payments", contract)],
        )]),
        BTreeMap::from([(
            (PACKAGE_ID.to_string(), VERSION.to_string()),
            vec![resolved_package_schema("self", &seed.package).unwrap()],
        )]),
    )
}

fn executable<'a>(executables: &'a [ExecutableIr], name: &str) -> &'a ExecutableIr {
    executables
        .iter()
        .find(|executable| executable.symbol.ends_with(&format!(".{name}")))
        .unwrap_or_else(|| panic!("missing File IR executable `{name}`"))
}

fn assert_execution_signature_eq(left: &ExecutableIr, right: &ExecutableIr) {
    assert_eq!(left.kind, right.kind);
    assert_eq!(left.type_params, right.type_params);
    assert_eq!(left.params, right.params);
    assert_eq!(left.return_type, right.return_type);
    assert_eq!(left.self_type, right.self_type);
    assert_eq!(left.may_suspend, right.may_suspend);
}

fn opaque_unknown() -> TypeRefIr {
    TypeRefIr::builtin("unknown")
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
