mod common;

use std::collections::BTreeMap;

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project_with_contract_dependencies, PublishedPackageProject,
    },
    TestDir,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallIr,
    CallTargetIr, ContractTypeRef, ExprIr, PackageLocalAbiSymbol, PackageRefIr, PackageTypeRef,
    ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

const PACKAGE_ID: &str = "example.com/compiler-owned-std-owner";

fn compile_with_contract(fixture: &str, source: &str) -> PublishedPackageProject {
    let temp = TestDir::new("skiff-compiler", &format!("compiler-owned-std-{fixture}"));
    temp.write("package.yml", format!("id: {PACKAGE_ID}\nversion: 1.0.0\n"));
    temp.write("api.yml", "run: main.run\n");
    temp.write("main.skiff", source);
    let dependencies = BTreeMap::from([(
        (PACKAGE_ID.to_string(), "1.0.0".to_string()),
        vec![package_contract_dependency("payments", probe_contract())],
    )]);
    compile_package_project_with_contract_dependencies(temp.path(), &dependencies)
        .expect("contract dependency and compiler-owned std must compile through one real package")
}

fn probe_contract() -> skiff_compiler::ServiceContract {
    let linkable = |owner| BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    };
    compile_service_contract(ServiceContractDefinition {
        service_id: "example.payments".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "input".to_string(),
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
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
        package_type_requirements: Vec::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Compiler-owned std owner probe".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("code-free probe contract must compile")
}

fn public_callable_id(
    package: &skiff_compiler::PublishedPackageArtifact,
    public_path: &str,
) -> skiff_artifact_model::PackageCallableId {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &package.artifact.package_local_abi.public_symbols[public_path]
    else {
        panic!("package should expose callable {public_path}")
    };
    callable_id.clone()
}

fn native_call<'a>(
    file: &'a skiff_compiler_emission::PublishedFileIrArtifact,
    binding_key: &str,
) -> Option<&'a CallIr> {
    file.unit.executables.iter().find_map(|executable| {
        executable.body.expressions.iter().find_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            let CallTargetIr::Native { target } = &call.target else {
                return None;
            };
            (target.binding_key.as_deref() == Some(binding_key)).then_some(call)
        })
    })
}

fn count_json_kind(value: &serde_json::Value, expected: &str) -> usize {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| count_json_kind(item, expected))
            .sum(),
        serde_json::Value::Object(fields) => {
            usize::from(fields.get("kind").and_then(serde_json::Value::as_str) == Some(expected))
                + fields
                    .values()
                    .map(|field| count_json_kind(field, expected))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_owned_std_exact_signatures_coexist_with_service_contract_dependency() {
        let project = compile_with_contract(
            "exact-signatures",
            r#"import std

function run(connectionId: string) -> void {
  std.time.sleep(1)
  std.websocket.sendTextToConnection(connectionId, "ready")
}
"#,
        );
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("canonical std artifact must remain in the package closure");
        let requirement = project
            .package
            .artifact
            .package_requirements
            .iter()
            .find(|requirement| requirement.alias == "std")
            .expect("lowered std refs must retain one exact package requirement");
        assert_eq!(requirement.package_id, std.artifact.package_id);
        assert_eq!(requirement.exact_version, std.artifact.package_version);
        assert_eq!(
            requirement.expected_local_abi,
            std.artifact.package_local_abi.local_abi_identity
        );
        assert_eq!(
            project.package.artifact.package_requirements.len(),
            1,
            "the code-free contract dependency must not leak a provider package artifact"
        );
        assert_eq!(project.package.artifact.contract_requirements.len(), 1);

        let main = module_artifact(&project.package, "main");
        for public_path in ["std.time.sleep", "std.websocket.sendTextToConnection"] {
            let callable_id = public_callable_id(std, public_path);
            assert!(
                native_call(main, public_path).is_some(),
                "lowering must retain the compiler-owned std native call {public_path} ({callable_id})"
            );
        }
    }

    #[test]
    fn compiler_owned_std_websocket_connect_type_owner_rehydrates_and_lowers() {
        let project = compile_with_contract(
            "websocket-connect-owner",
            r#"import std

function run(input: string) -> std.websocket.WebSocketConnectResult {
  return std.json.decode<std.websocket.WebSocketConnectResult>(input)
}
"#,
        );
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("canonical std artifact must remain in the package closure");
        let call = native_call(module_artifact(&project.package, "main"), "std.json.decode")
            .expect("generic std call must lower through the selected native registry");
        let ty = call.type_args.get("T0").unwrap_or_else(|| {
            panic!(
                "the exact std generic binder must survive lowering: {:?}",
                call.type_args
            )
        });
        let TypeRefIr::PackageSymbol { symbol } = ty else {
            panic!("std WebSocket type argument must retain its exact package owner: {ty:?}");
        };
        assert_eq!(
            symbol.package,
            PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string()
            }
        );
        assert_eq!(symbol.symbol_path, "std.websocket.WebSocketConnectResult");
        assert_eq!(
            project.package.artifact.package_requirements[0].expected_local_abi,
            std.artifact.package_local_abi.local_abi_identity
        );
    }

    #[test]
    fn compiler_owned_std_http_stream_uses_exact_symbol_owner_and_lowers() {
        let project = compile_with_contract(
            "http-stream-exact-owner",
            r#"import std

function run(input: std.http.HttpClientRequest) -> integer {
  final response = std.http.stream(input)
  return response.status
}
"#,
        );
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("canonical std artifact must remain in the package closure");
        let PackageLocalAbiSymbol::Callable { signature, .. } =
            &std.artifact.package_local_abi.public_symbols["std.http.stream"]
        else {
            panic!("std.http.stream must remain a public callable")
        };
        assert_eq!(
            signature.return_type,
            PackageTypeRef::Local {
                local_type: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "std.http".to_string(),
                        symbol: "HttpClientStreamHandle".to_string(),
                    },
                },
            }
        );
        assert_eq!(
            count_json_kind(
                &serde_json::to_value(&std.artifact.package_local_abi.public_symbols).unwrap(),
                "localType",
            ),
            0,
            "fresh official std public symbols must not contain ownerless LocalType values"
        );

        let stream_call = native_call(module_artifact(&project.package, "main"), "std.http.client.stream")
            .expect("the exact std.http.stream signature must lower through the canonical native registry binding");
        let CallTargetIr::Native { target } = &stream_call.target else {
            panic!("std.http.stream must lower through a native target")
        };
        assert_eq!(target.namespace, "std.http");
        assert_eq!(target.symbol, "stream");
        assert_eq!(
            target.binding_key.as_deref(),
            Some("std.http.client.stream")
        );
        assert!(stream_call.type_args.is_empty());
    }
}
