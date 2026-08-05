mod common;
use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};
use skiff_artifact_model::{
    ExprIr, InstructionSourceSite, StmtIr, FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION,
    FILE_IR_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

fn executable<'a>(
    file: &'a skiff_artifact_model::FileIrUnit,
    name: &str,
) -> &'a skiff_artifact_model::ExecutableIr {
    file.executables
        .iter()
        .find(|executable| executable.symbol.ends_with(&format!(".{name}")))
        .unwrap_or_else(|| panic!("missing executable {name}"))
}

fn expression_kind<T>(
    executable: &skiff_artifact_model::ExecutableIr,
    mut select: impl FnMut(&ExprIr) -> Option<T>,
) -> T {
    executable
        .body
        .expressions
        .iter()
        .find_map(&mut select)
        .expect("missing expected expression")
}

fn assert_source_site(site: &InstructionSourceSite) {
    let InstructionSourceSite::Source { span } = site else {
        panic!("execution wrapper must retain an authored source site")
    };
    assert!(span.end.offset.unwrap() > span.start.offset.unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_execution_plans_lower_to_exact_timeout_ir() {
        let fixture = TestDir::new("skiff-compiler", "timeout-artifact-lowering");
        fixture.write(
            "package.yml",
            "id: example.com/timeout-artifact\nversion: 1.0.0\n",
        );
        fixture.write("api.yml", "{}\n");
        fixture.write(
            "main.skiff",
            r#"
function statementTimeout() -> number {
  timeout(20ms) {
    const ignored = 1
  }
  return 2
}

function sequentialValue() -> string {
  return timeout(30ms) value {
    const value = "ok"
    value
  }
}

"#,
        );

        let project =
            compile_package_project(fixture.path()).expect("timeout fixture should compile");
        let file = &module_artifact(&project.package, "main").unit;

        assert_eq!(file.schema_version, FILE_IR_SCHEMA_VERSION);
        assert_eq!(file.ir_format_version, FILE_IR_FORMAT_VERSION);
        assert_eq!(file.opcode_table_version, FILE_IR_OPCODE_TABLE_VERSION);
        assert_eq!(FILE_IR_SCHEMA_VERSION, "skiff-file-ir-v12");
        assert_eq!(FILE_IR_FORMAT_VERSION, "skiff-file-ir-format-v7");
        assert_eq!(FILE_IR_OPCODE_TABLE_VERSION, "skiff-opcode-table-v2");
        // TODO(L1): golden File IR identity must be recomputed after the
        // skiff-file-ir-v11 -> v12 schema bump (hash input includes schema_version).
        assert_eq!(
            file.file_ir_identity,
            "skiff-file-ir-v12:sha256:<recompute-after-schema-bump>"
        );
        assert_eq!(
            skiff_artifact_identity::file_ir_identity(file).unwrap(),
            file.file_ir_identity
        );

        let statement = executable(file, "statementTimeout");
        let timeout = statement
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                StmtIr::Timeout {
                    duration_ms,
                    body,
                    site,
                } => Some((*duration_ms, body, site)),
                _ => None,
            })
            .expect("statement timeout wrapper");
        assert_eq!(timeout.0, 20);
        assert!(timeout.1.starts_with("timeout_body$"));
        assert_source_site(timeout.2);

        let sequential = executable(file, "sequentialValue");
        let timed_value = expression_kind(sequential, |expression| match expression {
            ExprIr::Timeout {
                duration_ms,
                value,
                site,
            } => Some((*duration_ms, value.expression, site.clone())),
            _ => None,
        });
        assert_eq!(timed_value.0, 30);
        assert_source_site(&timed_value.2);
        let ExprIr::ValueBlock { block, result } =
            &sequential.body.expressions[timed_value.1 as usize]
        else {
            panic!("timeout value must preserve its user-authored sequential value block")
        };
        let value_block = sequential
            .body
            .blocks
            .iter()
            .find(|candidate| candidate.label == *block)
            .expect("sequential value block body");
        let [body_statement] = value_block.statements.as_slice() else {
            panic!("sequential value body must retain its single binding")
        };
        let StmtIr::Let {
            slot: value_slot,
            value: initializer,
        } = &sequential.body.statements[body_statement.statement as usize]
        else {
            panic!("sequential value body must lower its binding")
        };
        assert!(matches!(
            sequential.body.expressions[initializer.expression as usize],
            ExprIr::Literal { .. }
        ));
        assert!(matches!(
            sequential.body.expressions[result.expression as usize],
            ExprIr::LoadSlot { slot } if slot == *value_slot
        ));
        assert_eq!(
            sequential.return_type,
            skiff_artifact_model::TypeRefIr::builtin("string")
        );
    }

    #[test]
    fn timeout_file_ir_upgrade_does_not_change_unrelated_top_level_schemas() {
        assert_eq!(
            PACKAGE_ARTIFACT_SCHEMA_VERSION,
            "skiff-package-artifact-v11"
        );
        assert_eq!(SERVICE_CONTRACT_SCHEMA_VERSION, "skiff-service-contract-v5");
        assert_eq!(RUNTIME_ASSEMBLY_SCHEMA_VERSION, "skiff-runtime-assembly-v3");
    }

    #[test]
    fn concurrent_and_serial_source_is_rejected_in_package_compile() {
        for (fixture_name, body) in [
            (
                "concurrent-statement",
                "function run() -> void {\n  concurrent { const value = 1 }\n}\n",
            ),
            (
                "concurrent-value",
                "function run() -> number {\n  return concurrent value { 1 }\n}\n",
            ),
            (
                "serial",
                "function run() -> void {\n  serial { const value = 1 }\n}\n",
            ),
        ] {
            let fixture = TestDir::new("skiff-compiler", fixture_name);
            fixture.write(
                "package.yml",
                "id: example.com/timeout-artifact\nversion: 1.0.0\n",
            );
            fixture.write("api.yml", "{}\n");
            fixture.write("main.skiff", body);
            let error = compile_package_project(fixture.path())
                .expect_err("concurrent/serial must be rejected in v1");
            assert!(
                error.to_string().contains("not supported in v1"),
                "unexpected diagnostic: {error}"
            );
        }
    }

    #[test]
    fn standalone_lowering_without_package_execution_plan_fails_closed() {
        let error = skiff_compiler_lowering::source_file_lowering::compile_source_file_ir_unit(
            "function run() -> number {\n  timeout(20ms) {\n  }\n  return 1\n}\n",
            "timeout-standalone.skiff",
            "timeout.standalone",
            "package",
        )
        .expect_err("execution syntax requires the package source plan");
        assert!(
            error
                .to_string()
                .contains("PackageSourceModel::execution_semantics()"),
            "unexpected diagnostic: {error}"
        );
    }

    #[test]
    fn timeout_body_changes_build_identity_but_not_existing_public_callable_abi() {
        let plain = TestDir::new("skiff-compiler", "timeout-public-abi-plain");
        let timed = TestDir::new("skiff-compiler", "timeout-public-abi-timed");
        for fixture in [&plain, &timed] {
            fixture.write(
                "package.yml",
                "id: example.com/timeout-public-abi\nversion: 1.0.0\n",
            );
            fixture.write("api.yml", "run: main.run\n");
        }
        plain.write(
            "main.skiff",
            "function run(value: number) -> number {\n  return value\n}\n",
        );
        timed.write(
        "main.skiff",
        "function run(value: number) -> number {\n  timeout(20ms) {\n    const ignored = value\n  }\n  return value\n}\n",
    );

        let plain =
            compile_package_project(plain.path()).expect("plain public package should compile");
        let timed =
            compile_package_project(timed.path()).expect("timed public package should compile");

        assert_eq!(
            plain.package.artifact.package_local_abi.public_symbols,
            timed.package.artifact.package_local_abi.public_symbols
        );
        assert_eq!(
            plain.package.artifact.package_local_abi.local_abi_identity,
            timed.package.artifact.package_local_abi.local_abi_identity
        );
        assert_ne!(
            plain.package.artifact.package_build_id,
            timed.package.artifact.package_build_id
        );
    }
}
