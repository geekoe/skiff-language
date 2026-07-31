mod common;

use std::{fs, path::Path};

use common::{
    artifacts::module_artifact,
    package_project::{compile_package_project, compile_service_package_project},
    TestDir,
};
use skiff_artifact_model::PackageLocalAbiSymbol;
use skiff_compiler_input::package_config::read_user_package_manifest;
use skiff_syntax::parser::parse_source;

fn collect_foreign_db_targets<'a>(
    value: &'a serde_json::Value,
    targets: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("typeRef")
                && object.contains_key("typeName")
                && object["typeRef"]["kind"] == "packageSymbol"
            {
                targets.push(value);
            }
            for child in object.values() {
                collect_foreign_db_targets(child, targets);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_foreign_db_targets(child, targets);
            }
        }
        _ => {}
    }
}

fn assert_file_ir_contains_package_callable(
    package: &skiff_compiler::PublishedPackageArtifact,
    module_path: &str,
    dependency_ref: &str,
    package_id: &str,
    public_path: &str,
) {
    let file = module_artifact(package, module_path);
    let expected_id = format!("pkg-callable:{package_id}:{public_path}");
    assert!(
        file.unit
            .external_refs
            .package_callables
            .iter()
            .any(|callable| {
                matches!(
                    &callable.package_ref,
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref: actual }
                        if actual == dependency_ref
                ) && callable.package_callable_id.as_str() == expected_id
            }),
        "File IR module {module_path} should reference {dependency_ref}:{expected_id}: {}",
        file.value()
    );
}

fn write_cloud_dependency(root: &Path) {
    let cloud = root.join(".skiff-packages/google~com~~cloud/0.1.0");
    fs::create_dir_all(cloud.join("cloud")).unwrap();
    fs::write(
        cloud.join("package.yml"),
        "id: google.com/cloud\nversion: 0.1.0\n",
    )
    .unwrap();
    fs::write(
        cloud.join("api.yml"),
        "compute:\n  start: cloud.compute.start\nstorage:\n  upload: cloud.storage.upload\n",
    )
    .unwrap();
    fs::write(
        cloud.join("cloud/compute.skiff"),
        "function start() -> string { return \"ok\" }\n",
    )
    .unwrap();
    fs::write(
        cloud.join("cloud/storage.skiff"),
        "function upload() -> string { return \"ok\" }\n",
    )
    .unwrap();
}

