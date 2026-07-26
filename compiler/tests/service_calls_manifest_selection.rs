mod common;

use std::collections::BTreeMap;

use common::{package_project::compile_service_package_project, TestDir};
use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, NominalTypeRefBaseIr, PackageCallableId,
    PackageLocalAbiSymbol, TypeRefIr,
};
use skiff_compiler::ServiceApiFunctionStatus;
use skiff_compiler_contract::{project_service_api, ContractDefinitionError};

const PACKAGE_ID: &str = "example.com/manifest-selection-package";
const SERVICE_ID: &str = "example.com/manifest-selection";

#[test]
fn manifest_selection_projects_function_and_complete_public_instance_only() {
    let selected = service_fixture(
        "manifest-selection-function-instance",
        ServiceCalls::Paths(&["worker", "selected"]),
    );
    let (selected_project, selected_api) =
        compile_service_package_project(selected.path()).unwrap();
    validate_package_artifact_identities(&selected_project.package.artifact).unwrap();

    assert_eq!(
        selected_api
            .available
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["selected", "worker.run", "worker.stop"]
    );
    assert_eq!(selected_api.contract.operations.len(), 3);
    for path in ["selected", "worker.run", "worker.stop"] {
        assert_eq!(
            selected_api.available.get(path),
            public_callable_id(&selected_project.package.artifact, path)
        );
    }
    let package_only = selected_api
        .visibility
        .functions
        .iter()
        .find(|function| function.public_path == "packageOnly")
        .unwrap();
    assert!(matches!(
        package_only.status,
        ServiceApiFunctionStatus::Available {
            service_operation_id: None
        }
    ));

    let instance_only = service_fixture(
        "manifest-selection-instance-only",
        ServiceCalls::Paths(&["worker"]),
    );
    let (instance_project, instance_api) =
        compile_service_package_project(instance_only.path()).unwrap();
    assert_eq!(
        selected_project.package.artifact,
        instance_project.package.artifact
    );
    assert_eq!(
        selected_project.package.artifact.package_build_id,
        instance_project.package.artifact.package_build_id
    );
    assert_eq!(
        selected_project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity,
        instance_project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity
    );
    assert_ne!(
        selected_api.contract.service_protocol_identity,
        instance_api.contract.service_protocol_identity
    );
    assert_eq!(
        instance_api
            .available
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["worker.run", "worker.stop"]
    );
}

#[test]
fn manifest_selection_is_canonical_and_missing_matches_empty() {
    let left = service_fixture(
        "manifest-selection-order-left",
        ServiceCalls::Paths(&["selected", "worker"]),
    );
    let right = service_fixture(
        "manifest-selection-order-right",
        ServiceCalls::Paths(&["worker", "selected"]),
    );
    let missing = service_fixture("manifest-selection-missing", ServiceCalls::Missing);
    let empty = service_fixture("manifest-selection-empty", ServiceCalls::Empty);

    let (left_project, left_api) = compile_service_package_project(left.path()).unwrap();
    let (right_project, right_api) = compile_service_package_project(right.path()).unwrap();
    let (missing_project, missing_api) = compile_service_package_project(missing.path()).unwrap();
    let (empty_project, empty_api) = compile_service_package_project(empty.path()).unwrap();

    assert_eq!(
        left_project.package.artifact,
        right_project.package.artifact
    );
    assert_eq!(left_api.contract, right_api.contract);
    assert_eq!(left_api.service_calls, ["selected", "worker"]);
    assert_eq!(right_api.service_calls, ["selected", "worker"]);
    assert_eq!(
        left_api.contract.service_protocol_identity,
        right_api.contract.service_protocol_identity
    );
    assert_eq!(
        missing_project.package.artifact,
        empty_project.package.artifact
    );
    assert_eq!(missing_api.contract, empty_api.contract);
    assert!(missing_api.contract.operations.is_empty());
    assert!(missing_api.contract.package_type_requirements.is_empty());
    assert!(missing_api.available.is_empty());
    assert!(missing_api.unavailable.is_empty());
    assert!(missing_api.visibility.functions.iter().all(|function| {
        !matches!(
            &function.status,
            ServiceApiFunctionStatus::Available {
                service_operation_id: Some(_)
            }
        )
    }));

    let direct_left = project_selection(
        &left_project.package.artifact,
        &left_project.package.resolved_package_schema_type_records,
        &["worker", "selected"],
    )
    .unwrap();
    let direct_right = project_selection(
        &left_project.package.artifact,
        &left_project.package.resolved_package_schema_type_records,
        &["selected", "worker"],
    )
    .unwrap();
    assert_eq!(direct_left, direct_right);
    assert_eq!(direct_left.service_calls, ["selected", "worker"]);
}

