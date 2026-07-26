mod common;

use common::{package_project::compile_service_package_project, TestDir};
use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{PackageLocalAbiSymbol, PackageServiceCallRoot};
use skiff_compiler::ServiceApiFunctionStatus;

#[test]
fn service_call_markers_flow_from_api_yml_to_exact_function_and_instance_roots() {
    let marked = service_fixture("service-call-roots-marked", true, true);
    let (marked_project, marked_api) =
        compile_service_package_project(marked.path(), "example.com/service-call-roots").unwrap();
    validate_package_artifact_identities(&marked_project.package.artifact).unwrap();

    let roots = &marked_project.package.artifact.service_call_roots;
    assert_eq!(roots.len(), 2);
    let PackageServiceCallRoot::Function {
        public_path,
        callable_id,
    } = &roots[0]
    else {
        panic!("first serviceCall root must be the selected function")
    };
    assert_eq!(public_path, "selected");
    assert_eq!(
        Some(callable_id),
        public_callable_id(&marked_project.package.artifact, "selected")
    );
    let PackageServiceCallRoot::PublicInstance {
        public_path,
        methods,
    } = &roots[1]
    else {
        panic!("second serviceCall root must be the public instance")
    };
    assert_eq!(public_path, "worker");
    assert_eq!(
        methods.keys().map(String::as_str).collect::<Vec<_>>(),
        ["run", "stop"]
    );
    for (method, callable_id) in methods {
        assert_eq!(
            Some(callable_id),
            public_callable_id(
                &marked_project.package.artifact,
                &format!("worker.{method}")
            )
        );
    }

    assert_eq!(
        marked_api
            .available
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["selected", "worker.run", "worker.stop"]
    );
    assert_eq!(marked_api.contract.operations.len(), 3);
    let package_only = marked_api
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

    let unmarked = service_fixture("service-call-roots-unmarked", false, true);
    let (unmarked_project, unmarked_api) =
        compile_service_package_project(unmarked.path(), "example.com/service-call-roots").unwrap();
    validate_package_artifact_identities(&unmarked_project.package.artifact).unwrap();
    assert_eq!(
        marked_project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity,
        unmarked_project
            .package
            .artifact
            .package_local_abi
            .local_abi_identity
    );
    assert_ne!(
        marked_project.package.artifact.package_build_id,
        unmarked_project.package.artifact.package_build_id
    );
    assert_ne!(
        marked_api.contract.service_protocol_identity,
        unmarked_api.contract.service_protocol_identity
    );
    assert_eq!(
        unmarked_project.package.artifact.service_call_roots.len(),
        1
    );
    assert_eq!(unmarked_api.contract.operations.len(), 2);

    let zero = service_fixture("service-call-roots-zero", false, false);
    let (zero_project, zero_api) =
        compile_service_package_project(zero.path(), "example.com/service-call-roots").unwrap();
    assert!(zero_project.package.artifact.service_call_roots.is_empty());
    assert!(zero_api.contract.operations.is_empty());
    assert!(zero_api.available.is_empty());
    assert!(zero_api.unavailable.is_empty());
    assert!(zero_api.visibility.functions.iter().all(|function| {
        matches!(
            &function.status,
            ServiceApiFunctionStatus::Available {
                service_operation_id: None
            }
        )
    }));
}

fn service_fixture(name: &str, select_function: bool, select_instance: bool) -> TestDir {
    let root = TestDir::new("skiff-compiler", name);
    root.write(
        "package.yml",
        "id: example.com/service-call-roots-implementation\nversion: 1.0.0\n",
    );
    let selected = if select_function {
        "selected:\n  source: main.selected\n  serviceCall: true\n"
    } else {
        "selected: main.selected\n"
    };
    let instance_marker = if select_instance {
        "  serviceCall: true\n"
    } else {
        ""
    };
    root.write(
        "api.yml",
        format!(
            "{selected}packageOnly: main.packageOnly\nworker:\n  const: root.main.worker\n  interfaces:\n    - root.main.WorkerApi\n{instance_marker}"
        ),
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
"#,
    );
    root
}

fn public_callable_id<'a>(
    artifact: &'a skiff_artifact_model::PackageArtifact,
    path: &str,
) -> Option<&'a skiff_artifact_model::PackageCallableId> {
    match artifact.package_local_abi.public_symbols.get(path) {
        Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) => Some(callable_id),
        _ => None,
    }
}
