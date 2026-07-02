use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

mod common;
use common::{
    cli_command::{assert_failure, assert_success, stderr, CliCommand},
    TestDir,
};
use skiff_compiler_emission::identity::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes_from_json,
    FILE_IR_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX, SERVICE_UNIT_IDENTITY_PREFIX,
};

#[test]
fn cli_rejects_file_input_as_service_publication() {
    let temp = TestDir::new("skiff-compiler", "file");
    let source_path = temp.path().join("main.skiff");
    let artifact_path = temp.path().join("out").join("artifact.json");
    fs::write(
        &source_path,
        r#"
            function main() -> number {
                return 40 + 2
            }
        "#,
    )
    .unwrap();

    let output = CliCommand::compile(&source_path)
        .arg("--out")
        .arg(&artifact_path)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("expects a service publication root containing service.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("test runner or package tooling"),
        "unexpected stderr: {stderr}"
    );
    assert!(!artifact_path.exists());
}

#[test]
fn cli_rejects_directory_without_service_config_instead_of_main_skiff_fallback() {
    let temp = TestDir::new("skiff-compiler", "directory");
    let input_dir = temp.path().join("service");
    fs::create_dir_all(&input_dir).unwrap();
    let source_path = input_dir.join("main.skiff");
    let artifact_path = temp.path().join("artifact.json");
    fs::write(
        &source_path,
        r#"
            function main(name: string) -> string {
                return "hi, " + name
            }
        "#,
    )
    .unwrap();

    let output = CliCommand::compile(&input_dir)
        .arg("-o")
        .arg(&artifact_path)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("is not a service publication root; expected service.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("test runner or package tooling"),
        "unexpected stderr: {stderr}"
    );
    assert!(!artifact_path.exists());
}