#[test]
fn manifest_selection_rejects_non_roots_aliases_duplicates_and_unknowns() {
    let fixture = service_fixture("manifest-selection-negative", ServiceCalls::Empty);
    let (project, _) = compile_service_package_project(fixture.path()).unwrap();
    let package = &project.package.artifact;
    let records = &project.package.resolved_package_schema_type_records;

    assert!(matches!(
        project_selection(package, records, &["worker.run"]),
        Err(ContractDefinitionError::PublicInstanceMethodSelection {
            path,
            public_instance,
        }) if path == "worker.run" && public_instance == "worker"
    ));
    assert!(matches!(
        project_selection(package, records, &["unknown"]),
        Err(ContractDefinitionError::UnknownServiceCallPath { path })
            if path == "unknown"
    ));
    assert!(matches!(
        project_selection(package, records, &["Worker"]),
        Err(ContractDefinitionError::NonCallableServiceCallPath { path, kind })
            if path == "Worker" && kind == "type"
    ));
    assert!(matches!(
        project_selection(package, records, &["version"]),
        Err(ContractDefinitionError::NonCallableServiceCallPath { path, kind })
            if path == "version" && kind == "constant"
    ));
    assert!(matches!(
        project_selection(package, records, &["selected", "selected"]),
        Err(ContractDefinitionError::DuplicateServiceCallPath { path })
            if path == "selected"
    ));

    let mut method_alias = package.clone();
    method_alias.package_local_abi.public_symbols.insert(
        "runAlias".to_string(),
        method_alias.package_local_abi.public_symbols["worker.run"].clone(),
    );
    assert!(matches!(
        project_selection(&method_alias, records, &["runAlias"]),
        Err(ContractDefinitionError::PublicInstanceMethodAlias {
            path,
            method_paths,
            ..
        }) if path == "runAlias" && method_paths == ["worker.run"]
    ));

    let mut duplicate_callable = package.clone();
    duplicate_callable.package_local_abi.public_symbols.insert(
        "packageOnly".to_string(),
        duplicate_callable.package_local_abi.public_symbols["selected"].clone(),
    );
    assert!(matches!(
        project_selection(
            &duplicate_callable,
            records,
            &["packageOnly", "selected"]
        ),
        Err(ContractDefinitionError::DuplicatePublicCallable { first, second, .. })
            if first == "packageOnly" && second == "selected"
    ));
}

#[test]
fn manifest_selection_fails_closed_on_boundary_gaps_and_aggregates_unavailable() {
    let fixture = service_fixture("manifest-selection-boundary", ServiceCalls::Empty);
    let (project, _) = compile_service_package_project(fixture.path()).unwrap();
    let records = &project.package.resolved_package_schema_type_records;

    let mut missing = project.package.artifact.clone();
    let selected_id = public_callable_id(&missing, "selected").unwrap().clone();
    missing.boundary_projections.remove(&selected_id);
    assert!(matches!(
        project_selection(&missing, records, &["selected"]),
        Err(ContractDefinitionError::MissingBoundaryProjection { callable_id })
            if callable_id == selected_id.as_str()
    ));

    let mut unavailable = project.package.artifact.clone();
    for (path, reasons) in [
        (
            "selected",
            vec![
                BoundaryUnavailableReason::AnalysisPending,
                BoundaryUnavailableReason::WritesCallerReachable,
            ],
        ),
        (
            "packageOnly",
            vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
        ),
    ] {
        let callable_id = public_callable_id(&unavailable, path).unwrap().clone();
        unavailable.boundary_projections.insert(
            callable_id,
            BoundaryCallableProjection::Unavailable { reasons },
        );
    }
    let ContractDefinitionError::UnavailableServiceCalls {
        unavailable: reported,
    } = project_selection(&unavailable, records, &["selected", "packageOnly"]).unwrap_err()
    else {
        panic!("all selected unavailable callables must be reported together")
    };
    assert_eq!(
        reported.keys().map(String::as_str).collect::<Vec<_>>(),
        ["packageOnly", "selected"]
    );
    assert_eq!(
        reported["selected"],
        [
            BoundaryUnavailableReason::AnalysisPending,
            BoundaryUnavailableReason::WritesCallerReachable,
        ]
    );
}