fn write_llm_dependency(root: &Path, api: &str) {
    let llm = root.join(".skiff-packages/skiff~run~~llm/0.1.0");
    fs::create_dir_all(&llm).unwrap();
    fs::write(
        llm.join("package.yml"),
        "id: skiff.run/llm\nversion: 0.1.0\n",
    )
    .unwrap();
    fs::write(llm.join("api.yml"), api).unwrap();
    fs::write(
        llm.join("llm_impl.skiff"),
        "function chat() -> string { return \"ok\" }\n",
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_import_syntax_accepts_only_one_identifier() {
        for source in [
            "import std as foo\nfunction run() -> number { return 1 }",
            "import google.com/cloud\nfunction run() -> number { return 1 }",
            "import google.com/cloud as gcloud\nfunction run() -> number { return 1 }",
            "import google/cloud\nfunction run() -> number { return 1 }",
            "import 123\nfunction run() -> number { return 1 }",
        ] {
            let error = parse_source(source).unwrap_err().to_string();
            assert!(
                error.contains("import name must be a single ASCII identifier"),
                "unexpected import error: {error}"
            );
        }

        let ast = parse_source("import billing\nfunction run() -> number { return 1 }")
            .expect("simple import should parse");
        assert_eq!(ast.imports[0].alias, None);
        assert_eq!(ast.imports[0].local_binding.as_deref(), Some("billing"));
    }

    #[test]
    fn package_manifest_rejects_removed_fields_and_unsafe_ids() {
        for (field, yaml) in [
            ("transports", "transports: [legacy]"),
            ("providers", "providers: []"),
            ("effects", "effects:\n  symbols: {}"),
            (
                "publicEffects",
                "publicEffects:\n  example.com/removed.run:\n    target: example.com/removed.run",
            ),
        ] {
            let temp = TestDir::new("skiff-compiler", field);
            fs::write(
                temp.path().join("package.yml"),
                format!("id: example.com/removed\nversion: 0.1.0\n{yaml}\n"),
            )
            .unwrap();
            fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
            let error = read_user_package_manifest(&temp.path().join("package.yml"))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("unknown field `{field}`")),
                "unexpected manifest error: {error}"
            );
        }

        let unsafe_id = TestDir::new("skiff-compiler", "unsafe-package-id");
        fs::write(
            unsafe_id.path().join("package.yml"),
            "id: app/escape/extra\nversion: 0.1.0\n",
        )
        .unwrap();
        fs::write(unsafe_id.path().join("api.yml"), "{}\n").unwrap();
        let error = read_user_package_manifest(&unsafe_id.path().join("package.yml"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("id app/escape/extra"));
        assert!(error.contains("publication id"));
    }

    #[test]
    fn dependency_alias_projects_each_public_operation_into_file_ir() {
        let temp = TestDir::new("skiff-compiler", "complex-package-alias");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/import-app
    version: 1.0.0
    packages:
      - id: google.com/cloud
        version: 0.1.0
        alias: gcloud
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import gcloud
    function run() -> string {
      const stored = gcloud/storage.upload()
      return gcloud/compute.start()
    }
    "#,
        )
        .unwrap();
        write_cloud_dependency(temp.path());

        let project = compile_package_project(temp.path()).expect("alias graph should compile");
        let cloud = project
            .dependency("google.com/cloud", "0.1.0")
            .expect("cloud artifact should be in the dependency closure");
        let requirement = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.package_id == "google.com/cloud")
            .expect("root artifact should retain its canonical requirement");

        assert_eq!(requirement.alias, "gcloud");
        assert_eq!(
            requirement.expected_local_abi,
            cloud.artifact.package_local_abi.local_abi_identity
        );
        assert_file_ir_contains_package_callable(
            &project.package,
            "main",
            "gcloud",
            "google.com/cloud",
            "storage.upload",
        );
        assert_file_ir_contains_package_callable(
            &project.package,
            "main",
            "gcloud",
            "google.com/cloud",
            "compute.start",
        );
    }

    #[test]
    fn public_path_shape_is_preserved_under_dependency_alias() {
        let nested = TestDir::new("skiff-compiler", "nested-public-path");
        fs::write(
            nested.path().join("package.yml"),
            r#"id: example.com/nested-consumer
    version: 1.0.0
    packages:
      - id: skiff.run/llm
        version: 0.1.0
        alias: llm
    "#,
        )
        .unwrap();
        fs::write(nested.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            nested.path().join("main.skiff"),
            "import llm\nfunction run() -> string { return llm/llm.chat() }\n",
        )
        .unwrap();
        write_llm_dependency(nested.path(), "llm:\n  chat: llm_impl.chat\n");

        let project = compile_package_project(nested.path()).expect("nested export should compile");
        assert_file_ir_contains_package_callable(
            &project.package,
            "main",
            "llm",
            "skiff.run/llm",
            "llm.chat",
        );

        let folded = TestDir::new("skiff-compiler", "folded-public-path");
        fs::write(
            folded.path().join("package.yml"),
            r#"id: example.com/folded-consumer
    version: 1.0.0
    packages:
      - id: skiff.run/llm
        version: 0.1.0
        alias: llm
    "#,
        )
        .unwrap();
        fs::write(folded.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            folded.path().join("main.skiff"),
            "import llm\nfunction run() -> string { return llm/chat() }\n",
        )
        .unwrap();
        write_llm_dependency(folded.path(), "llm:\n  chat: llm_impl.chat\n");
        let error = compile_package_project(folded.path())
            .expect_err("folded shorthand should stay invalid")
            .to_string();
        assert!(
            error.contains("package dependency `llm` has no callable public path `chat`"),
            "unexpected error: {error}"
        );

        let flat = TestDir::new("skiff-compiler", "flat-public-path");
        fs::write(
            flat.path().join("package.yml"),
            r#"id: example.com/flat-consumer
    version: 1.0.0
    packages:
      - id: skiff.run/llm
        version: 0.1.0
        alias: llm
    "#,
        )
        .unwrap();
        fs::write(flat.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            flat.path().join("main.skiff"),
            "import llm\nfunction run() -> string { return llm/chat() }\n",
        )
        .unwrap();
        write_llm_dependency(flat.path(), "chat: llm_impl.chat\n");
        let project = compile_package_project(flat.path()).expect("flat export should compile");
        assert_file_ir_contains_package_callable(
            &project.package,
            "main",
            "llm",
            "skiff.run/llm",
            "chat",
        );
    }

    #[test]
    fn top_level_alias_keeps_public_and_source_views_explicit_on_one_dependency() {
        let temp = TestDir::new("skiff-compiler", "top-level-alias-explicit-views");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/widget-tests
    version: 1.0.0
    packages:
      - id: example.com/widget
        version: 1.0.0
        alias: widget
        topLevelAlias: widgetImpl
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import widget
    import widgetImpl

    function run() -> string {
      const publicValue = widget/internal.codec.reveal()
      const seed: widgetImpl/internal.codec.PrivateValue = widgetImpl/internal.codec.PRIVATE_VALUE
      const revealed: widgetImpl/internal.codec.PrivateValue = widgetImpl/internal.codec.reveal(seed)
      const contextual: widgetImpl/internal.codec.PrivateValue = widgetImpl/internal.codec.reveal({ value: "contextual" })
      return publicValue + revealed.value + contextual.value
    }

    function construct() -> widgetImpl/internal.codec.PrivateValue {
      return widgetImpl/internal.codec.PrivateValue { value: "constructed" }
    }
    "#,
        )
        .unwrap();
        let dependency = temp
            .path()
            .join(".skiff-packages/example~com~~widget/1.0.0");
        fs::create_dir_all(dependency.join("internal")).unwrap();
        fs::write(
            dependency.join("package.yml"),
            "id: example.com/widget\nversion: 1.0.0\n",
        )
        .unwrap();
        // The same path deliberately points at another implementation in api.yml.
        fs::write(
            dependency.join("api.yml"),
            "internal:\n  codec:\n    reveal: public_api.decoy\npublicOnly: public_api.decoy\n",
        )
        .unwrap();
        fs::write(
            dependency.join("internal/codec.skiff"),
            r#"
    type PrivateValue {
      value: string
    }

    const PRIVATE_VALUE: PrivateValue = PrivateValue { value: "private" }

    function reveal(value: PrivateValue) -> PrivateValue {
      return value
    }

    function privateOnly() -> string {
      return "private-only"
    }
    "#,
        )
        .unwrap();
        fs::write(
            dependency.join("public_api.skiff"),
            "function decoy() -> string { return \"public\" }\n",
        )
        .unwrap();

        let project =
            compile_package_project(temp.path()).expect("top-level test service compiles");
        let dependency_artifact = project
            .dependency("example.com/widget", "1.0.0")
            .expect("exact dependency artifact should be retained");
        assert!(matches!(
            dependency_artifact
                .artifact
                .package_local_abi
                .implementation_symbols
                .get("internal.codec.PrivateValue"),
            Some(skiff_artifact_model::PackageLocalAbiSymbol::Type { .. })
        ));
        assert!(matches!(
            dependency_artifact
                .artifact
                .package_local_abi
                .implementation_symbols
                .get("internal.codec.PRIVATE_VALUE"),
            Some(skiff_artifact_model::PackageLocalAbiSymbol::Constant { .. })
        ));
        let requirement = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == "widget")
            .expect("top-level dependency requirement should exist");
        assert_eq!(
            project
                .package
                .artifact
                .package_requirements
                .iter()
                .filter(|requirement| requirement.package_id == "example.com/widget")
                .count(),
            1,
            "the second local alias must not create another package requirement"
        );
        assert_eq!(
            requirement.expected_package_build.as_ref(),
            Some(&dependency_artifact.artifact.package_build_id)
        );
        let file = module_artifact(&project.package, "main");
        assert!(file
            .unit
            .external_refs
            .package_callables
            .iter()
            .any(|callable| {
                callable.package_callable_id.as_str()
                    == "pkg-callable:example.com/widget:top-level:internal.codec.reveal"
            }));
        assert!(file
            .unit
            .external_refs
            .package_callables
            .iter()
            .any(|callable| {
                callable.package_callable_id.as_str()
                    == "pkg-callable:example.com/widget:internal.codec.reveal"
            }));
        assert!(file.unit.executables.iter().any(|executable| {
            executable.body.expressions.iter().any(|expression| {
                matches!(
                    expression,
                    skiff_artifact_model::ExprIr::LoadPackageConst { symbol }
                        if symbol.symbol_path == "internal.codec.PRIVATE_VALUE"
                            && symbol.abi_expectation.as_deref()
                                == Some(
                                    dependency_artifact
                                        .artifact
                                        .package_local_abi
                                        .local_abi_identity
                                        .as_str()
                                )
                )
            })
        }));
        let run = file
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with(".run"))
            .expect("run executable should be lowered");
        assert!(run.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                skiff_artifact_model::ExprIr::Construct {
                    type_ref: skiff_artifact_model::TypeRefIr::PackageSymbol { symbol },
                    ..
                } if matches!(
                    &symbol.package,
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                        if dependency_ref == "widget"
                )
                    && symbol.symbol_path == "internal.codec.PrivateValue"
                    && symbol.abi_expectation.as_deref()
                        == Some(
                            dependency_artifact
                                .artifact
                                .package_local_abi
                                .local_abi_identity
                                .as_str()
                        )
            )
        }));
        let construct = file
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with(".construct"))
            .expect("construct executable should be lowered");
        assert!(matches!(
            &construct.return_type,
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                        if dependency_ref == "widget"
                )
                    && symbol.symbol_path == "internal.codec.PrivateValue"
                    && symbol.abi_expectation.as_deref()
                        == Some(
                            dependency_artifact
                                .artifact
                                .package_local_abi
                                .local_abi_identity
                                .as_str()
                        )
        ));

        fs::write(
            temp.path().join("main.skiff"),
            "import widgetImpl\nfunction run() -> string { return widgetImpl/publicOnly() }\n",
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("top-level alias must not fall back to api.yml")
            .to_string();
        assert!(
            error.contains("has no callable public path `publicOnly`"),
            "{error}"
        );

        fs::write(
            temp.path().join("main.skiff"),
            "import widgetImpl\nfunction bad(value: widgetImpl.internal.codec.PrivateValue) -> string { return value.value }\n",
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("top-level type access must require slash syntax")
            .to_string();
        assert!(
            error.contains("uses top-level type syntax `widgetImpl/<source-module>.<name>`"),
            "{error}"
        );

        fs::write(
            temp.path().join("main.skiff"),
            "import widget\nfunction bad() -> string { return widget/internal.codec.privateOnly() }\n",
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("public mode must not fall back to private source symbols")
            .to_string();
        assert!(
            error.contains("has no callable public path `internal.codec.privateOnly`"),
            "{error}"
        );
    }

    #[test]
    fn test_service_top_level_alias_executes_exact_package_receiver_method() {
        let temp = TestDir::new("skiff-compiler", "top-level-alias-package-receiver");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/box-tests
    version: 1.0.0
    packages:
      - id: example.com/box
        version: 1.0.0
        alias: subject
        topLevelAlias: subjectImpl
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("service.yml"),
            "id: example.com/box-tests\nkind: test\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import subjectImpl

    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.read()
    }
    "#,
        )
        .unwrap();

        let dependency = temp.path().join(".skiff-packages/example~com~~box/1.0.0");
        fs::create_dir_all(&dependency).unwrap();
        fs::write(
            dependency.join("package.yml"),
            "id: example.com/box\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(dependency.join("api.yml"), "{}\n").unwrap();
        fs::write(
            dependency.join("internal.skiff"),
            r#"
    type Box {
      value: string
    }

    function makeBox(value: string) -> Box {
      return Box { value: value }
    }

    impl Box {
      function read(self: Box) -> string {
        return self.value
      }
    }
    "#,
        )
        .unwrap();

        let provider = compile_package_project(&dependency)
            .expect("provider package should compile independently");
        assert!(matches!(
            provider
                .package
                .artifact
                .package_local_abi
                .implementation_symbols
                .get("internal.Box"),
            Some(skiff_artifact_model::PackageLocalAbiSymbol::Type { .. })
        ));
        assert!(matches!(
            provider
                .package
                .artifact
                .package_local_abi
                .implementation_symbols
                .get("internal.makeBox"),
            Some(skiff_artifact_model::PackageLocalAbiSymbol::Callable { .. })
        ));
        let PackageLocalAbiSymbol::Callable {
            callable_id: receiver_callable_id,
            signature,
        } = &provider
            .package
            .artifact
            .package_local_abi
            .implementation_symbols["internal.Box.read"]
        else {
            panic!("provider implementation ABI must contain Box.read");
        };
        assert_eq!(signature.parameters.len(), 1);
        assert_eq!(signature.parameters[0].name, "self");
        assert!(provider
            .package
            .artifact
            .callable_links
            .contains_key(receiver_callable_id));

        let (project, _) = compile_service_package_project(temp.path())
            .expect("top-level receiver call should compile");
        let file = module_artifact(&project.package, "main");
        let run = file
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with(".run"))
            .expect("run executable should be lowered");
        let receiver_call = run
            .body
            .expressions
            .iter()
            .find_map(|expression| {
                let skiff_artifact_model::ExprIr::Call { call } = expression else {
                    return None;
                };
                let skiff_artifact_model::CallTargetIr::PackageCallable {
                    package_ref,
                    package_callable_id,
                } = &call.target
                else {
                    return None;
                };
                (package_callable_id == receiver_callable_id).then_some((package_ref, call))
            })
            .expect("receiver method must lower to the exact package callable");
        assert!(matches!(
            receiver_call.0,
            skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                if dependency_ref == "subject"
        ));
        assert_eq!(
            receiver_call.1.args.len(),
            1,
            "receiver must be the first and only execution argument"
        );
        assert!(file
            .unit
            .external_refs
            .package_callables
            .iter()
            .any(|callable| {
                callable.package_callable_id == *receiver_callable_id
                    && matches!(
                        &callable.package_ref,
                        skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                            if dependency_ref == "subject"
                    )
            }));
    }

    #[test]
    fn package_receiver_calls_fail_closed_outside_the_exact_top_level_surface() {
        let temp = TestDir::new("skiff-compiler", "top-level-package-receiver-negative");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/box-negative-tests
    version: 1.0.0
    packages:
      - id: example.com/box-negative
        version: 1.0.0
        alias: subject
        topLevelAlias: subjectImpl
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("service.yml"),
            "id: example.com/box-negative-tests\nkind: test\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();

        let dependency = temp
            .path()
            .join(".skiff-packages/example~com~~box-negative/1.0.0");
        fs::create_dir_all(&dependency).unwrap();
        fs::write(
            dependency.join("package.yml"),
            "id: example.com/box-negative\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            dependency.join("api.yml"),
            "Box: internal.Box\nmakeBox: internal.makeBox\n",
        )
        .unwrap();
        fs::write(
            dependency.join("internal.skiff"),
            r#"
    type Box {
      value: string
    }

    function makeBox(value: string) -> Box {
      return Box { value: value }
    }

    impl Box {
      function read(self: Box) -> string {
        return self.value
      }

      function append(self: Box, suffix: string) -> string {
        return self.value + suffix
      }
    }
    "#,
        )
        .unwrap();

        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import subjectImpl
    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.append("!")
    }
    "#,
        )
        .unwrap();
        compile_service_package_project(temp.path())
            .expect("a package receiver method with one explicit argument should compile");

        let cases = [
            (
                "ordinary alias",
                r#"
    import subject
    function run() -> string {
      const box = subject/makeBox("ok")
      return box.read()
    }
    "#,
                "receiver method `read`",
            ),
            (
                "missing method",
                r#"
    import subjectImpl
    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.missing()
    }
    "#,
                "internal.Box.missing",
            ),
            (
                "missing explicit argument",
                r#"
    import subjectImpl
    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.append()
    }
    "#,
                "expected 1 arguments, found 0",
            ),
            (
                "explicit receiver argument",
                r#"
    import subjectImpl
    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.read(box)
    }
    "#,
                "expected 0 arguments, found 1",
            ),
            (
                "wrong argument type",
                r#"
    import subjectImpl
    function run() -> string {
      const box = subjectImpl/internal.makeBox("ok")
      return box.append(1)
    }
    "#,
                "argument 1 type mismatch",
            ),
        ];
        for (label, source, expected) in cases {
            fs::write(temp.path().join("main.skiff"), source).unwrap();
            let error = compile_service_package_project(temp.path())
                .expect_err(label)
                .to_string();
            assert!(
                error.contains(expected),
                "{label} should contain `{expected}`, got:\n{error}"
            );
        }

        fs::write(
            temp.path().join("main.skiff"),
            "function run() -> string { return \"unreachable\" }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("service.yml"),
            "id: example.com/box-negative-tests\n",
        )
        .unwrap();
        let error = compile_service_package_project(temp.path())
            .expect_err("non-test services must reject top-level aliases")
            .to_string();
        assert!(
            error.contains("topLevelAlias") && error.contains("kind: test"),
            "unexpected top-level permission diagnostic: {error}"
        );
    }

    #[test]
    fn package_receiver_generic_applied_nominal_keeps_exact_owner_and_type_arguments() {
        let temp = TestDir::new("skiff-compiler", "top-level-generic-package-receiver");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/generic-box-tests
    version: 1.0.0
    packages:
      - id: example.com/generic-box
        version: 1.0.0
        alias: subject
        topLevelAlias: subjectImpl
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("service.yml"),
            "id: example.com/generic-box-tests\nkind: test\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import subjectImpl

    function run() -> string {
      const box = subjectImpl/internal.Box<string> { value: "ok" }
      return box.read()
    }
    "#,
        )
        .unwrap();

        let dependency = temp
            .path()
            .join(".skiff-packages/example~com~~generic-box/1.0.0");
        fs::create_dir_all(&dependency).unwrap();
        fs::write(
            dependency.join("package.yml"),
            "id: example.com/generic-box\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(dependency.join("api.yml"), "{}\n").unwrap();
        fs::write(
            dependency.join("internal.skiff"),
            r#"
    type Box<T> {
      value: T
    }

    impl Box<T> {
      function read(self: Box<T>) -> T {
        return self.value
      }
    }
    "#,
        )
        .unwrap();

        let provider = compile_package_project(&dependency)
            .expect("generic provider should compile independently");
        let PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } = &provider
            .package
            .artifact
            .package_local_abi
            .implementation_symbols["internal.Box.read"]
        else {
            panic!(
                "generic receiver method must use the owner path without source type parameters"
            );
        };
        assert_eq!(signature.type_params, vec!["T"]);
        assert_eq!(signature.parameters[0].name, "self");

        let (project, _) = compile_service_package_project(temp.path())
            .expect("closed generic package receiver should compile");
        let file = module_artifact(&project.package, "main");
        let call = file
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with(".run"))
            .and_then(|run| {
                run.body.expressions.iter().find_map(|expression| {
                    let skiff_artifact_model::ExprIr::Call { call } = expression else {
                        return None;
                    };
                    (matches!(
                        &call.target,
                        skiff_artifact_model::CallTargetIr::PackageCallable {
                            package_callable_id,
                            ..
                        } if package_callable_id == callable_id
                    ))
                    .then_some(call)
                })
            })
            .expect("generic receiver call must lower to its exact package callable");
        assert_eq!(call.args.len(), 1);
        assert_eq!(
            call.type_args.get("T0"),
            Some(&skiff_artifact_model::TypeRefIr::builtin("string"))
        );
    }

    #[test]
    fn test_service_top_level_alias_lowers_the_foreign_db_operation_matrix() {
        let temp = TestDir::new("skiff-compiler", "foreign-db-top-level-alias");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/provider-tests
    version: 1.0.0
    packages:
      - id: example.com/provider
        version: 1.0.0
        alias: provider
        topLevelAlias: providerImpl
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("service.yml"),
            "id: example.com/provider-tests\nkind: test\n",
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import providerImpl

    function matrix(rows: Array<providerImpl/model.Session>) -> bool {
      const inserted = db insert providerImpl/model.Session {
        id = "one"
        value = "first"
        visits = 0
      }
      const found = db find many providerImpl/model.Session {
        where value != null
        order id asc
        limit 10
      }
      const optional = db optional providerImpl/model.Session("one")
      const required = db require providerImpl/model.Session("one")
      const updated = db update providerImpl/model.Session("one") { visits += 1 }
      const replaced = db replace providerImpl/model.Session("one") {
        value = "replacement"
        visits = 2
      }
      const upserted = db upsert providerImpl/model.Session("two") {
        value = "created"
        visits = 0
      } { visits += 1 }
      const count = db count providerImpl/model.Session { where id != null }
      const exists = db exists providerImpl/model.Session("one")
      const query = db query providerImpl/model.Session { where visits >= 0 }
      const insertedMany = db insert many providerImpl/model.Session values rows
      const updatedMany = db update many providerImpl/model.Session {
        where visits >= 0
      } { visits += 1 }
      const deleted = db delete providerImpl/model.Session("two")
      const deletedMany = db delete many providerImpl/model.Session { where visits > 10 }
      const claimed = db claim providerImpl/model.Session("one").worker as locked {
        db update providerImpl/model.Session("one") { visits += 1 }
      }
      const lease = db lease providerImpl/model.Session("one").worker
      db transaction {
        db update providerImpl/model.Session("one") { value = "transaction" }
        db require providerImpl/model.Session("one")
      }
      return claimed
    }
    "#,
        )
        .unwrap();
        let provider = temp
            .path()
            .join(".skiff-packages/example~com~~provider/1.0.0");
        fs::create_dir_all(&provider).unwrap();
        fs::write(
            provider.join("package.yml"),
            "id: example.com/provider\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(provider.join("api.yml"), "Session: model.Session\n").unwrap();
        fs::write(
            provider.join("model.skiff"),
            r#"
    type Session {
      id: string,
      value: string,
      visits: number
    }

    type NotDb {
      id: string
    }

    db object Session {
      name "sessions"
      primary key(id)
      lease worker ttl 1000 max 5000
      index byVisits(visits asc)
    }
    "#,
        )
        .unwrap();

        let (project, _) =
            compile_service_package_project(temp.path()).expect("foreign DB matrix should compile");
        let requirement = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == "provider")
            .expect("one primary provider requirement");
        assert_eq!(
            project
                .package
                .artifact
                .package_requirements
                .iter()
                .filter(|candidate| candidate.package_id == "example.com/provider")
                .count(),
            1
        );
        let provider_artifact = project
            .dependency("example.com/provider", "1.0.0")
            .expect("provider artifact belongs to the exact dependency closure");
        assert_eq!(
            requirement.expected_package_build.as_ref(),
            Some(&provider_artifact.artifact.package_build_id)
        );
        assert!(
            project
                .package
                .file_ir_units
                .iter()
                .all(|file| file.unit.declarations.db.is_empty()),
            "the consumer must not copy the provider DB declaration or schema"
        );
        let file = module_artifact(&project.package, "main");
        let value = file.value();
        let mut targets = Vec::new();
        collect_foreign_db_targets(&value, &mut targets);
        assert!(
            targets.len() >= 18,
            "expected full DB target matrix: {targets:#?}"
        );
        for target in targets {
            assert_eq!(target["typeRef"]["kind"], "packageSymbol");
            assert_eq!(
                target["typeRef"]["symbol"]["package"],
                serde_json::json!({"kind": "dependency", "dependencyRef": "provider"})
            );
            assert_eq!(target["typeRef"]["symbol"]["symbolPath"], "model.Session");
            assert_eq!(
                target["typeRef"]["symbol"]["abiExpectation"],
                requirement.expected_local_abi.as_str()
            );
        }

        // This compiler-owned matrix intentionally stops at File IR. Exact foreign-package DB
        // identity through linking and execution belongs to the runtime host acceptance suite.
        fs::write(
            temp.path().join("main.skiff"),
            "import provider\nfunction bad() -> number { return db count provider/Session {} }\n",
        )
        .unwrap();
        let error = compile_service_package_project(temp.path())
            .expect_err("the public alias must not select a foreign DB attachment")
            .to_string();
        assert!(
            error.contains("not a declared db object") || error.contains("has no DB metadata"),
            "{error}"
        );

        fs::write(
            temp.path().join("main.skiff"),
            "import providerImpl\nfunction bad() -> number { return db count providerImpl/model.NotDb {} }\n",
        )
        .unwrap();
        let error = compile_service_package_project(temp.path())
            .expect_err("a top-level non-DB type must not become a DB target")
            .to_string();
        assert!(
            error.contains("not a declared db object") || error.contains("has no DB metadata"),
            "{error}"
        );
    }

    #[test]
    fn top_level_alias_accepts_canonically_equivalent_public_and_source_type_descriptors() {
        let temp = TestDir::new("skiff-compiler", "top-level-canonical-type-descriptor");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/agent-tests
    version: 1.0.0
    packages:
      - id: example.com/subject
        version: 1.0.0
        alias: subject
        topLevelAlias: subjectImpl
      - id: example.com/agent
        version: 1.0.0
        alias: agent
        topLevelAlias: agentImpl
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import agentImpl
    import subjectImpl

    function pointId(value: agentImpl/canonical.CanonicalMessagePointView) -> string {
      return value.point.id
    }

    function bindings(value: agentImpl/canonical.AgentRuntimeBindings) -> agentImpl/canonical.AgentRuntimeBindings {
      return value
    }

    function subjectId(value: subjectImpl/internal.SubjectState) -> string {
      return value.id
    }
    "#,
        )
        .unwrap();

        let store = temp.path().join(".skiff-packages");
        let agent = store.join("example~com~~agent/1.0.0");
        fs::create_dir_all(&agent).unwrap();
        fs::write(
            agent.join("package.yml"),
            "id: example.com/agent\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            agent.join("api.yml"),
            r#"canonical:
      CanonicalMessagePoint: canonical.CanonicalMessagePoint
      CanonicalMessagePointView: canonical.CanonicalMessagePointView
      ToolProvider: canonical.ToolProvider
      AgentRuntimeBindings: canonical.AgentRuntimeBindings
    "#,
        )
        .unwrap();
        fs::write(
            agent.join("canonical.skiff"),
            r#"
    type CanonicalMessagePoint {
      id: string
    }

    type CanonicalMessagePointView {
      point: CanonicalMessagePoint
    }

    interface ToolProvider {}

    type AgentRuntimeBindings {
      provider: any ToolProvider
    }
    "#,
        )
        .unwrap();

        let subject = store.join("example~com~~subject/1.0.0");
        fs::create_dir_all(&subject).unwrap();
        fs::write(
            subject.join("package.yml"),
            "id: example.com/subject\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(subject.join("api.yml"), "{}\n").unwrap();
        fs::write(
            subject.join("internal.skiff"),
            "type SubjectState { id: string }\n",
        )
        .unwrap();

        let project = compile_package_project(temp.path())
            .expect("public and source descriptors should select one exact agent type identity");
        let agent = project
            .dependency("example.com/agent", "1.0.0")
            .expect("agent artifact");
        let view = &agent.artifact.package_local_abi.implementation_symbols
            ["canonical.CanonicalMessagePointView"];
        let skiff_artifact_model::PackageLocalAbiSymbol::Type { descriptor, .. } = view else {
            panic!("view must remain an implementation type");
        };
        assert!(matches!(
            descriptor,
            skiff_artifact_model::TypeDescriptorIr::Record { fields }
                if matches!(
                    fields.get("point"),
                    Some(skiff_artifact_model::TypeRefIr::PackageSymbol { symbol })
                        if symbol.symbol_path
                            == "canonical.CanonicalMessagePoint"
                )
        ));
    }

    #[test]
    fn top_level_callable_signatures_use_the_primary_alias_for_public_type_identity() {
        let temp = TestDir::new(
            "skiff-compiler",
            "top-level-callable-primary-alias-signature",
        );
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/provider-tests
    version: 1.0.0
    packages:
      - id: example.com/provider
        version: 1.0.0
        alias: provider
        topLevelAlias: providerImpl
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();

        let provider = temp
            .path()
            .join(".skiff-packages/example~com~~provider/1.0.0");
        fs::create_dir_all(&provider).unwrap();
        fs::write(
            provider.join("package.yml"),
            "id: example.com/provider\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            provider.join("api.yml"),
            r#"PublicInput: api.PublicInput
    PublicOutput: api.PublicOutput
    PublicHandler: api.PublicHandler
    PublicEnvelope: api.PublicEnvelope
    "#,
        )
        .unwrap();
        fs::write(
            provider.join("api.skiff"),
            r#"
    type PublicInput {
      text: string
    }

    type PublicOutput {
      text: string
    }

    interface PublicHandler {
      function handle(self: Self, input: PublicInput) -> PublicOutput
    }

    type PublicEnvelope<T> {
      value: T
    }
    "#,
        )
        .unwrap();
        fs::write(
            provider.join("internal.skiff"),
            r#"
    type PrivateInput {
      text: string
    }

    function run(
      input: root.api.PublicInput,
      values: Array<root.api.PublicInput?>,
      handler: any root.api.PublicHandler
    ) -> root.api.PublicOutput {
      return handler.handle(input)
    }

    function echoEnvelope(
      value: root.api.PublicEnvelope<root.api.PublicInput>
    ) -> root.api.PublicEnvelope<root.api.PublicInput> {
      return value
    }

    function privateRun(input: PrivateInput) -> root.api.PublicOutput {
      return { text: input.text }
    }
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import provider
    import providerImpl

    function run(
      input: provider.PublicInput,
      values: Array<provider.PublicInput?>,
      handler: any provider.PublicHandler,
      envelope: provider.PublicEnvelope<provider.PublicInput>
    ) -> provider.PublicOutput {
      const echoed: provider.PublicEnvelope<provider.PublicInput> =
        providerImpl/internal.echoEnvelope(envelope)
      const result: provider.PublicOutput =
        providerImpl/internal.run(echoed.value, values, handler)
      return result
    }
    "#,
        )
        .unwrap();

        compile_package_project(temp.path()).expect(
            "top-level callable signatures should bind public types through the primary alias",
        );

        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import provider
    import providerImpl

    function bad(input: provider.PublicInput) -> provider.PublicOutput {
      return providerImpl/internal.privateRun(input)
    }
    "#,
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("a private implementation type must not impersonate a public type")
            .to_string();
        assert!(
            error.contains("call `providerImpl/internal.privateRun` argument 1")
                && error.contains("type mismatch"),
            "{error}"
        );

        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import provider

    function bad(
      input: provider.PublicInput,
      values: Array<provider.PublicInput?>,
      handler: any provider.PublicHandler
    ) -> provider.PublicOutput {
      return provider/internal.run(input, values, handler)
    }
    "#,
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("the primary alias must not select a private implementation callable")
            .to_string();
        assert!(
            error.contains(
                "package dependency `provider` has no callable public path `internal.run`"
            ),
            "{error}"
        );
    }

    #[test]
    fn package_nominal_object_literals_keep_the_exact_file_ir_construct_target() {
        let temp = TestDir::new("skiff-compiler", "package-nominal-object-lowering");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/tool-consumer
    version: 1.0.0
    packages:
      - id: example.com/tool-types
        version: 0.1.0
        alias: tools
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import tools

    function automatic() -> tools.ToolChoice {
      return { tag: "auto" }
    }

    function named(name: string) -> tools.ToolChoice {
      return { tag: "tool", name: name, options: { note: null } }
    }

    function throughCall() -> tools.ToolChoice {
      return tools/accept({ tag: "auto" })
    }
    "#,
        )
        .unwrap();

        let dependency = temp
            .path()
            .join(".skiff-packages/example~com~~tool-types/0.1.0");
        fs::create_dir_all(&dependency).unwrap();
        fs::write(
            dependency.join("package.yml"),
            "id: example.com/tool-types\nversion: 0.1.0\n",
        )
        .unwrap();
        fs::write(
            dependency.join("api.yml"),
            "ToolChoice: types.ToolChoice\nToolOptions: types.ToolOptions\naccept: types.accept\n",
        )
        .unwrap();
        fs::write(
            dependency.join("types.skiff"),
            r#"
    type ToolOptions { note: string? }
    type ToolChoice discriminator "tag" =
      { tag: "auto" }
      | { tag: "tool", name: string, options: ToolOptions? }

    function accept(choice: ToolChoice) -> ToolChoice {
      return choice
    }
    "#,
        )
        .unwrap();

        let project = compile_package_project(temp.path())
            .expect("Package nominal constructors should lower");
        let dependency_artifact = project
            .dependency("example.com/tool-types", "0.1.0")
            .expect("exact tool type dependency artifact should be retained");
        let file = module_artifact(&project.package, "main");
        let targets = file
            .unit
            .executables
            .iter()
            .flat_map(|executable| executable.body.expressions.iter())
            .filter_map(|expression| match expression {
                skiff_artifact_model::ExprIr::Construct { type_ref, .. } => Some(type_ref),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            targets
                .iter()
                .filter(|target| matches!(
                    target,
                    skiff_artifact_model::TypeRefIr::PackageSymbol {
                        symbol: skiff_artifact_model::PackageSymbolRef {
                            package: skiff_artifact_model::PackageRefIr::Dependency { dependency_ref },
                            symbol_path,
                            ..
                        }
                    } if dependency_ref == "tools" && symbol_path == "ToolChoice"
                ))
                .count()
                >= 3,
            "return and public-call literals must retain the exact Package nominal target: {}",
            file.value()
        );
        let through_call = file
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol.ends_with(".throughCall"))
            .expect("public package call executable should be lowered");
        assert!(through_call
            .body
            .expressions
            .iter()
            .any(|expression| matches!(
                expression,
                skiff_artifact_model::ExprIr::Construct {
                    type_ref: skiff_artifact_model::TypeRefIr::PackageSymbol {
                        symbol: skiff_artifact_model::PackageSymbolRef {
                            package:
                                skiff_artifact_model::PackageRefIr::Dependency { dependency_ref },
                            symbol_path,
                            abi_expectation,
                        },
                    },
                    ..
                } if dependency_ref == "tools"
                    && symbol_path == "ToolChoice"
                    && abi_expectation.as_deref()
                        == Some(
                            dependency_artifact
                                .artifact
                                .package_local_abi
                                .local_abi_identity
                                .as_str()
                        )
            )));

        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import tools

    function invalid() -> tools.ToolChoice {
      return tools/accept({ tag: "tool", name: 42 })
    }
    "#,
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("public package callable arguments must retain their exact target shape")
            .to_string();
        assert!(
            error.contains("call `tools/accept` argument 1 object literal field `name`")
                && error.contains("expected string, found integer"),
            "unexpected public package parameter error: {error}"
        );
    }

    #[test]
    fn dependency_callable_local_parameter_preserves_schema_result_field_types() {
        let temp = TestDir::new("skiff-compiler", "dependency-local-parameter-schema-result");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/result-consumer
    version: 1.0.0
    packages:
      - id: example.com/result-provider
        version: 1.0.0
        alias: provider
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        let provider = temp
            .path()
            .join(".skiff-packages/example~com~~result-provider/1.0.0");
        fs::create_dir_all(&provider).unwrap();
        fs::write(
            provider.join("package.yml"),
            "id: example.com/result-provider\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            provider.join("api.yml"),
            r#"Handler: tools.Handler
    Bindings: tools.Bindings
    Child: api.Child
    Result: api.Result
    Label: api.Label
    run: api.run
    "#,
        )
        .unwrap();
        fs::write(
            provider.join("tools.skiff"),
            r#"
    interface Handler {
      function handle(self: Self, input: string) -> string
    }

    type Bindings {
      handler: any Handler
    }
    "#,
        )
        .unwrap();
        fs::write(
            provider.join("api.skiff"),
            r#"

    type Child {
      id: string
    }

    alias Label = string

    type Result {
      ok: bool,
      note: string?,
      child: Child,
      label: Label,
    }

    type PrivateBindings {
      marker: string
    }

    function run(bindings: root.tools.Bindings) -> Result {
      return {
        ok: true,
        note: null,
        child: { id: "child" },
        label: "ready",
      }
    }
    "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import provider

    function throughLocal(bindings: provider.Bindings) -> JsonObject {
      const result = provider/run(bindings)
      return {
        ok: result.ok,
        note: result.note,
        childId: result.child.id,
        label: result.label,
      }
    }

    function direct(bindings: provider.Bindings) -> bool {
      return provider/run(bindings).ok
    }
    "#,
        )
        .unwrap();

        let project = compile_package_project(temp.path()).expect(
            "artifact-only owner-local parameter should preserve schema result field types",
        );
        let provider = project
            .dependency("example.com/result-provider", "1.0.0")
            .expect("fresh provider artifact");
        assert!(
            !provider.package_schema_index.types.contains_key("Label"),
            "transparent alias must not acquire PackageSchema identity"
        );
        assert!(
            !provider.package_schema_index.types.contains_key("Bindings"),
            "owner-local any-interface bindings must remain outside boundary schema"
        );
        for public_path in ["Child", "Result"] {
            assert!(
                provider
                    .package_schema_index
                    .types
                    .contains_key(public_path),
                "schema-closed public record {public_path} should retain schema identity"
            );
        }
        let run = provider
            .artifact
            .package_local_abi
            .public_symbols
            .get("run")
            .expect("public run callable");
        let skiff_artifact_model::PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } = run
        else {
            panic!("run must remain a package callable")
        };
        assert_eq!(
            callable_id.as_str(),
            "pkg-callable:example.com/result-provider:run"
        );
        assert!(matches!(
            &signature.parameters[0].ty,
            skiff_artifact_model::PackageTypeRef::Local {
                local_type:
                    skiff_artifact_model::TypeRefIr::ServiceSymbol {
                        symbol:
                            skiff_artifact_model::ServiceSymbolRef {
                                module_path,
                                symbol,
                            },
                    },
            } if module_path == "tools" && symbol == "Bindings"
        ));
        assert!(matches!(
            &signature.return_type,
            skiff_artifact_model::PackageTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                ..
            } if package_id == "example.com/result-provider" && stable_schema_key == "Result"
        ));

        let consumer = module_artifact(&project.package, "main");
        assert!(
            consumer
                .unit
                .external_refs
                .package_callables
                .iter()
                .any(|reference| {
                    matches!(
                        &reference.package_ref,
                        skiff_artifact_model::PackageRefIr::Dependency { dependency_ref }
                            if dependency_ref == "provider"
                    ) && reference.package_callable_id == *callable_id
                }),
            "lowered call must retain the manifest alias and exact callable id"
        );
        let requirement = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == "provider")
            .expect("provider requirement");
        assert_eq!(
            requirement.expected_local_abi, provider.artifact.package_local_abi.local_abi_identity,
            "lowered dependency requirement must retain the expected Local ABI"
        );

        let compile_error = |source: &str| {
            fs::write(temp.path().join("main.skiff"), source).unwrap();
            compile_package_project(temp.path())
                .expect_err("negative dependency-call fixture should fail")
                .to_string()
        };
        let error = compile_error(
            r#"
    import provider
    function bad() -> bool {
      return provider/run().ok
    }
    "#,
        );
        assert!(
            error.contains("call `provider/run` arity mismatch"),
            "{error}"
        );
        assert!(
            !error.contains("unknown field `ok`")
                && !error.contains("has no resolved expression type"),
            "arity failure must not discard an independently resolved return: {error}"
        );

        let error = compile_error(
            r#"
    import provider
    function bad() -> bool {
      return provider/run("wrong").ok
    }
    "#,
        );
        assert!(
            error.contains("call `provider/run` argument 1")
                && error.contains("type mismatch")
                && error.contains("expected Bindings"),
            "{error}"
        );
        assert!(
            !error.contains("unknown field `ok`")
                && !error.contains("has no resolved expression type"),
            "scalar mismatch must not discard an independently resolved return: {error}"
        );

        let error = compile_error(
            r#"
    import provider
    type OtherBindings { marker: string }
    function bad(other: OtherBindings) -> bool {
      return provider/run(other).ok
    }
    "#,
        );
        assert!(
            error.contains("call `provider/run` argument 1") && error.contains("type mismatch"),
            "{error}"
        );
        assert!(
            !error.contains("unknown field `ok`")
                && !error.contains("has no resolved expression type"),
            "nominal mismatch must not discard an independently resolved return: {error}"
        );

        let error = compile_error(
            r#"
    import provider
    function bad(bindings: provider.Bindings) -> string {
      return provider/run(bindings).missing
    }
    "#,
        );
        assert!(error.contains("unknown field `missing`"), "{error}");

        let error = compile_error(
            r#"
    import provider
    function bad(value: provider.PrivateBindings) -> string {
      return value.marker
    }
    "#,
        );
        assert!(
            error.contains("unknown field `marker` on provider.PrivateBindings"),
            "private/nonexported owner-local record shape must not resolve through the dependency alias: {error}"
        );
    }

    #[test]
    fn transitive_aliases_are_owned_by_each_package_artifact() {
        let temp = TestDir::new("skiff-compiler", "transitive-package-alias");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/transitive-app
    version: 1.0.0
    packages:
      - id: example.com/facade
        version: 0.1.0
        alias: app
        topLevelAlias: appImpl
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            "import app\nfunction run() -> string { return app/facade() }\n",
        )
        .unwrap();
        write_cloud_dependency(temp.path());

        let facade = temp
            .path()
            .join(".skiff-packages/example~com~~facade/0.1.0");
        fs::create_dir_all(&facade).unwrap();
        fs::write(
            facade.join("package.yml"),
            r#"id: example.com/facade
    version: 0.1.0
    packages:
      - id: google.com/cloud
        version: 0.1.0
        alias: platform
    "#,
        )
        .unwrap();
        fs::write(facade.join("api.yml"), "facade: facade_impl.facade\n").unwrap();
        fs::write(
            facade.join("facade_impl.skiff"),
            "import platform\nfunction facade() -> string { return platform/storage.upload() }\n",
        )
        .unwrap();

        let project =
            compile_package_project(temp.path()).expect("transitive graph should compile");
        let facade = project
            .dependency("example.com/facade", "0.1.0")
            .expect("facade artifact should be present");
        assert!(project.dependency("google.com/cloud", "0.1.0").is_some());
        assert_eq!(
            project.package.artifact.package_requirements[0].alias,
            "app"
        );
        assert_eq!(facade.artifact.package_requirements[0].alias, "platform");
        assert_file_ir_contains_package_callable(
            facade,
            "facade_impl",
            "platform",
            "google.com/cloud",
            "storage.upload",
        );
        assert_file_ir_contains_package_callable(
            &project.package,
            "main",
            "app",
            "example.com/facade",
            "facade",
        );

        fs::write(
            temp.path().join("main.skiff"),
            "import platform\nfunction bad() -> string { return platform/storage.upload() }\n",
        )
        .unwrap();
        let error = compile_package_project(temp.path())
            .expect_err("a direct topLevelAlias must not expose its provider's dependencies")
            .to_string();
        assert!(
            error.contains("import platform requires top-level packages to include platform"),
            "{error}"
        );
    }

    #[test]
    fn public_aliases_expand_across_fresh_package_artifacts_without_nominal_schema() {
        let temp = TestDir::new("skiff-compiler", "public-alias-expansion");
        fs::write(
            temp.path().join("package.yml"),
            r#"id: example.com/alias-consumer
    version: 1.0.0
    packages:
      - id: example.com/alias-facade
        version: 1.0.0
        alias: facade
      - id: example.com/alias-base
        version: 1.0.0
        alias: base
    "#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            temp.path().join("main.skiff"),
            r#"
    import facade

    function scalar(value: facade.Scalar) -> string {
      return value
    }

    function status(value: facade.Status) -> string {
      return value
    }

    function users(value: facade.Users) -> facade.Users {
      return value
    }

    function remote(value: facade.RemoteUser) -> facade.RemoteUser {
      return value
    }
    "#,
        )
        .unwrap();

        let base = temp
            .path()
            .join(".skiff-packages/example~com~~alias-base/1.0.0");
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("package.yml"),
            "id: example.com/alias-base\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(base.join("api.yml"), "User: types.User\n").unwrap();
        fs::write(base.join("types.skiff"), "type User { id: string }\n").unwrap();

        let facade = temp
            .path()
            .join(".skiff-packages/example~com~~alias-facade/1.0.0");
        fs::create_dir_all(&facade).unwrap();
        fs::write(
            facade.join("package.yml"),
            r#"id: example.com/alias-facade
    version: 1.0.0
    packages:
      - id: example.com/alias-base
        version: 1.0.0
        alias: base
    "#,
        )
        .unwrap();
        fs::write(
            facade.join("api.yml"),
            r#"Scalar: aliases.Scalar
    Status: aliases.Status
    Users: aliases.Users
    RemoteUser: aliases.RemoteUser
    Envelope: aliases.Envelope
    echoScalar: aliases.echoScalar
    echoStatus: aliases.echoStatus
    echoUsers: aliases.echoUsers
    echoRemote: aliases.echoRemote
    "#,
        )
        .unwrap();
        fs::write(
            facade.join("aliases.skiff"),
            r#"
    import base

    alias Scalar = string
    alias Status = "running" | "completed" | "failed"
    alias Users = Array<base.User?>
    alias RemoteUser = base.User

    type Envelope {
      user: RemoteUser,
      status: Status,
      users: Users,
    }

    function echoScalar(value: Scalar) -> Scalar { return value }
    function echoStatus(value: Status) -> Status { return value }
    function echoUsers(value: Users) -> Users { return value }
    function echoRemote(value: RemoteUser) -> RemoteUser { return value }
    "#,
        )
        .unwrap();

        let project = compile_package_project(temp.path())
            .expect("fresh base, facade, and consumer artifacts should compile");
        let base = project
            .dependency("example.com/alias-base", "1.0.0")
            .expect("base artifact");
        assert!(
            base.package_schema_index.types.contains_key("User"),
            "the genuine nominal record must retain schema identity"
        );

        let facade = project
            .dependency("example.com/alias-facade", "1.0.0")
            .expect("facade artifact");
        for public_path in ["Scalar", "Status", "Users", "RemoteUser"] {
            assert!(
                !facade.package_schema_index.types.contains_key(public_path),
                "transparent alias {public_path} must not receive a PackageSchema identity"
            );
            assert!(matches!(
                facade
                    .artifact
                    .package_local_abi
                    .public_symbols
                    .get(public_path),
                Some(skiff_artifact_model::PackageLocalAbiSymbol::Type { is_alias: true, .. })
            ));
        }

        let status = facade
            .artifact
            .package_local_abi
            .public_symbols
            .get("Status")
            .expect("Status ABI metadata");
        let skiff_artifact_model::PackageLocalAbiSymbol::Type {
            descriptor:
                skiff_artifact_model::TypeDescriptorIr::Alias {
                    target: skiff_artifact_model::TypeRefIr::Union { items },
                },
            ..
        } = status
        else {
            panic!("Status alias must retain its exact literal-union RHS metadata")
        };
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|item| matches!(
            item,
            skiff_artifact_model::TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::String { .. }
            }
        )));

        let remote = facade
            .artifact
            .package_local_abi
            .public_symbols
            .get("RemoteUser")
            .expect("RemoteUser ABI metadata");
        assert!(
            matches!(
                remote,
                skiff_artifact_model::PackageLocalAbiSymbol::Type {
                    descriptor:
                        skiff_artifact_model::TypeDescriptorIr::Alias {
                            target:
                                skiff_artifact_model::TypeRefIr::PackageSymbol {
                                    symbol:
                                        skiff_artifact_model::PackageSymbolRef {
                                            package:
                                                skiff_artifact_model::PackageRefIr::Dependency {
                                                    dependency_ref
                                                },
                                            symbol_path,
                                            ..
                                        }
                                }
                        },
                    is_alias: true,
                    ..
                } if dependency_ref == "base" && symbol_path == "User"
            ),
            "unexpected RemoteUser ABI metadata: {remote:#?}"
        );

        let consumer = module_artifact(&project.package, "main");
        let status_function = consumer
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol == "main.status")
            .expect("consumer status function");
        assert!(matches!(
            &status_function.params[0].ty,
            skiff_artifact_model::TypeRefIr::Union { items }
                if items.len() == 3
                    && items.iter().all(|item| matches!(
                        item,
                        skiff_artifact_model::TypeRefIr::Literal {
                            value: skiff_artifact_model::LiteralIr::String { .. }
                        }
                    ))
        ));
        assert_eq!(
            status_function.return_type,
            skiff_artifact_model::TypeRefIr::builtin("string")
        );

        let scalar_function = consumer
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol == "main.scalar")
            .expect("consumer scalar function");
        assert_eq!(
            scalar_function.params[0].ty,
            skiff_artifact_model::TypeRefIr::builtin("string")
        );

        let users_function = consumer
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol == "main.users")
            .expect("consumer users function");
        for ty in [&users_function.params[0].ty, &users_function.return_type] {
            assert!(matches!(
                ty,
                skiff_artifact_model::TypeRefIr::Builtin { name, args }
                    if name == "Array"
                        && matches!(
                            args.as_slice(),
                            [skiff_artifact_model::TypeRefIr::Nullable { inner }]
                                if matches!(
                                    inner.as_ref(),
                                    skiff_artifact_model::TypeRefIr::PackageSymbol {
                                        symbol:
                                            skiff_artifact_model::PackageSymbolRef {
                                                package:
                                                    skiff_artifact_model::PackageRefIr::PackageId {
                                                        package_id
                                                    },
                                                symbol_path,
                                                ..
                                            }
                                    } if package_id == "example.com/alias-base"
                                        && symbol_path == "User"
                                )
                        )
            ));
        }

        let remote_function = consumer
            .unit
            .executables
            .iter()
            .find(|executable| executable.symbol == "main.remote")
            .expect("consumer remote function");
        for ty in [&remote_function.params[0].ty, &remote_function.return_type] {
            assert!(matches!(
                ty,
                skiff_artifact_model::TypeRefIr::PackageSymbol {
                    symbol:
                        skiff_artifact_model::PackageSymbolRef {
                            package:
                                skiff_artifact_model::PackageRefIr::PackageId {
                                    package_id
                                },
                            symbol_path,
                            ..
                        }
                } if package_id == "example.com/alias-base" && symbol_path == "User"
            ));
        }
    }

    #[test]
    fn dependency_alias_participates_in_package_build_identity() {
        let mut identities = Vec::new();
        for alias in ["left", "right"] {
            let temp = TestDir::new("skiff-compiler", alias);
            fs::write(
                temp.path().join("package.yml"),
                format!(
                    "id: example.com/identity-app\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n    alias: {alias}\n"
                ),
            )
            .unwrap();
            fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
            fs::write(
                temp.path().join("main.skiff"),
                format!(
                    "import {alias}\nfunction run() -> string {{ return {alias}/storage.upload() }}\n"
                ),
            )
            .unwrap();
            write_cloud_dependency(temp.path());
            identities.push(
                compile_package_project(temp.path())
                    .expect("identity fixture should compile")
                    .package
                    .artifact
                    .package_build_id,
            );
        }
        assert_ne!(identities[0], identities[1]);
    }

    #[test]
    fn invalid_dependency_aliases_and_unknown_roots_fail_closed() {
        for (name, manifest, expected) in [
            (
                "complex-without-alias",
                "id: example.com/invalid\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n",
                "google.com/cloud requires alias",
            ),
            (
                "duplicate-alias",
                "id: example.com/invalid\nversion: 1.0.0\npackages:\n  - id: google.com/cloud\n    version: 0.1.0\n    alias: cloud\n  - id: example.org/cloud\n    version: 0.1.0\n    alias: cloud\n",
                "packages alias cloud",
            ),
        ] {
            let temp = TestDir::new("skiff-compiler", name);
            fs::write(temp.path().join("package.yml"), manifest).unwrap();
            fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
            let error = read_user_package_manifest(&temp.path().join("package.yml"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }

        let unknown = TestDir::new("skiff-compiler", "unknown-root-call");
        fs::write(
            unknown.path().join("package.yml"),
            "id: example.com/unknown-root-call\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(unknown.path().join("api.yml"), "{}\n").unwrap();
        fs::write(
            unknown.path().join("main.skiff"),
            "function run() -> string { return unknown.root.call() }\n",
        )
        .unwrap();
        let error = compile_package_project(unknown.path())
            .expect_err("unknown root should fail")
            .to_string();
        assert!(error.contains("unresolved root unknown"));
        assert!(error.contains("unknown.root.call"));
    }
}
