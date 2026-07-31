mod common;

use std::collections::BTreeMap;

use common::{
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_service_package_project,
        compile_service_package_project_with_contract_dependencies,
    },
    TestDir,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef,
};
use skiff_compiler::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};

const ROOT_PACKAGE_ID: &str = "example.com/inline-effect-tests";

fn write_package_effect_fixture(fixture: &TestDir, test_source: &str) {
    fixture.write(
        "package.yml",
        format!(
            r#"id: {ROOT_PACKAGE_ID}
version: 1.0.0
packages:
  - id: example.com/helper
    version: 1.0.0
    alias: helper
"#
        ),
    );
    fixture.write("api.yml", "{}\n");
    fixture.write(
        "service.yml",
        format!("id: {ROOT_PACKAGE_ID}\nkind: test\n"),
    );
    fixture.write("main.test.skiff", test_source);

    let dependency = ".skiff-packages/example~com~~helper/1.0.0";
    fixture.write(
        format!("{dependency}/package.yml"),
        "id: example.com/helper\nversion: 1.0.0\n",
    );
    fixture.write(
        format!("{dependency}/api.yml"),
        r#"EffectRequest: main.EffectRequest
EffectResponse: main.EffectResponse
tools:
  lookup: main.lookup
  events: main.events
"#,
    );
    fixture.write(
        format!("{dependency}/main.skiff"),
        r#"
type EffectRequest {
  method: string,
  url: string,
  detail: string,
}

type EffectResponse { value: string }

function lookup(input: EffectRequest) -> EffectResponse {
  return EffectResponse { value: input.url }
}

function events(input: EffectRequest) -> Stream<EffectResponse> {
  emit(EffectResponse { value: input.url })
}
"#,
    );
}

fn echo_contract() -> skiff_compiler::ServiceContract {
    let plan = |owner| BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    };
    compile_service_contract(ServiceContractDefinition {
        service_id: "example.com/payments".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "input".to_string(),
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: plan(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: plan(BoundaryValueOwner::Provider),
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
        package_type_requirements: Vec::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Payments".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("echo contract compiles")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_test_source_is_ignored_by_ordinary_compile_and_rejected_by_test_service_compile() {
        let fixture = TestDir::new("skiff-compiler", "malformed-test-source-selection");
        fixture.write(
            "package.yml",
            format!("id: {ROOT_PACKAGE_ID}\nversion: 1.0.0\n"),
        );
        fixture.write("api.yml", "{}\n");
        fixture.write(
            "service.yml",
            format!("id: {ROOT_PACKAGE_ID}\nkind: test\n"),
        );
        fixture.write(
            "main.skiff",
            "function production() -> string { return \"ok\" }\n",
        );
        fixture.write("main.test.skiff", "test \"broken\" { assert true\n");

        compile_package_project(fixture.path())
            .expect("ordinary package compilation must not read or parse test-only source");

        let error = compile_service_package_project(fixture.path())
            .expect_err("kind:test compilation must parse its explicit test source surface")
            .to_string();
        assert!(error.contains("main.test.skiff"), "{error}");
    }

    #[test]
    fn test_service_rejects_inline_effect_request_and_outcome_type_mismatches() {
        let cases = [
            (
                "common-expect",
                r#"
test "invalid common expect" effects {
  helper/tools.lookup {
    expect: { method: 7 },
    respond: helper.EffectResponse { value: "ok" },
  }
} { assert true }
"#,
                "test effect expect subset",
            ),
            (
                "step-expect",
                r#"
test "invalid step expect" effects {
  helper/tools.lookup {
    sequence: [{
      expect: { url: 7 },
      respond: helper.EffectResponse { value: "ok" },
    }],
  }
} { assert true }
"#,
                "test effect expect subset",
            ),
            (
                "respond",
                r#"
test "invalid response" effects {
  helper/tools.lookup {
    respond: { value: 7 },
  }
} { assert true }
"#,
                "test effect respond",
            ),
            (
                "stream",
                r#"
test "invalid stream event" effects {
  helper/tools.events {
    stream: [{ value: 7 }],
  }
} { assert true }
"#,
                "test effect stream event",
            ),
            (
                "throw",
                r#"
test "invalid throw" effects {
  helper/tools.lookup {
    throw: "not-a-nominal-error",
  }
} { assert true }
"#,
                "throw has invalid catch payload",
            ),
        ];

        for (label, source, expected) in cases {
            let fixture = TestDir::new("skiff-compiler", &format!("test-effect-{label}"));
            write_package_effect_fixture(&fixture, source);

            let error = compile_service_package_project(fixture.path())
                .expect_err("invalid inline effect type must fail test-service compilation")
                .to_string();
            assert!(
                error.contains(expected),
                "expected {expected:?} for {label}, got {error}"
            );
        }
    }

    #[test]
    fn test_service_rejects_two_service_aliases_for_one_exact_effect_target() {
        let fixture = TestDir::new("skiff-compiler", "duplicate-service-effect-alias");
        fixture.write(
            "package.yml",
            format!(
                r#"id: {ROOT_PACKAGE_ID}
version: 1.0.0
services:
  - id: example.com/payments
    version: 1.0.0
    alias: payments
  - id: example.com/payments
    version: 1.0.0
    alias: paymentsTwin
"#
            ),
        );
        fixture.write("api.yml", "{}\n");
        fixture.write(
            "service.yml",
            format!("id: {ROOT_PACKAGE_ID}\nkind: test\n"),
        );
        fixture.write(
            "main.test.skiff",
            r#"
test "duplicate exact service target" effects {
  payments/echo { respond: "first" },
  paymentsTwin/echo { respond: "second" },
} { assert true }
"#,
        );

        let contract = echo_contract();
        let dependencies = BTreeMap::from([(
            (ROOT_PACKAGE_ID.to_string(), "1.0.0".to_string()),
            vec![
                package_contract_dependency("payments", contract.clone()),
                package_contract_dependency("paymentsTwin", contract),
            ],
        )]);
        let error = compile_service_package_project_with_contract_dependencies(
            fixture.path(),
            &dependencies,
        )
        .expect_err("duplicate aliases for one exact effect target must fail")
        .to_string();

        assert!(error.contains("payments/echo"), "{error}");
        assert!(error.contains("paymentsTwin/echo"), "{error}");
        assert!(error.contains("use one explicit sequence"), "{error}");
    }
}
