mod common;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ExecutableIr, PackageRefIr, PackageSymbolRef, PackageTypeRequirement, TypeRefIr,
};
use skiff_compiler::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_package_project_with_contract_dependencies,
    },
    package_schemas::public_contract_type,
    TestDir,
};

const PACKAGE_ID: &str = "example.com/file-ir-execution-types";
const SCHEMA_PACKAGE_ID: &str = "example.com/file-ir-execution-schema";
const SCHEMA_ALIAS: &str = "contractSchema";
const REQUEST_SCHEMA_KEY: &str = "Request";
const SERVICE_ID: &str = "example.payments";
const VERSION: &str = "1.0.0";

#[test]
fn contract_typed_executables_preserve_package_nominal_execution_identity() {
    let without_external_symbol = compile_package_nominal_fixture("without-symbol", "");
    let with_external_symbol =
        compile_package_nominal_fixture("with-symbol", "type Unrelated { value: string }\n");

    let baseline = module_artifact(&without_external_symbol.package, "main");
    let with_symbol = module_artifact(&with_external_symbol.package, "main");
    for name in ["wrapper", "private_helper", "consume"] {
        assert_execution_signature_eq(
            executable(baseline.unit.executables.as_slice(), name),
            executable(with_symbol.unit.executables.as_slice(), name),
        );
    }

    let request_type = package_nominal_request_type();
    let wrapper = executable(&baseline.unit.executables, "wrapper");
    assert_eq!(wrapper.params.len(), 2);
    assert_eq!(wrapper.params[0].name, "label");
    assert_eq!(wrapper.params[0].ty, TypeRefIr::builtin("string"));
    assert_eq!(wrapper.params[1].name, "request");
    assert_eq!(wrapper.params[1].ty, request_type);
    assert_eq!(wrapper.return_type, request_type);
    assert!(!wrapper.may_suspend);

    let private_helper = executable(&baseline.unit.executables, "private_helper");
    let nested = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Nullable {
            inner: Box::new(request_type.clone()),
        }],
    };
    assert_eq!(private_helper.params[0].ty, nested);
    assert_eq!(private_helper.return_type, nested);

    let consume = executable(&baseline.unit.executables, "consume");
    assert!(consume.may_suspend);
    assert_eq!(consume.params[0].ty, request_type);
    assert_eq!(consume.return_type, request_type);

    assert!(baseline.unit.external_refs.service_symbols.is_empty());
    assert_schema_dependency_requirement(&without_external_symbol);
    let schema_dependency = without_external_symbol
        .dependency(SCHEMA_PACKAGE_ID, VERSION)
        .expect("canonical schema owner must remain in the dependency closure");
    let contract_type_id =
        &schema_dependency.package_schema_index.types[REQUEST_SCHEMA_KEY].package_schema_type_id;
    let executable_wire = serde_json::to_string(&baseline.unit.executables).unwrap();
    assert!(executable_wire.contains(SCHEMA_PACKAGE_ID));
    assert!(executable_wire.contains(REQUEST_SCHEMA_KEY));
    assert!(!executable_wire.contains(contract_type_id.as_str()));
    assert!(!executable_wire.contains("packageSchema"));
    assert!(!executable_wire.contains("serviceSymbol"));
    assert!(
        !executable_wire.contains("\"unknown\""),
        "Package nominal execution identity must not degrade to builtin unknown"
    );
}

#[test]
fn impl_receiver_stays_local_while_contract_parameter_preserves_package_nominal_identity() {
    let temp = TestDir::new("skiff-compiler", "file-ir-execution-impl-receiver");
    write_consumer_manifest(&temp);
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
    write_schema_package_dependency(&temp);
    let dependencies = package_nominal_contract_fixture();
    let project = compile_package_project_with_contract_dependencies(temp.path(), &dependencies)
        .expect("impl receiver and Package nominal contract parameter fixture should compile");
    let main = module_artifact(&project.package, "main");
    let relay = executable(&main.unit.executables, "Adapter.relay");

    assert_eq!(
        relay.self_type,
        Some(TypeRefIr::LocalType { type_index: 0 })
    );
    assert_eq!(relay.params.len(), 1);
    assert_eq!(relay.params[0].name, "request");
    assert_eq!(relay.params[0].ty, package_nominal_request_type());
    assert_eq!(relay.return_type, package_nominal_request_type());
    assert!(main.unit.external_refs.service_symbols.is_empty());
    assert_schema_dependency_requirement(&project);
}