#[test]
fn cli_rejects_ambiguous_publication_root_before_reading_service_config() {
    let temp = TestDir::new("skiff-compiler", "ambiguous-root");
    let input_dir = temp.path().join("service");
    fs::create_dir_all(&input_dir).unwrap();
    let artifact_path = temp.path().join("artifact.json");
    fs::write(input_dir.join("package.yml"), "not: relevant\n").unwrap();
    fs::write(input_dir.join("service.yml"), ": invalid service yaml\n").unwrap();

    let output = CliCommand::compile(&input_dir)
        .arg("--out")
        .arg(&artifact_path)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("publication root is ambiguous"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("package.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("service.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        !stderr.contains("service config error"),
        "ambiguous roots should fail before service config parsing: {stderr}"
    );
    assert!(!artifact_path.exists());
}

#[test]
fn cli_rejects_package_publication_root_as_service_input() {
    let temp = TestDir::new("skiff-compiler", "package-root");
    let input_dir = temp.path().join("package");
    fs::create_dir_all(&input_dir).unwrap();
    let artifact_path = temp.path().join("artifact.json");
    fs::write(
        input_dir.join("package.yml"),
        r#"
id: example.com/package
version: 1.0.0
"#,
    )
    .unwrap();

    let output = CliCommand::compile(&input_dir)
        .arg("--out")
        .arg(&artifact_path)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("package publication root"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("service publication root"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("service.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(!artifact_path.exists());
}

#[test]
fn cli_does_not_read_activation_secret_config_beside_valid_service_root() {
    let temp = TestDir::new("skiff-compiler", "activation-secret-config");
    let service_root = temp.path().join("service");
    write_minimal_service_project(&service_root, "example.com/activation-secret-config");
    let artifact_path = temp.path().join("artifact.json");
    fs::write(service_root.join("config.dev.secret.yml"), ":\n").unwrap();

    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&artifact_path)
        .arg("--profile")
        .arg("dev")
        .output();

    assert_success(&output);
    assert!(artifact_path.exists());
}

#[test]
fn cli_rejects_manifest_out_for_single_file_input() {
    let temp = TestDir::new("skiff-compiler", "manifest");
    let source_path = temp.path().join("main.skiff");
    let artifact_path = temp.path().join("artifact.json");
    let manifest_path = temp.path().join("router-manifest.json");
    fs::write(
        &source_path,
        r#"
            function add(a: number, b: number) -> number {
                return a + b
            }
        "#,
    )
    .unwrap();

    let output = CliCommand::compile(&source_path)
        .arg("--out")
        .arg(&artifact_path)
        .arg("--manifest-out")
        .arg(&manifest_path)
        .arg("--service-id")
        .arg("example.com/calc")
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("expects a service publication root containing service.yml"),
        "unexpected stderr: {stderr}"
    );
    assert!(!manifest_path.exists());
    assert!(!artifact_path.exists());
}

#[test]
fn cli_compiles_service_project_to_assembly_and_runtime_manifest_json_rejects_legacy_mongo_provider_package(
) {
    let temp = TestDir::new("skiff-compiler", "service-project");
    let artifact_path = temp.path().join("artifact.json");
    let manifest_path = temp.path().join("router-manifest.json");
    let assembly_copy_path = temp.path().join("service-assembly-copy.json");
    let packages_dir = temp.path().join("packages");
    write_legacy_mongo_package(&packages_dir);
    let service_root = temp.path().join("service");
    write_mongo_dependent_service_project(&service_root, "example.com/legacy-mongo-cli");

    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&artifact_path)
        .arg("--manifest-out")
        .arg(&manifest_path)
        .arg("--assembly-out")
        .arg(&assembly_copy_path)
        .arg("--packages-dir")
        .arg(&packages_dir)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("service publication build failed"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("skiff~run~~mongo/1.0.0/mongo.skiff"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("legacy provider syntax has been removed"),
        "unexpected stderr: {stderr}"
    );
    assert!(!artifact_path.exists());
    assert!(!manifest_path.exists());
    assert!(!assembly_copy_path.exists());
}

#[test]
fn cli_writes_artifact_root_and_links_else_if_operation_body() {
    let temp = TestDir::new("skiff-compiler", "artifact-root");
    let service_root = temp.path().join("service");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
access:
  visibility: internal
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("example.skiff"),
        r#"
type Output {}
interface ExampleService {
  function choose(flag: boolean, fallback: boolean) -> Output
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("example.skiff"),
        r#"
function choose(flag: boolean, fallback: boolean) -> root.api.example.Output {
  if flag {
    return {}
  } else if fallback {
    return {}
  } else {
    return {}
  }
}

type ExampleService {}

impl ExampleService {
  function choose(self: ExampleService, flag: boolean, fallback: boolean) -> root.api.example.Output {
    return root.internal.example.choose(flag, fallback)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("artifact-root");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .output();

    assert_success(&output);

    assert!(!artifact_root.join("sha256").exists());
    assert_eq!(json_file_count(&artifact_root.join("files")), 0);
    assert_eq!(
        json_file_count(&artifact_root.join("assemblies").join("services")),
        1
    );
    assert_eq!(json_file_count(&artifact_root.join("contracts")), 1);
    assert_eq!(json_file_count(&artifact_root.join("bundles")), 1);
    assert_eq!(json_file_count(&artifact_root.join("indexes")), 1);
    assert_eq!(
        json_file_count(&artifact_root.join("units").join("services")),
        1
    );
    assert_eq!(
        json_file_count(&artifact_root.join("units").join("files")),
        2
    );

    let index_path = first_json_file(&artifact_root.join("indexes")).unwrap();
    let index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    assert_eq!(index["schemaVersion"], "skiff-artifact-index-v1");
    assert_eq!(index["serviceId"], "example.com/example");
    assert!(index["service"].get("revisionIdentity").is_none());
    assert_eq!(
        index["contractIdentity"],
        index["service"]["protocolIdentity"]
    );
    assert_eq!(
        index["service"]["access"],
        serde_json::json!({ "visibility": "internal", "organizationRole": "viewer" })
    );
    let contract_hash = index["contractIdentity"]
        .as_str()
        .unwrap()
        .rsplit_once(":sha256:")
        .unwrap()
        .1;
    let contract_schema_path = format!("contracts/{contract_hash}.json");
    assert_eq!(
        index_path.strip_prefix(&artifact_root).unwrap(),
        Path::new("indexes")
            .join("services")
            .join(publication_storage_segment("example.com/example"))
            .join(format!("{contract_hash}.json"))
    );
    assert_eq!(index["contract"]["contractHash"], contract_hash);
    assert_eq!(
        index["contract"]["protocolIdentity"],
        index["contractIdentity"]
    );
    assert_eq!(index["contract"]["schemaPath"], contract_schema_path);
    let contract_artifact: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join(&contract_schema_path)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        contract_artifact["schemaVersion"],
        "skiff-contract-schema-v1"
    );
    assert!(contract_artifact.get("serviceId").is_none());
    assert!(contract_artifact.get("displayName").is_none());
    assert!(contract_artifact.get("revisionId").is_none());
    assert_eq!(contract_artifact["contractHash"], contract_hash);
    assert_eq!(
        contract_artifact["protocolIdentity"],
        index["contractIdentity"]
    );
    assert_eq!(
        contract_artifact["schema"]["schemaVersion"],
        "skiff-contract-canonical-v1"
    );
    assert_eq!(
        contract_artifact["schema"]["interfaces"]["ExampleService"]["operations"]["choose"]
            ["params"][0]["name"],
        "flag"
    );
    let assembly: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(assembly["sourceMap"]["format"], "skiff-source-map-v1");
    let source_map_paths = assembly["sourceMap"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["path"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        source_map_paths,
        BTreeSet::from(["api/example.skiff", "internal/example.skiff"])
    );
    assert!(assembly.get("packages").is_none());
    assert!(index.get("packages").is_none());
    assert_eq!(assembly["dependencyLock"], serde_json::json!([]));
    assert_eq!(index["dependencyLock"], assembly["dependencyLock"]);
    let assembly_files = assembly["files"].as_array().unwrap();
    let implementation_file_ref = assembly_files
        .iter()
        .find(|file| file["sourcePath"] == "internal/example.skiff")
        .unwrap();
    assert_eq!(implementation_file_ref["role"], "implementation");
    assert_eq!(implementation_file_ref["modulePath"], "internal.example");
    let api_file_ref = assembly_files
        .iter()
        .find(|file| file["sourcePath"] == "api/example.skiff")
        .unwrap();
    assert_eq!(api_file_ref["role"], "implementation");
    assert_eq!(api_file_ref["modulePath"], "api.example");
    assert!(assembly_files
        .iter()
        .all(|file| file["role"] == "implementation"));
    assert_eq!(
        assembly["service"]["protocolIdentity"],
        contract_artifact["protocolIdentity"]
    );
    assert_eq!(index["service"]["access"], assembly["service"]["access"]);
    assert_eq!(
        index["serviceAssembly"]["assemblyIdentity"],
        assembly["service"]["assemblyIdentity"]
    );
    assert_eq!(
        artifact_root.join(index["serviceAssembly"]["assemblyPath"].as_str().unwrap()),
        artifact_root.join(assembly_path_from_identity(
            "example.com/example",
            assembly["service"]["assemblyIdentity"].as_str().unwrap()
        ))
    );
    assert_eq!(
        index["serviceUnit"]["schemaVersion"],
        "skiff-service-unit-v1"
    );
    assert!(index["serviceUnit"]["unitIdentity"]
        .as_str()
        .unwrap()
        .starts_with(&format!("{SERVICE_UNIT_IDENTITY_PREFIX}:")));
    let service_unit_path = index["serviceUnit"]["unitPath"].as_str().unwrap();
    assert!(service_unit_path.starts_with("units/services/example~com~~example/"));
    let service_unit: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact_root.join(service_unit_path)).unwrap())
            .unwrap();
    assert_eq!(service_unit["schemaVersion"], "skiff-service-unit-v1");
    assert_eq!(service_unit["service"]["id"], "example.com/example");
    assert_eq!(service_unit["protocolIdentity"], index["contractIdentity"]);
    assert!(service_unit
        .get("packageDependencies")
        .is_none_or(|value| value == &serde_json::json!([])));
    assert!(service_unit.get("packageSymbolUsage").is_none());
    assert!(service_unit.get("packageAbiExpectations").is_none());
    assert!(service_unit.get("dependencyLock").is_none());
    assert!(service_unit.get("serviceDependencies").is_none());
    assert!(service_unit.get("buildId").is_none());
    assert!(service_unit.get("assemblyIdentity").is_none());
    assert_eq!(service_unit["files"].as_array().unwrap().len(), 2);
    assert!(service_unit["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| {
            file["fileIrIdentity"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{FILE_IR_IDENTITY_PREFIX}:"))
                && file["artifactPath"]
                    .as_str()
                    .unwrap()
                    .starts_with("units/files/")
        }));
    assert_eq!(
        service_unit["operations"][0]["executable"]["fileRef"]["modulePath"],
        "internal.example"
    );
    assert_eq!(
        service_unit["operations"][0]["executable"]["callableKind"],
        "publicFunction"
    );
    assert!(service_unit["operations"][0]["executable"]["executableIndex"].is_number());

    let file_ir_refs = index["fileIrUnits"].as_array().unwrap();
    assert_eq!(file_ir_refs.len(), 2);
    let implementation_file_ir_ref = file_ir_refs
        .iter()
        .find(|file| file["modulePath"] == "internal.example")
        .unwrap();
    assert_eq!(
        implementation_file_ir_ref["schemaVersion"],
        "skiff-file-ir-v3"
    );
    let implementation_file_ir: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            artifact_root.join(implementation_file_ir_ref["fileIrPath"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        implementation_file_ir["fileIrIdentity"],
        implementation_file_ir_ref["fileIrIdentity"]
    );
    assert_eq!(implementation_file_ir["modulePath"], "internal.example");
    let helper_index = implementation_file_ir["declarations"]["executables"]["choose"]
        ["executableIndex"]
        .as_u64()
        .unwrap() as usize;
    let body = &implementation_file_ir["executables"][helper_index]["body"];
    assert!(body["blocks"]
        .as_array()
        .is_some_and(|blocks| !blocks.is_empty()));
    assert!(body["statements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|statement| statement["kind"] == "if"));
    let operation = assembly["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == "ExampleService.choose")
        .unwrap();
    assert!(operation.get("fileIrIdentity").is_none());
    assert_eq!(
        operation["implementation"]["modulePath"],
        "internal.example"
    );
    assert_eq!(
        operation["implementation"]["symbol"],
        "__skiff_service_operation_adapter_ExampleService_choose"
    );
    assert_eq!(
        operation["implementation"]["function"],
        "__skiff_service_operation_adapter_ExampleService_choose"
    );
    assert!(operation["implementation"].get("method").is_none());
    assert!(operation["implementation"].get("receiver").is_none());
    let file_identity = operation["implementation"]["fileIrIdentity"]
        .as_str()
        .unwrap();
    let file_ref = assembly_files
        .iter()
        .find(|file| file["fileIrIdentity"] == file_identity)
        .unwrap();
    assert_eq!(file_ref["sourcePath"], "internal/example.skiff");
    assert_eq!(
        file_ref["modulePath"],
        operation["implementation"]["modulePath"]
    );
    assert_eq!(file_ref["role"], "implementation");
    assert_eq!(file_ref["fileIrIdentity"], file_identity);
    assert!(file_ref["fileIrPath"]
        .as_str()
        .unwrap()
        .starts_with("units/files/"));
    assert_eq!(
        file_ref["fileIrPath"],
        implementation_file_ir_ref["fileIrPath"]
    );
    assert_eq!(file_identity, implementation_file_ir_ref["fileIrIdentity"]);
    let implementation_source = assembly["sourceMap"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["path"] == "internal/example.skiff")
        .unwrap();
    assert_eq!(implementation_source["fileIrIdentity"], file_identity);
}

#[test]
fn cli_resolves_top_level_service_dependencies_into_service_unit_and_file_ir_calls() {
    let temp = TestDir::new("skiff-compiler", "service-dependencies");
    let service_artifact_root = temp.path().join("callee-artifacts");
    let callee = write_callee_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: account
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(userId: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, userId: string) -> Json {
    return account.UserApi.get(userId)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_success(&output);

    let assembly: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(
        assembly["dependencyLock"],
        serde_json::json!([{
            "kind": "service",
            "id": "skiff.run/account",
            "version": "0.1.0",
            "alias": "account",
            "declaredAlias": "account",
            "buildId": callee.build_id,
            "serviceProtocolIdentity": callee.service_protocol_identity,
            "operations": ["UserApi.get"],
            "targets": [CALLEE_OPERATION_ABI_ID],
        }])
    );
    let service_unit_path = assembly["serviceUnit"]["unitPath"].as_str().unwrap();
    let service_unit: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact_root.join(service_unit_path)).unwrap())
            .unwrap();
    assert_eq!(
        service_unit["serviceDependencies"],
        serde_json::json!([{
            "id": "skiff.run/account",
            "version": "0.1.0",
            "alias": "account",
            "buildId": callee.build_id,
            "serviceProtocolIdentity": callee.service_protocol_identity,
            "publicationAbi": callee_publication_abi()
        }])
    );

    let service_file = assembly["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["sourcePath"] == "internal/caller.skiff")
        .unwrap();
    let file_ir_path = service_file["fileIrPath"].as_str().unwrap();
    let file_ir: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact_root.join(file_ir_path)).unwrap())
            .unwrap();
    let service_dependency_symbols = file_ir["externalRefs"]["serviceDependencySymbols"]
        .as_array()
        .unwrap();
    assert_eq!(
        service_dependency_symbols,
        &vec![serde_json::json!({
            "dependencyRef": "account",
            "operation": callee_operation_ref()
        })]
    );
    assert!(file_ir["executables"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|executable| executable["body"]["expressions"].as_array().unwrap())
        .any(|expr| expr["kind"] == "call"
            && expr["call"]["target"]["kind"] == "serviceDependencySymbol"
            && expr["call"]["target"]["symbol"]["dependencyRef"] == "account"
            && expr["call"]["target"]["symbol"]["operation"]["operationAbiId"]
                == CALLEE_OPERATION_ABI_ID));
}

#[test]
fn cli_rejects_service_dependency_calls_to_unknown_callee_operations() {
    let temp = TestDir::new("skiff-compiler", "service-dependencies-missing-operation");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: account
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(userId: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, userId: string) -> Json {
    return account.UserApi.missing(userId)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("service publication build failed"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains(
            "service dependency `account` does not export public operation `UserApi.missing`"
        ),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("source call `account.UserApi.missing`"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_lowers_remote_public_instance_direct_call_and_box_source() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-source");
    let service_artifact_root = temp.path().join("callee-artifacts");
    let callee = write_callee_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}

interface LlmClient {
  function send(self: Self, input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    const boxed = remoteLlm/managedLlm as api.caller.LlmClient
    const boxedAgain = remoteLlm/managedLlm as api.caller.LlmClient
    const indirect = boxed.send(input)
    return remoteLlm/managedLlm.send(input)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_success(&output);

    let assembly: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(
        assembly["dependencyLock"],
        serde_json::json!([{
            "kind": "service",
            "id": "skiff.run/account",
            "version": "0.1.0",
            "alias": "remoteLlm",
            "declaredAlias": "remoteLlm",
            "buildId": callee.build_id,
            "serviceProtocolIdentity": callee.service_protocol_identity,
            "operations": ["managedLlm.send"],
            "targets": [CALLEE_PUBLIC_INSTANCE_OPERATION_ABI_ID],
            "remoteBoxProvenance": [{
                "interface": caller_llm_client_interface_ref(),
                "interfaceDisplay": caller_llm_client_interface_ref()["interfaceAbiId"].as_str().unwrap(),
                "publicInstance": "managedLlm",
                "methodAbiId": caller_llm_client_method_abi_id(),
                "operation": callee_public_instance_operation_ref()
            }]
        }])
    );

    let service_file = assembly["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["sourcePath"] == "internal/caller.skiff")
        .unwrap();
    let file_ir_path = service_file["fileIrPath"].as_str().unwrap();
    let file_ir: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact_root.join(file_ir_path)).unwrap())
            .unwrap();
    assert!(file_ir["executables"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|executable| executable["body"]["expressions"].as_array().unwrap())
        .any(|expr| expr["kind"] == "interfaceBox"
            && expr["source"]["kind"] == "remote"
            && expr["source"]["dependencyRef"] == "remoteLlm"
            && expr["source"]["publicInstanceKey"] == "managedLlm"));
    assert!(file_ir["executables"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|executable| executable["body"]["expressions"].as_array().unwrap())
        .any(|expr| expr["kind"] == "call"
            && expr["call"]["target"]["kind"] == "serviceDependencySymbol"
            && expr["call"]["target"]["symbol"]["dependencyRef"] == "remoteLlm"
            && expr["call"]["target"]["symbol"]["operation"]["operationAbiId"]
                == CALLEE_PUBLIC_INSTANCE_OPERATION_ABI_ID));
    assert!(file_ir["executables"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|executable| executable["body"]["expressions"].as_array().unwrap())
        .any(|expr| expr["kind"] == "call"
            && expr["call"]["target"]["kind"] == "interfaceMethod"
            && expr["call"]["target"]["methodAbiId"] == caller_llm_client_method_abi_id()));
    let remote_box_provenance = assembly["dependencyLock"][0]["remoteBoxProvenance"]
        .as_array()
        .unwrap();
    assert_eq!(
        remote_box_provenance.len(),
        1,
        "duplicate remote boxes for the same dependency/interface/operation should lock once"
    );
    assert!(assembly["dependencyLock"][0]
        .get("bindingProvenance")
        .is_none());
}

#[test]
fn cli_rejects_remote_public_instance_source_in_value_position() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-source-value");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_multi_interface_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    const source = remoteLlm/managedLlm
    return null
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("remote public instance source `remoteLlm/managedLlm` is not a value"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("exports 2 interfaces")
            && stderr.contains("cannot be inferred without `as I`"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_rejects_unknown_remote_public_instance_method() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-missing-method");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    return remoteLlm/managedLlm.missing(input)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("public instance `managedLlm` has no method `missing`"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_rejects_remote_public_instance_box_when_interface_not_implemented() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-not-implements");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}

interface OtherClient {
  function run(self: Self, input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    const boxed = remoteLlm/managedLlm as api.caller.OtherClient
    return null
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("does not implement selected interface"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_rejects_remote_public_instance_with_undeclared_dependency() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-undeclared");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}

interface LlmClient {
  function send(self: Self, input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    const boxed = remoteLlm/managedLlm as api.caller.LlmClient
    return null
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("service dependency `remoteLlm` is not declared"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_rejects_remote_box_when_method_signature_contains_any_interface() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-any-signature");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_unsafe_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}

interface Tool {
  function run(self: Self) -> Json
}

interface UnsafeClient {
  function send(self: Self, tool: any Tool) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    const boxed = remoteLlm/managedLlm as api.caller.UnsafeClient
    return null
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let box_stderr = stderr(&output);
    assert!(
        box_stderr.contains("cannot be used as a remote operation")
            && box_stderr.contains("contains any interface"),
        "unexpected stderr: {box_stderr}"
    );

    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    return remoteLlm/managedLlm.send(input)
  }
}
"#,
    )
    .unwrap();

    let direct_out_path = temp.path().join("service-assembly-direct.json");
    let direct_artifact_root = temp.path().join("caller-direct-artifacts");
    let direct_output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&direct_out_path)
        .arg("--artifact-root")
        .arg(&direct_artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&direct_output);
    let direct_stderr = stderr(&direct_output);
    assert!(
        direct_stderr.contains("cannot be used as a remote operation")
            && direct_stderr.contains("contains any interface"),
        "unexpected stderr: {direct_stderr}"
    );
}

#[test]
fn cli_rejects_remote_public_instance_duplicate_method_names() {
    let temp = TestDir::new("skiff-compiler", "remote-public-instance-duplicate-method");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_duplicate_public_instance_service_artifact_root(&service_artifact_root);
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: remoteLlm
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(input: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, input: string) -> Json {
    return remoteLlm/managedLlm.send(input)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("method `send` is ambiguous"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn cli_rejects_service_dependency_artifacts_with_unknown_operation_modes() {
    let temp = TestDir::new("skiff-compiler", "service-dependencies-unknown-mode");
    let service_artifact_root = temp.path().join("callee-artifacts");
    write_callee_service_artifact_root_with_operation_kind(&service_artifact_root, "clientStream");
    let service_root = temp.path().join("caller");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/caller
version: 1.0.0
services:
  - id: skiff.run/account
    version: 0.1.0
    alias: account
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
CallerService: internal.caller.CallerService
api:
  caller:
    CallerService: api.caller.CallerService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("caller.skiff"),
        r#"
interface CallerService {
  function lookup(userId: string) -> Json
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("caller.skiff"),
        r#"
type CallerService {}

impl CallerService {
  function lookup(self: CallerService, userId: string) -> Json {
    return account.UserApi.get(userId)
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("caller-artifacts");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--service-artifact-root")
        .arg(&service_artifact_root)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("service publication build failed"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("unknown variant `clientStream`"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("publicFunction"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn service_publication_build_emits_runtime_ir_units_for_packages_rejects_legacy_mongo_provider_package(
) {
    let temp = TestDir::new("skiff-compiler", "artifact-root-ir-units");
    let packages_dir = temp.path().join("packages");
    write_legacy_mongo_package(&packages_dir);
    let service_root = temp.path().join("service");
    write_mongo_dependent_service_project(&service_root, "example.com/legacy-mongo-artifacts");
    let out_path = temp.path().join("service-assembly.json");
    let artifact_root = temp.path().join("artifact-root");

    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--packages-dir")
        .arg(&packages_dir)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("service publication build failed"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("skiff~run~~mongo/1.0.0/mongo.skiff"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("legacy provider syntax has been removed"),
        "unexpected stderr: {stderr}"
    );
    assert!(!out_path.exists());
    assert!(!artifact_root.join("index").exists());
}

#[derive(Debug)]
struct CalleeServiceArtifact {
    build_id: String,
    service_protocol_identity: String,
}

const CALLEE_OPERATION_ABI_ID: &str = "operation:skiff.run/account:UserApi.get";
const CALLEE_PUBLIC_INSTANCE_OPERATION_ABI_ID: &str = "operation:skiff.run/account:managedLlm.send";

fn callee_operation_ref_with_kind(kind: &str) -> serde_json::Value {
    serde_json::json!({
        "operationAbiId": CALLEE_OPERATION_ABI_ID,
        "kind": kind,
        "publicPath": "UserApi.get",
        "displayName": "UserApi.get"
    })
}

fn callee_operation_ref() -> serde_json::Value {
    callee_operation_ref_with_kind("publicFunction")
}

fn callee_public_signature() -> serde_json::Value {
    serde_json::json!({
        "params": [{
            "name": "userId",
            "ty": { "kind": "builtin", "name": "string" }
        }],
        "returnType": { "kind": "builtin", "name": "Json" },
        "maySuspend": false
    })
}

fn callee_publication_abi() -> serde_json::Value {
    callee_publication_abi_with_operation_kind("publicFunction")
}

fn callee_publication_abi_with_operation_kind(kind: &str) -> serde_json::Value {
    let operation_ref = callee_operation_ref_with_kind(kind);
    serde_json::json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": "skiff.run/account",
        "version": "0.1.0",
        "abiIdentity": "skiff-publication-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "operationExports": [operation_ref.clone()],
        "operationAbi": [{
            "operation": operation_ref.clone(),
            "publicSignature": callee_public_signature()
        }],
        "sourceCallOperationIndex": [{
            "sourceCallPath": "UserApi.get",
            "operation": operation_ref
        }]
    })
}

fn write_callee_service_artifact_root(root: &Path) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_operation_kind(root, "publicFunction")
}

fn caller_interface_ref(symbol: &str) -> serde_json::Value {
    let interface_abi_id = serde_json::to_string(&serde_json::json!({
        "kind": "serviceSymbol",
        "symbol": {
            "modulePath": "api.caller",
            "symbol": symbol
        }
    }))
    .unwrap();
    serde_json::json!({
        "interfaceAbiId": interface_abi_id
    })
}

fn caller_llm_client_interface_ref() -> serde_json::Value {
    caller_interface_ref("LlmClient")
}

fn caller_llm_client_method_abi_id() -> String {
    caller_method_abi_id("LlmClient", "send")
}

fn caller_method_abi_id(interface_symbol: &str, method: &str) -> String {
    format!(
        "method:{}:{method}",
        caller_interface_ref(interface_symbol)["interfaceAbiId"]
            .as_str()
            .unwrap()
    )
}

fn callee_public_instance_operation_ref() -> serde_json::Value {
    let interface = caller_llm_client_interface_ref();
    let method_abi_id = caller_llm_client_method_abi_id();
    serde_json::json!({
        "operationAbiId": CALLEE_PUBLIC_INSTANCE_OPERATION_ABI_ID,
        "kind": "publicInstanceMethod",
        "publicPath": "managedLlm.send",
        "publicInstanceKey": "managedLlm",
        "interface": interface,
        "methodAbiId": method_abi_id,
        "displayName": "managedLlm.send"
    })
}

fn callee_public_instance_signature() -> serde_json::Value {
    serde_json::json!({
        "params": [{
            "name": "input",
            "ty": { "kind": "builtin", "name": "string" }
        }],
        "returnType": { "kind": "builtin", "name": "Json" },
        "maySuspend": false
    })
}

fn callee_public_instance_publication_abi() -> serde_json::Value {
    let operation_ref = callee_public_instance_operation_ref();
    let interface = caller_llm_client_interface_ref();
    serde_json::json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": "skiff.run/account",
        "version": "0.1.0",
        "abiIdentity": "skiff-publication-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "operationExports": [operation_ref.clone()],
        "operationAbi": [{
            "operation": operation_ref.clone(),
            "publicSignature": callee_public_instance_signature()
        }],
        "sourceCallOperationIndex": [{
            "sourceCallPath": "managedLlm.send",
            "operation": operation_ref.clone()
        }],
        "publicInstances": [{
            "publicInstanceKey": "managedLlm",
            "interfaces": [interface],
            "sourceCallMethodIndex": [{
                "methodName": "send",
                "operation": operation_ref.clone()
            }],
            "methodOperations": [operation_ref]
        }]
    })
}

fn write_callee_public_instance_service_artifact_root(root: &Path) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_publication_abi(
        root,
        callee_public_instance_publication_abi(),
        callee_public_instance_operation_ref(),
    )
}

fn callee_multi_interface_public_instance_publication_abi() -> serde_json::Value {
    let mut publication_abi = callee_public_instance_publication_abi();
    publication_abi["publicInstances"][0]["interfaces"] = serde_json::json!([
        caller_llm_client_interface_ref(),
        caller_interface_ref("StreamingClient")
    ]);
    publication_abi
}

fn write_callee_multi_interface_public_instance_service_artifact_root(
    root: &Path,
) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_publication_abi(
        root,
        callee_multi_interface_public_instance_publication_abi(),
        callee_public_instance_operation_ref(),
    )
}

fn unsafe_public_instance_operation_ref() -> serde_json::Value {
    let interface = caller_interface_ref("UnsafeClient");
    let method_abi_id = caller_method_abi_id("UnsafeClient", "send");
    serde_json::json!({
        "operationAbiId": CALLEE_PUBLIC_INSTANCE_OPERATION_ABI_ID,
        "kind": "publicInstanceMethod",
        "publicPath": "managedLlm.send",
        "publicInstanceKey": "managedLlm",
        "interface": interface,
        "methodAbiId": method_abi_id,
        "displayName": "managedLlm.send"
    })
}

fn unsafe_public_instance_signature() -> serde_json::Value {
    let tool_interface = caller_interface_ref("Tool");
    serde_json::json!({
        "params": [{
            "name": "tool",
            "ty": {
                "kind": "anyInterface",
                "interface": tool_interface
            }
        }],
        "returnType": { "kind": "builtin", "name": "Json" },
        "maySuspend": false
    })
}

fn unsafe_public_instance_publication_abi() -> serde_json::Value {
    let operation_ref = unsafe_public_instance_operation_ref();
    let interface = caller_interface_ref("UnsafeClient");
    serde_json::json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": "skiff.run/account",
        "version": "0.1.0",
        "abiIdentity": "skiff-publication-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "operationExports": [operation_ref.clone()],
        "operationAbi": [{
            "operation": operation_ref.clone(),
            "publicSignature": unsafe_public_instance_signature()
        }],
        "sourceCallOperationIndex": [{
            "sourceCallPath": "managedLlm.send",
            "operation": operation_ref.clone()
        }],
        "publicInstances": [{
            "publicInstanceKey": "managedLlm",
            "interfaces": [interface],
            "sourceCallMethodIndex": [{
                "methodName": "send",
                "operation": operation_ref.clone()
            }],
            "methodOperations": [operation_ref]
        }]
    })
}

fn write_callee_unsafe_public_instance_service_artifact_root(root: &Path) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_publication_abi(
        root,
        unsafe_public_instance_publication_abi(),
        unsafe_public_instance_operation_ref(),
    )
}

fn duplicate_public_instance_operation_ref() -> serde_json::Value {
    let mut duplicate = callee_public_instance_operation_ref();
    duplicate["operationAbiId"] = serde_json::Value::String(
        "operation:skiff.run/account:managedLlm.sendDuplicate".to_string(),
    );
    duplicate
}

fn duplicate_public_instance_publication_abi() -> serde_json::Value {
    let first = callee_public_instance_operation_ref();
    let duplicate = duplicate_public_instance_operation_ref();
    serde_json::json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": "skiff.run/account",
        "version": "0.1.0",
        "abiIdentity": "skiff-publication-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "operationExports": [first.clone(), duplicate.clone()],
        "operationAbi": [{
            "operation": first.clone(),
            "publicSignature": callee_public_instance_signature()
        }, {
            "operation": duplicate.clone(),
            "publicSignature": callee_public_instance_signature()
        }],
        "sourceCallOperationIndex": [{
            "sourceCallPath": "managedLlm.send",
            "operation": first.clone()
        }],
        "publicInstances": [{
            "publicInstanceKey": "managedLlm",
            "interfaces": [caller_llm_client_interface_ref()],
            "sourceCallMethodIndex": [{
                "methodName": "send",
                "operation": first
            }, {
                "methodName": "send",
                "operation": duplicate.clone()
            }],
            "methodOperations": [duplicate]
        }]
    })
}

fn write_callee_duplicate_public_instance_service_artifact_root(
    root: &Path,
) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_publication_abi(
        root,
        duplicate_public_instance_publication_abi(),
        callee_public_instance_operation_ref(),
    )
}

fn write_callee_service_artifact_root_with_operation_kind(
    root: &Path,
    operation_kind: &str,
) -> CalleeServiceArtifact {
    write_callee_service_artifact_root_with_publication_abi(
        root,
        callee_publication_abi_with_operation_kind(operation_kind),
        callee_operation_ref_with_kind(operation_kind),
    )
}

fn write_callee_service_artifact_root_with_publication_abi(
    root: &Path,
    publication_abi: serde_json::Value,
    operation_ref: serde_json::Value,
) -> CalleeServiceArtifact {
    let service_unit = serde_json::json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": {
            "id": "skiff.run/account",
            "displayName": "Account"
        },
        "version": "0.1.0",
        "protocolIdentity": "skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "publicationAbi": publication_abi,
        "files": [{
            "fileIrIdentity": "skiff-file-ir-v3:sha256:2222222222222222222222222222222222222222222222222222222222222222",
            "modulePath": "internal.account",
            "artifactPath": "units/files/account.json"
        }],
        "operations": [{
            "kind": "localExecutable",
            "operation": operation_ref,
            "executable": {
                "fileRef": {
                    "fileIrIdentity": "skiff-file-ir-v3:sha256:2222222222222222222222222222222222222222222222222222222222222222",
                    "modulePath": "internal.account",
                    "artifactPath": "units/files/account.json"
                },
                "executableIndex": 0,
                "callableAbiId": "callable:internal.account.UserApi.get",
                "callableKind": "publicFunction"
            }
        }],
        "gateway": {},
        "config": {}
    });
    let service_unit_hash = sha256_json(&service_unit);
    let service_unit_path = format!("units/services/skiff~run~~account/{service_unit_hash}.json");
    write_json(root, &service_unit_path, &service_unit);
    let build_id = dynamic_build_id_for_test_service_unit(&service_unit)
        .unwrap_or_else(|| format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{service_unit_hash}"));
    write_json(
        root,
        "dev/services/skiff~run~~account.json",
        &serde_json::json!({
            "mode": "dev",
            "serviceId": "skiff.run/account",
            "serviceVersion": "0.1.0",
            "profile": "test",
            "buildId": build_id,
            "contractHash": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "protocolIdentity": service_unit["protocolIdentity"],
            "serviceAssembly": {
                "assemblyIdentity": "skiff-service-assembly-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333",
                "assemblyPath": "assemblies/services/skiff~run~~account/3333333333333333333333333333333333333333333333333333333333333333.json"
            },
            "serviceUnit": {
                "schemaVersion": "skiff-service-unit-v1",
                "unitIdentity": format!("{SERVICE_UNIT_IDENTITY_PREFIX}:{service_unit_hash}"),
                "unitHash": service_unit_hash,
                "unitPath": service_unit_path
            }
        }),
    );

    CalleeServiceArtifact {
        build_id,
        service_protocol_identity: service_unit["protocolIdentity"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

fn dynamic_build_id_for_test_service_unit(service_unit: &serde_json::Value) -> Option<String> {
    let bytes = runtime_program_service_unit_identity_bytes_from_json(service_unit).ok()?;
    Some(runtime_program_dynamic_build_id(&bytes, []))
}

fn write_json(root: &Path, relative_path: &str, value: &serde_json::Value) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

fn sha256_json(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical_json(value)).unwrap(),
    ))
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut sorted = serde_json::Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonical_json(&object[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_json).collect())
        }
        _ => value.clone(),
    }
}

fn json_file_count(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    json_files(path).len()
}

fn first_json_file(path: &Path) -> Option<PathBuf> {
    json_files(path).into_iter().next()
}

fn write_minimal_service_project(service_root: &Path, service_id: &str) {
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        format!(
            r#"
id: {service_id}
version: 1.0.0
"#
        ),
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("example.skiff"),
        r#"
type Output {}
interface ExampleService {
  function ping() -> Output
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("example.skiff"),
        r#"
type ExampleService {}

impl ExampleService {
  function ping(self: ExampleService) -> root.api.example.Output {
    return {}
  }
}
"#,
    )
    .unwrap();
}

fn write_mongo_dependent_service_project(service_root: &Path, service_id: &str) {
    write_minimal_service_project(service_root, service_id);
    fs::write(
        service_root.join("service.yml"),
        format!(
            r#"
id: {service_id}
version: 1.0.0
packages:
  - id: skiff.run/mongo
    version: 1.0.0
    alias: mongo
"#
        ),
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("example.skiff"),
        r#"
import mongo

type ExampleService {}

impl ExampleService {
  function ping(self: ExampleService) -> root.api.example.Output {
    const target = mongo.Target("test-cluster", "example")
    return {}
  }
}
"#,
    )
    .unwrap();
}

fn write_legacy_mongo_package(packages_dir: &Path) {
    let package_root = packages_dir.join("skiff~run~~mongo").join("1.0.0");
    fs::create_dir_all(&package_root).unwrap();
    fs::write(
        package_root.join("package.yml"),
        r#"
id: skiff.run/mongo
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_root.join("api.yml"),
        r#"
MongoTarget: mongo.MongoTarget
"#,
    )
    .unwrap();
    fs::write(
        package_root.join("mongo.skiff"),
        r#"
provider mongo

export type MongoTarget {}
"#,
    )
    .unwrap();
}

fn json_files(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json_files(path, &mut files);
    files.sort();
    files
}

fn collect_json_files(path: &Path, files: &mut Vec<PathBuf>) {
    if !path.exists() {
        return;
    }
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_json_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            files.push(path);
        }
    }
}

fn assembly_path_from_identity(service_id: &str, identity: &str) -> String {
    let hash = identity.rsplit_once(":sha256:").unwrap().1;
    format!(
        "assemblies/services/{}/{hash}.json",
        publication_storage_segment(service_id)
    )
}

fn publication_storage_segment(publication_id: &str) -> String {
    publication_id.replace('.', "~").replace('/', "~~")
}