#[test]
fn generic_public_instance_preserves_receiver_arguments_and_callable_binders() {
    let fixture = generic_public_instance_fixture();
    let (project, api) = compile_service_package_project(fixture.path()).unwrap();
    let artifact = &project.package.artifact;
    validate_package_artifact_identities(artifact).unwrap();

    for method in ["run", "stop"] {
        let PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } = &artifact.package_local_abi.public_symbols[&format!("worker.{method}")]
        else {
            panic!("generic public instance method must remain callable")
        };
        assert_eq!(signature.type_params, ["T"]);
        let exact_method_path = format!("Worker<T>.{method}");
        let method_link = &artifact.implementation_links.impl_methods[&exact_method_path];
        let callable_target = &artifact.callable_links[callable_id].target;
        assert_eq!(callable_target.file_ref, method_link.file);
        assert_eq!(
            callable_target.executable_index,
            method_link.executable_index
        );
        assert!(!artifact
            .implementation_links
            .impl_methods
            .contains_key(&format!("Worker.{method}")));
    }

    let receiver = &artifact.implementation_links.constants["worker"].ty;
    assert!(matches!(
        receiver,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::ServiceSymbol { symbol },
            arguments,
        } if symbol.module_path == "main"
            && symbol.symbol == "Worker"
            && arguments == &[TypeRefIr::builtin("string")]
    ));
    assert_eq!(
        api.available.keys().map(String::as_str).collect::<Vec<_>>(),
        ["worker.run", "worker.stop"]
    );
}

fn project_selection(
    package: &skiff_artifact_model::PackageArtifact,
    records: &BTreeMap<
        skiff_artifact_model::PackageSchemaTypeId,
        skiff_artifact_model::PackageSchemaTypeRecord,
    >,
    paths: &[&str],
) -> Result<skiff_compiler::ServiceApiProjection, ContractDefinitionError> {
    let paths = paths
        .iter()
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    project_service_api(SERVICE_ID, &paths, package, records)
}

enum ServiceCalls<'a> {
    Missing,
    Empty,
    Paths(&'a [&'a str]),
}

fn service_fixture(name: &str, service_calls: ServiceCalls<'_>) -> TestDir {
    let root = TestDir::new("skiff-compiler", name);
    root.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    let service_calls = match service_calls {
        ServiceCalls::Missing => String::new(),
        ServiceCalls::Empty => "serviceCalls: []\n".to_string(),
        ServiceCalls::Paths(paths) => format!(
            "serviceCalls:\n{}",
            paths
                .iter()
                .map(|path| format!("  - {path}\n"))
                .collect::<String>()
        ),
    };
    root.write("service.yml", format!("id: {SERVICE_ID}\n{service_calls}"));
    root.write(
        "api.yml",
        "Worker: main.Worker\nversion: main.VERSION\nselected: main.selected\npackageOnly: main.packageOnly\nworker:\n  const: root.main.worker\n  interfaces:\n    - root.main.WorkerApi\n",
    );
    root.write(
        "main.skiff",
        r#"
function selected() -> string {
  return "selected"
}

function packageOnly() -> string {
  return "package-only"
}

interface WorkerApi {
  function run(self: Self, input: string) -> string
  function stop(self: Self) -> string
}

type Worker implements WorkerApi {}

impl Worker {
  function run(input: string) -> string {
    return "ran"
  }

  function stop() -> string {
    return "stopped"
  }

  function helper() -> string {
    return "private"
  }
}

const worker: Worker = Worker {}
const VERSION: string = "1"
"#,
    );
    root
}

fn generic_public_instance_fixture() -> TestDir {
    let root = TestDir::new("skiff-compiler", "manifest-selection-generic-instance");
    root.write(
        "package.yml",
        "id: example.com/generic-service-call-root\nversion: 1.0.0\n",
    );
    root.write(
        "service.yml",
        "id: example.com/generic-service-call-root\nserviceCalls:\n  - worker\n",
    );
    root.write(
        "api.yml",
        "worker:\n  const: root.main.worker\n  interfaces:\n    - root.main.WorkerApi\n",
    );
    root.write(
        "main.skiff",
        r#"
interface WorkerApi {
  function run(self: Self, input: string) -> string
  function stop(self: Self) -> string
}

type Worker<T> implements WorkerApi {
  value: T
}

impl Worker<T> {
  function run(self: Worker<T>, input: string) -> string {
    return "ran"
  }

  function stop() -> string {
    return "stopped"
  }

  function helper() -> string {
    return "private"
  }
}

const worker: Worker<string> = Worker<string> { value: "worker" }
"#,
    );
    root
}

fn public_callable_id<'a>(
    artifact: &'a skiff_artifact_model::PackageArtifact,
    path: &str,
) -> Option<&'a PackageCallableId> {
    match artifact.package_local_abi.public_symbols.get(path) {
        Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) => Some(callable_id),
        _ => None,
    }
}