fn compile_package_nominal_fixture(
    suffix: &str,
    unrelated_type: &str,
) -> common::package_project::PublishedPackageProject {
    let temp = TestDir::new(
        "skiff-compiler",
        &format!("file-ir-execution-type-{suffix}"),
    );
    write_consumer_manifest(&temp);
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
    write_schema_package_dependency(&temp);
    let dependencies = package_nominal_contract_fixture();
    compile_package_project_with_contract_dependencies(temp.path(), &dependencies)
        .expect("Package nominal execution identity fixture should compile")
}

fn package_nominal_contract_fixture() -> BTreeMap<
    skiff_compiler_input::package_config::PackageManifestKey,
    Vec<skiff_compiler::PackageContractCompileDependency>,
> {
    let seed = TestDir::new("skiff-compiler", "file-ir-execution-schema-seed");
    seed.write(
        "package.yml",
        format!("id: {SCHEMA_PACKAGE_ID}\nversion: {VERSION}\n"),
    );
    seed.write("api.yml", format!("{REQUEST_SCHEMA_KEY}: main.Request\n"));
    seed.write("main.skiff", "type Request { message: string }\n");
    let seed =
        compile_package_project(seed.path()).expect("independent schema owner seed should compile");
    let (request, request_id) = public_contract_type(&seed.package, REQUEST_SCHEMA_KEY);
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
            package_id: SCHEMA_PACKAGE_ID.to_string(),
            required_type_ids: vec![request_id.clone()],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Payments".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::from([(request_id.clone(), "Request".to_string())]),
        },
    })
    .unwrap();
    assert_eq!(
        contract.package_type_requirements,
        vec![PackageTypeRequirement {
            package_id: SCHEMA_PACKAGE_ID.to_string(),
            required_type_ids: vec![request_id],
        }]
    );
    BTreeMap::from([(
        (PACKAGE_ID.to_string(), VERSION.to_string()),
        vec![package_contract_dependency("payments", contract)],
    )])
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

fn package_nominal_request_type() -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: SCHEMA_PACKAGE_ID.to_string(),
            },
            symbol_path: REQUEST_SCHEMA_KEY.to_string(),
            abi_expectation: None,
        },
    }
}

fn write_consumer_manifest(temp: &TestDir) {
    temp.write(
        "package.yml",
        format!(
            "id: {PACKAGE_ID}\nversion: {VERSION}\npackages:\n  - id: {SCHEMA_PACKAGE_ID}\n    version: {VERSION}\n    alias: {SCHEMA_ALIAS}\n"
        ),
    );
}

fn write_schema_package_dependency(temp: &TestDir) {
    let encoded = SCHEMA_PACKAGE_ID.replace('.', "~").replace('/', "~~");
    let root = format!(".skiff-packages/{encoded}/{VERSION}");
    temp.write(
        &format!("{root}/package.yml"),
        format!("id: {SCHEMA_PACKAGE_ID}\nversion: {VERSION}\n"),
    );
    temp.write(
        &format!("{root}/api.yml"),
        format!("{REQUEST_SCHEMA_KEY}: main.Request\n"),
    );
    temp.write(
        &format!("{root}/main.skiff"),
        "type Request { message: string }\n",
    );
}

fn assert_schema_dependency_requirement(
    project: &common::package_project::PublishedPackageProject,
) {
    let requirement = project
        .package
        .artifact
        .package_requirements
        .iter()
        .find(|requirement| requirement.package_id == SCHEMA_PACKAGE_ID)
        .expect("consumer manifest must retain the canonical schema owner requirement");
    assert_eq!(requirement.alias, SCHEMA_ALIAS);
    assert_eq!(requirement.exact_version, VERSION);
    let dependency = project
        .dependency(SCHEMA_PACKAGE_ID, VERSION)
        .expect("canonical schema owner must remain in the dependency closure");
    assert_eq!(
        requirement.expected_local_abi,
        dependency.artifact.package_local_abi.local_abi_identity
    );
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
