mod common;
use common::{package_project::compile_package_project, TestDir};

fn package_with_source(name: &str, source: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    temp.write(
        "package.yml",
        "id: example.com/config-fixture\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "configured: main.configured\n");
    temp.write("main.skiff", source);
    temp
}

fn compile_error(temp: TestDir) -> String {
    compile_package_project(temp.path())
        .expect_err("package compile should fail")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_config_reads_project_to_runtime_requirements() {
        let temp = package_with_source(
            "config-runtime-requirements",
            r#"
function configured() -> string {
  final token = config.require<string>("app.token")
  final region = config.optional<string>("app.region")
  final enabled = config.has("app.enabled")
  return token
}
"#,
        );
        let project = compile_package_project(temp.path()).expect("package should compile");

        assert_eq!(
            project.package.artifact.runtime_requirements.config,
            vec![
                skiff_artifact_model::PackageConfigRequirement {
                    path: "app.enabled".to_string(),
                    access: skiff_artifact_model::PackageConfigAccess::Presence,
                },
                skiff_artifact_model::PackageConfigRequirement {
                    path: "app.region".to_string(),
                    access: skiff_artifact_model::PackageConfigAccess::Optional {
                        value_type: "string".to_string(),
                    },
                },
                skiff_artifact_model::PackageConfigRequirement {
                    path: "app.token".to_string(),
                    access: skiff_artifact_model::PackageConfigAccess::Required {
                        value_type: "string".to_string(),
                    },
                },
            ]
        );
        let skiff_artifact_model::PackageLocalAbiSymbol::Callable { callable_id, .. } =
            &project.package.artifact.package_local_abi.public_symbols["configured"]
        else {
            panic!("configured must resolve to a public callable");
        };
        let skiff_artifact_model::BoundaryCallableProjection::Available {
            implementation_requirements,
            ..
        } = &project.package.artifact.boundary_projections[callable_id]
        else {
            panic!("direct config intrinsic caller must be boundary available");
        };
        assert_eq!(
            implementation_requirements.config,
            vec![
                skiff_artifact_model::BoundaryConfigRequirement {
                    path: "app.region".to_string(),
                    value_type: "string".to_string(),
                    required: false,
                },
                skiff_artifact_model::BoundaryConfigRequirement {
                    path: "app.token".to_string(),
                    value_type: "string".to_string(),
                    required: true,
                },
            ]
        );
        let semantic_facts = &project.package.artifact.callable_semantic_facts[callable_id];
        assert!(
            semantic_facts.resolved_call_targets.is_empty(),
            "config intrinsic must not publish an external or unknown target fact"
        );
    }

    #[test]
    fn package_test_sources_do_not_enter_runtime_config_requirements() {
        let temp = package_with_source(
            "config-test-source",
            "function configured() -> string { return config.require<string>(\"prod.token\") }\n",
        );
        temp.write(
            "main.test.skiff",
            r#"
const testToken = config.require<string>("test.token")
test "config helper" { assert testToken == testToken, "same token" }
"#,
        );

        let project = compile_package_project(temp.path()).expect("package should compile");
        let requirements = &project.package.artifact.runtime_requirements.config;
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].path, "prod.token");
    }

    #[test]
    fn config_intrinsics_reject_noncanonical_paths_types_and_conflicts() {
        for (name, source, expected) in [
            (
                "config-dynamic-path",
                r#"function configured(path: string) -> string { return config.require<string>(path) }"#,
                "const-foldable",
            ),
            (
                "config-empty-path",
                r#"function configured() -> string { return config.require<string>("") }"#,
                "path cannot be empty",
            ),
            (
                "config-invalid-path",
                r#"function configured() -> string { return config.require<string>("9invalid") }"#,
                "invalid segment",
            ),
            (
                "config-unsupported-type",
                r#"function configured() -> Array<string> { return config.require<Array<string>>("app.items") }"#,
                "is unsupported",
            ),
            (
                "config-conflicting-types",
                r#"
function configured() -> string {
  final text = config.require<string>("app.value")
  final number = config.optional<number>("app.value")
  return text
}
"#,
                "conflicting",
            ),
        ] {
            let error = compile_error(package_with_source(name, source));
            assert!(
                error.contains(expected),
                "expected {expected:?} in compile error: {error}"
            );
        }
    }

    #[test]
    fn config_values_are_not_package_manifest_data() {
        let temp = package_with_source(
            "package-config-values-rejected",
            "function configured() -> string { return \"ok\" }\n",
        );
        temp.write(
            "package.yml",
            "id: example.com/config-fixture\nversion: 1.0.0\nconfig:\n  app.token: secret\n",
        );
        let error = compile_error(temp);
        assert!(
            error.contains("unknown field `config`"),
            "package-owned config values must be rejected: {error}"
        );
    }

    #[test]
    fn removed_values_root_and_reserved_config_bindings_are_rejected() {
        let values = package_with_source(
            "removed-values-root",
            r#"function configured() -> string { return values.string("app.token") }"#,
        );
        let values_error = compile_error(values);
        assert!(values_error.contains("values.* has been removed"));

        let binding = package_with_source(
            "reserved-config-binding",
            r#"
function configured() -> string {
  final config = "shadow"
  return config
}
"#,
        );
        let binding_error = compile_error(binding);
        assert!(
            binding_error.contains("local binding config")
                && binding_error.contains("reserved prelude name")
        );
    }

    #[test]
    fn config_intrinsic_aliases_indirect_calls_and_unknown_methods_remain_rejected() {
        for (name, source, expected) in [
            (
                "config-alias",
                r#"
function configured() -> string? {
  final read = config.optional<string>
  return read("app.token")
}
"#,
                "cannot be aliased or called indirectly",
            ),
            (
                "config-unknown-method",
                r#"function configured() -> string { return config.unknown("app.token") }"#,
                "config only exposes direct",
            ),
        ] {
            let error = compile_error(package_with_source(name, source));
            assert!(
                error.contains(expected),
                "expected {expected:?} in compile error: {error}"
            );
        }
    }
}
