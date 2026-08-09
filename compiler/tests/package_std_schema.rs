use skiff_artifact_model::{ContractTypeDescriptor, PackageLocalAbiSymbol, PackageRefIr};

mod common;
use common::{
    artifacts::{module_artifact, source_artifact},
    package_project::compile_package_project,
    TestDir,
};

fn write_models_package(temp: &TestDir) {
    temp.write(
        ".skiff-packages/example~com~~models/0.1.0/package.yml",
        "id: example.com/models\nversion: 0.1.0\n",
    );
    temp.write(
        ".skiff-packages/example~com~~models/0.1.0/api.yml",
        "ModelRequest: models.ModelRequest\nmake: models.make\n",
    );
    temp.write(
        ".skiff-packages/example~com~~models/0.1.0/models.skiff",
        r#"type ModelRequest {}

function make() -> ModelRequest {
  return {}
}
"#,
    );
}

fn assert_std_file_ir_symbol(
    package: &skiff_compiler::PublishedPackageArtifact,
    module_path: &str,
    symbol_path: &str,
) {
    let file = module_artifact(package, module_path);
    assert!(
        file.unit
            .external_refs
            .package_symbols
            .iter()
            .any(|symbol| {
                symbol.symbol_path == symbol_path
                    && matches!(
                        &symbol.package,
                        PackageRefIr::PackageId { package_id } if package_id == "skiff.run/std"
                    )
            }),
        "File IR module {module_path} should reference canonical std symbol {symbol_path}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_nominal_records_keep_public_schema_fields() {
        let temp = TestDir::new("skiff-compiler", "nominal-record-schema-fields");
        temp.write("package.yml", "id: example.com/errors\nversion: 1.0.0\n");
        temp.write(
            "api.yml",
            "Ordinary: errors.Ordinary\nFailure: errors.Failure\n",
        );
        temp.write(
            "errors.skiff",
            r#"type Ordinary {
  value: string,
}

type Failure {
  code: string,
  message: string,
}
"#,
        );

        let project = compile_package_project(temp.path()).expect("record schemas should compile");
        let records = &project.package.package_schema_type_records;
        for (stable_key, expected) in [
            ("Ordinary", vec!["value"]),
            ("Failure", vec!["code", "message"]),
        ] {
            let entry = &project.package.package_schema_index.types[stable_key];
            let record = &records[&entry.package_schema_type_id];
            let ContractTypeDescriptor::Record { fields } = &record.canonical_descriptor.descriptor
            else {
                panic!("{stable_key} must remain a record");
            };
            assert_eq!(
                fields.keys().map(String::as_str).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn package_schema_references_require_a_declared_dependency() {
        for (fixture, api, source) in [
            (
                "record-field",
                "Envelope: schema.Envelope\n",
                r#"import models

type Envelope { request: models.ModelRequest }
"#,
            ),
            (
                "top-level-constant",
                "request: schema.request\n",
                r#"import models

const request: models.ModelRequest = {}
"#,
            ),
            (
                "package-expression",
                "make: schema.make\n",
                r#"import models

const make = models.make
"#,
            ),
            (
                "generic-function-type",
                "CallbackBag: schema.CallbackBag\n",
                r#"import models

type CallbackBag { callbacks: Array<fn(input: models.ModelRequest) -> void> }
"#,
            ),
        ] {
            let temp = TestDir::new("skiff-compiler", &format!("std-schema-{fixture}"));
            temp.write("package.yml", "id: example.com/schema\nversion: 1.0.0\n");
            temp.write("api.yml", api);
            temp.write("schema.skiff", source);

            let error = compile_package_project(temp.path())
                .expect_err("an undeclared package schema dependency must fail")
                .to_string();
            assert!(error.contains("import models"), "unexpected error: {error}");
            assert!(error.contains("packages"), "unexpected error: {error}");
        }
    }

    #[test]
    fn platform_std_schema_types_are_available_without_a_manifest_requirement() {
        let temp = TestDir::new("skiff-compiler", "implicit-platform-std-schema");
        temp.write(
            "package.yml",
            "id: example.com/http-schema\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "RequestEnvelope: schema.RequestEnvelope\n");
        temp.write(
            "schema.skiff",
            r#"type RequestEnvelope { request: std.http.HttpClientRequest }
"#,
        );

        let project =
            compile_package_project(temp.path()).expect("platform std schema should compile");
        assert_std_file_ir_symbol(&project.package, "schema", "std.http.HttpClientRequest");
        assert!(project.dependency("skiff.run/std", "1.0.0").is_some());
    }

    #[test]
    fn platform_std_rejects_a_user_dependency_alias() {
        let temp = TestDir::new("skiff-compiler", "explicit-platform-std-alias");
        temp.write(
            "package.yml",
            r#"id: example.com/schema
version: 1.0.0
packages:
  - id: skiff.run/std
    version: 1.0.0
    alias: corelib
"#,
        );
        temp.write("api.yml", "Envelope: schema.Envelope\n");
        temp.write(
            "schema.skiff",
            r#"import corelib

type Envelope { request: corelib.http.HttpClientRequest }
"#,
        );

        let error = compile_package_project(temp.path())
            .expect_err("platform std must not become a user dependency alias")
            .to_string();
        assert!(
            error.contains("platform std is built into the compiler"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn package_schema_dependencies_reach_file_ir_artifact_and_closure() {
        let temp = TestDir::new("skiff-compiler", "canonical-package-schema");
        temp.write(
            "package.yml",
            r#"id: example.com/schema
version: 1.0.0
packages:
  - id: example.com/models
    version: 0.1.0
    alias: models
"#,
        );
        temp.write("api.yml", "Envelope: schema.Envelope\n");
        temp.write(
            "schema.skiff",
            r#"import models
import std

type Envelope {
  model: models.ModelRequest,
  request: std.http.HttpClientRequest,
  callback: fn(input: models.ModelRequest) -> void,
}
"#,
        );
        write_models_package(&temp);

        let project =
            compile_package_project(temp.path()).expect("package schema graph should compile");
        let schema = source_artifact(&project.package, "schema.skiff");
        assert!(schema.unit.declarations.types.contains_key("Envelope"));
        assert!(schema
            .unit
            .external_refs
            .package_symbols
            .iter()
            .any(|symbol| {
                symbol.symbol_path == "ModelRequest"
                    && matches!(
                        &symbol.package,
                        PackageRefIr::Dependency { dependency_ref } if dependency_ref == "models"
                    )
            }));
        assert_std_file_ir_symbol(&project.package, "schema", "std.http.HttpClientRequest");

        let requirement_ids = project
            .package
            .artifact
            .package_requirements
            .iter()
            .map(|requirement| requirement.package_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(requirement_ids, vec!["example.com/models", "skiff.run/std"]);
        assert_eq!(
            project.package.artifact.implementation_links.types["Envelope"].symbol,
            "Envelope"
        );
        assert!(project.dependency("example.com/models", "0.1.0").is_some());
        assert!(project.dependency("skiff.run/std", "1.0.0").is_some());
    }

    #[test]
    fn sibling_type_refs_are_canonical_in_package_local_abi() {
        let temp = TestDir::new("skiff-compiler", "canonical-sibling-type-ref");
        temp.write(
            "package.yml",
            "id: example.com/direct-ref\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "echo: api.echo\n");
        temp.write(
            "api.skiff",
            r#"function echo(input: root.models.Payload) -> root.models.Payload {
  return input
}
"#,
        );
        temp.write("models.skiff", "type Payload { value: string }\n");

        let project =
            compile_package_project(temp.path()).expect("sibling type refs should compile");
        let api_file = module_artifact(&project.package, "api").value();
        assert!(
            api_file.to_string().contains("publicationType"),
            "File IR should retain the sibling-module reference: {api_file}"
        );

        let abi = serde_json::to_value(&project.package.artifact.package_local_abi).unwrap();
        let abi_text = abi.to_string();
        for forbidden in ["publicationType", "$type", "__unresolved_publication_type"] {
            assert!(
                !abi_text.contains(forbidden),
                "package-local ABI leaked {forbidden}: {abi}"
            );
        }
        assert!(matches!(
            project
                .package
                .artifact
                .package_local_abi
                .public_symbols
                .get("echo"),
            Some(PackageLocalAbiSymbol::Callable { .. })
        ));
    }

    #[test]
    fn std_discriminator_union_field_access_compiles_in_file_ir() {
        let temp = TestDir::new("skiff-compiler", "std-discriminator-field-access");
        temp.write("package.yml", "id: example.com/http-sse\nversion: 1.0.0\n");
        temp.write("api.yml", "eventStatus: sse.eventStatus\n");
        temp.write(
            "sse.skiff",
            r#"import std

function eventStatus(event: std.http.HttpSseEvent) -> integer? {
  if event.tag == "response" {
    return event.status
  }
  if event.tag == "event" {
    let data = event.data
    if data == "" {
      return null
    }
  }
  return null
}
"#,
        );

        let project =
            compile_package_project(temp.path()).expect("std discriminator access should compile");
        let file = module_artifact(&project.package, "sse");
        assert!(file
            .unit
            .declarations
            .executables
            .contains_key("eventStatus"));
        assert_std_file_ir_symbol(&project.package, "sse", "std.http.HttpSseEvent");
    }

    #[test]
    fn bare_http_envelopes_remain_prelude_schema_types() {
        let temp = TestDir::new("skiff-compiler", "bare-http-envelope");
        temp.write("package.yml", "id: example.com/raw\nversion: 1.0.0\n");
        temp.write(
            "api.yml",
            "rawRequest: raw.rawRequest\nRawEnvelope: raw.RawEnvelope\n",
        );
        temp.write(
            "raw.skiff",
            r#"const rawRequest: string = "GET"

type RawEnvelope {
  request: HttpRequest,
  response: HttpResponse,
}
"#,
        );

        let project =
            compile_package_project(temp.path()).expect("bare HTTP envelopes should compile");
        let raw = module_artifact(&project.package, "raw");
        assert_eq!(raw.unit.declarations.constants["rawRequest"].const_index, 0);
        assert!(raw.unit.declarations.types.contains_key("RawEnvelope"));
        assert!(project.dependency("skiff.run/std", "1.0.0").is_some());
    }
}
