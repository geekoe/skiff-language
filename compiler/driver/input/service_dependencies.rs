use std::path::{Path, PathBuf};

use skiff_compiler_core::artifact::ServiceUnit;
use skiff_compiler_emission::identity::runtime_program_dynamic_build_id_from_artifact_root;

use crate::{
    input::{ResolvedServiceDependencies, ServiceDependency},
    shared::publication_error::PublicationError,
};

pub(crate) fn resolve_service_dependencies(
    dependencies: &[ServiceDependency],
    artifact_roots: &[PathBuf],
) -> Result<ResolvedServiceDependencies, PublicationError> {
    skiff_compiler_input::service_dependencies::resolve_service_dependencies(
        dependencies,
        artifact_roots,
        dynamic_build_id,
    )
    .map_err(PublicationError::from)
}

pub(crate) fn dynamic_build_id(root: &Path, service_unit: &ServiceUnit) -> Result<String, String> {
    runtime_program_dynamic_build_id_from_artifact_root(root, service_unit)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde::Deserialize;
    use serde_json::{Map, Value};
    use skiff_compiler_core::artifact::ServiceUnit;

    use super::dynamic_build_id;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DynamicBuildIdFixture {
        applies_to: Vec<String>,
        service_unit_path: String,
        expected_dynamic_build_id: String,
        expected_service_unit_identity: String,
        artifact_root: BTreeMap<String, Value>,
    }

    #[test]
    fn dynamic_build_id_matches_cross_system_fixture() {
        let fixture = dynamic_build_id_fixture();
        assert!(fixture.applies_to.iter().any(|system| system == "compiler"));
        assert_fixture_contains_type_ref(
            &fixture.artifact_root,
            "packageSymbol",
            "std.http.HttpClientRequest",
        );
        assert_fixture_contains_type_ref(
            &fixture.artifact_root,
            "packageSymbol",
            "std.http.HttpResponseStreamEvent",
        );
        assert_fixture_contains_type_ref(
            &fixture.artifact_root,
            "packageSymbol",
            "std.file.ImmutableFile",
        );
        assert_fixture_contains_type_ref(&fixture.artifact_root, "builtin", "bytes");
        assert_fixture_service_unit_array_is_non_empty(&fixture, "spawnTargets");
        assert_fixture_service_unit_array_is_non_empty(&fixture, "actors");
        assert_eq!(
            fixture_operation_target(&fixture, 0)
                .get("executableIndex")
                .and_then(Value::as_u64),
            Some(0)
        );

        let temp = TempDir::new("compiler-dynamic-build-id-fixture");
        write_fixture_artifact_root(temp.path(), &fixture);
        let service_unit = fixture
            .artifact_root
            .get(&fixture.service_unit_path)
            .expect("fixture service unit path should exist");

        let service_unit: ServiceUnit = serde_json::from_value(service_unit.clone())
            .expect("fixture service unit should parse");
        let build_id =
            dynamic_build_id(temp.path(), &service_unit).expect("fixture dynamic build id");
        assert_eq!(build_id, fixture.expected_dynamic_build_id);

        assert_eq!(
            service_unit_identity(&service_unit),
            fixture.expected_service_unit_identity
        );
    }

    fn service_unit_identity(unit: &ServiceUnit) -> String {
        skiff_compiler_emission::identity::service_unit_identity(unit)
            .expect("service unit identity should compute")
    }

    fn dynamic_build_id_fixture() -> DynamicBuildIdFixture {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler crate should live under the skiff repository root")
            .join("cross-system-fixtures/dynamic-build-id-parity/case.json");
        let text = fs::read_to_string(&path).expect("dynamic build id fixture should be readable");
        serde_json::from_str(&text).expect("dynamic build id fixture should parse")
    }

    fn write_fixture_artifact_root(root: &Path, fixture: &DynamicBuildIdFixture) {
        for (relative_path, value) in &fixture.artifact_root {
            write_json(root, relative_path, value);
        }
    }

    fn write_json(root: &Path, relative_path: &str, value: &Value) {
        let path = root.join(relative_path);
        fs::create_dir_all(
            path.parent()
                .expect("fixture artifact path should have parent"),
        )
        .expect("fixture artifact directory should be created");
        fs::write(
            &path,
            serde_json::to_vec_pretty(value).expect("fixture JSON should serialize"),
        )
        .expect("fixture artifact should be written");
    }

    fn assert_fixture_contains_type_ref(
        artifact_root: &BTreeMap<String, Value>,
        kind: &str,
        symbol: &str,
    ) {
        assert!(
            json_contains_type_ref(&serde_json::to_value(artifact_root).unwrap(), kind, symbol),
            "dynamic build id fixture should contain {kind} type ref {symbol}"
        );
    }

    fn assert_fixture_service_unit_array_is_non_empty(
        fixture: &DynamicBuildIdFixture,
        field: &str,
    ) {
        let items = fixture
            .artifact_root
            .get(&fixture.service_unit_path)
            .and_then(|service_unit| service_unit.get(field))
            .and_then(Value::as_array)
            .expect("dynamic build id fixture service unit field should be an array");
        assert!(
            !items.is_empty(),
            "dynamic build id fixture should cover service unit {field}"
        );
    }

    fn fixture_operation_target(
        fixture: &DynamicBuildIdFixture,
        operation_index: usize,
    ) -> &Map<String, Value> {
        fixture
            .artifact_root
            .get(&fixture.service_unit_path)
            .and_then(|service_unit| service_unit.get("operations"))
            .and_then(Value::as_array)
            .and_then(|operations| operations.get(operation_index))
            .and_then(|operation| {
                operation.get("executable").or_else(|| {
                    operation
                        .get("receiverExecutable")
                        .and_then(|target| target.get("executableTarget"))
                })
            })
            .and_then(Value::as_object)
            .expect("dynamic build id fixture operation target should be an object")
    }

    fn json_contains_type_ref(value: &Value, kind: &str, symbol: &str) -> bool {
        match value {
            Value::Array(values) => values
                .iter()
                .any(|value| json_contains_type_ref(value, kind, symbol)),
            Value::Object(object) => {
                (object.get("kind").and_then(Value::as_str) == Some(kind)
                    && match kind {
                        "builtin" => object.get("name").and_then(Value::as_str) == Some(symbol),
                        "packageSymbol" => {
                            object
                                .get("symbol")
                                .and_then(|value| value.get("symbolPath"))
                                .and_then(Value::as_str)
                                == Some(symbol)
                        }
                        _ => false,
                    })
                    || object
                        .values()
                        .any(|value| json_contains_type_ref(value, kind, symbol))
            }
            _ => false,
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
