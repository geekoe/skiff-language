mod common;

use common::{
    artifacts::{module_artifact, source_artifact},
    package_project::compile_package_project,
    TestDir,
};
use skiff_artifact_model::{
    BoundaryCallableProjection, CallIr, CallTargetIr, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableTargetFact, ExprIr, PackageCallableId,
    PackageLocalAbiSymbol, PackageRefIr, TypeDescriptorIr, TypeRefIr, ValueProvenance,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_emission::PublishedFileIrArtifact;

fn std_requirement(
    package: &skiff_compiler::PublishedPackageArtifact,
) -> &skiff_artifact_model::PackageRequirement {
    package
        .artifact
        .package_requirements
        .iter()
        .find(|requirement| requirement.package_id == SKIFF_STD_PUBLICATION_ID)
        .expect("consumer should carry the canonical std requirement")
}

fn public_callable_id(
    package: &skiff_compiler::PublishedPackageArtifact,
    public_path: &str,
) -> PackageCallableId {
    let Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) = package
        .artifact
        .package_local_abi
        .public_symbols
        .get(public_path)
    else {
        panic!("package should expose callable {public_path}");
    };
    callable_id.clone()
}

fn file_contains_call(
    file: &PublishedFileIrArtifact,
    predicate: &impl Fn(&CallTargetIr) -> bool,
) -> bool {
    file.unit.executables.iter().any(|executable| {
        executable.body.expressions.iter().any(
            |expression| matches!(expression, ExprIr::Call { call } if predicate(&call.target)),
        )
    })
}

fn assert_native_wrapper_type_args(
    file: &PublishedFileIrArtifact,
    symbol: &str,
    expected: &[(&str, &str)],
) {
    let call = native_wrapper_call(file, symbol);
    let actual = call
        .type_args
        .iter()
        .map(|(key, ty)| {
            let TypeRefIr::TypeParam { name } = ty else {
                panic!("native wrapper type arg {key} must be a type parameter: {ty:?}");
            };
            (key.as_str(), name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn native_wrapper_call<'a>(file: &'a PublishedFileIrArtifact, symbol: &str) -> &'a CallIr {
    let expected_symbol = format!("{}.{}", file.module_path, symbol);
    let executable = file
        .unit
        .executables
        .iter()
        .find(|executable| executable.symbol == expected_symbol)
        .unwrap_or_else(|| panic!("{} should declare {expected_symbol}", file.source_path));
    executable
        .body
        .expressions
        .iter()
        .find_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            let CallTargetIr::Native { target } = &call.target else {
                return None;
            };
            (target.namespace == file.module_path && target.symbol == symbol).then_some(call)
        })
        .unwrap_or_else(|| panic!("{expected_symbol} should call its native target"))
}

fn file_contains_std_package_type(file: &PublishedFileIrArtifact, symbol_path: &str) -> bool {
    file_contains_type_ref(file, &|ty| {
        matches!(
            ty,
            TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id }
                        if package_id == SKIFF_STD_PUBLICATION_ID
                ) && symbol.symbol_path == symbol_path
        )
    })
}

fn assert_direct_type_ref(
    consumer: &PublishedFileIrArtifact,
    declaration_owner: &PublishedFileIrArtifact,
    symbol: &str,
) {
    let type_index = declaration_owner.unit.declarations.types[symbol].type_index;
    let expected = if consumer.module_path == declaration_owner.module_path {
        TypeRefIr::LocalType { type_index }
    } else {
        TypeRefIr::PublicationType {
            module_path: declaration_owner.module_path.clone(),
            type_index,
        }
    };
    assert!(file_contains_type_ref(consumer, &|ty| ty == &expected));
}

fn file_contains_type_ref(
    file: &PublishedFileIrArtifact,
    predicate: &impl Fn(&TypeRefIr) -> bool,
) -> bool {
    file.unit.type_table.iter().any(|ty| {
        descriptor_contains_type_ref(&ty.descriptor, predicate)
            || ty
                .implements
                .iter()
                .any(|implemented| type_ref_contains(implemented, predicate))
    }) || file
        .unit
        .constants
        .iter()
        .any(|constant| type_ref_contains(&constant.ty, predicate))
        || file.unit.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_contains(&param.ty, predicate))
                || type_ref_contains(&executable.return_type, predicate)
                || executable
                    .body
                    .expressions
                    .iter()
                    .any(|expression| expression_contains_type_ref(expression, predicate))
        })
}

fn expression_contains_type_ref(
    expression: &ExprIr,
    predicate: &impl Fn(&TypeRefIr) -> bool,
) -> bool {
    match expression {
        ExprIr::Construct { type_ref, .. } => type_ref_contains(type_ref, predicate),
        ExprIr::Catch { catch_type, .. } => type_ref_contains(catch_type, predicate),
        ExprIr::Throw { payload_type, .. } => type_ref_contains(payload_type, predicate),
        ExprIr::DbOperation { operation } => type_ref_contains(&operation.result_type, predicate),
        ExprIr::DbQuery { query } => type_ref_contains(&query.result_type, predicate),
        ExprIr::DbTransaction { transaction } => {
            type_ref_contains(&transaction.result_type, predicate)
        }
        ExprIr::DbLeaseClaim { claim } => type_ref_contains(&claim.result_type, predicate),
        ExprIr::DbLeaseRead { read } => type_ref_contains(&read.result_type, predicate),
        _ => false,
    }
}

fn descriptor_contains_type_ref(
    descriptor: &TypeDescriptorIr,
    predicate: &impl Fn(&TypeRefIr) -> bool,
) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains(field, predicate)),
        TypeDescriptorIr::Alias { target } => type_ref_contains(target, predicate),
        TypeDescriptorIr::Representation { representation } => {
            type_ref_contains(representation, predicate)
        }
        TypeDescriptorIr::Union { branches } => branches.iter().any(|branch| match branch {
            skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                type_ref_contains(nominal_type, predicate)
            }
            skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type,
                ..
            } => type_ref_contains(payload_type, predicate),
            skiff_artifact_model::NamedUnionBranchIr::Literal { .. } => false,
        }),
        TypeDescriptorIr::Interface => false,
    }
}

fn type_ref_contains(ty: &TypeRefIr, predicate: &impl Fn(&TypeRefIr) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    match ty {
        TypeRefIr::Builtin { args, .. } => args.iter().any(|arg| type_ref_contains(arg, predicate)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_contains(field, predicate)),
        TypeRefIr::Union { items } => items.iter().any(|item| type_ref_contains(item, predicate)),
        TypeRefIr::Nullable { inner } => type_ref_contains(inner, predicate),
        TypeRefIr::AppliedNominal { arguments, .. } => arguments
            .iter()
            .any(|argument| type_ref_contains(argument, predicate)),
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_contains(arg, predicate)),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_contains(&param.ty, predicate))
                || type_ref_contains(return_type, predicate)
        }
        TypeRefIr::PackageSchema { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_packages_reject_native_declarations() {
        for (name, declaration, expected) in [
            (
                "native-function",
                "native function hostOnly() -> string\n",
                "cannot declare native function hostOnly",
            ),
            ("native-type", "native type HostOnly\n", "expected function"),
        ] {
            let temp = TestDir::new("skiff-compiler", name);
            temp.write(
                "package.yml",
                format!("id: example.com/{name}\nversion: 1.0.0\n"),
            );
            temp.write("api.yml", "{}\n");
            temp.write("main.skiff", declaration);

            let error = compile_package_project(temp.path())
                .expect_err("user package native declarations must fail closed")
                .to_string();
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }

    #[test]
    fn truncate_utf8_bytes_projects_available() {
        let temp = TestDir::new("skiff-compiler", "truncate-utf8-bytes-projection");
        temp.write(
            "package.yml",
            "id: example.com/truncate-projection\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "truncate: main.truncate\n");
        temp.write(
            "main.skiff",
            r#"import std

function truncate(value: string, maxBytes: number) -> string {
  return std.string.truncateUtf8Bytes(value, maxBytes)
}
"#,
        );

        let project =
            compile_package_project(temp.path()).expect("truncate wrapper should compile");
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("std should be in the canonical dependency closure");
        let truncate_callable_id = public_callable_id(std, "std.string.truncateUtf8Bytes");
        let callable_id = public_callable_id(&project.package, "truncate");
        let facts = &project.package.artifact.callable_semantic_facts[&callable_id];
        assert_eq!(
            facts.effects,
            CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                },
            }
        );
        assert_eq!(
            facts.provenance,
            CallableProvenanceSummary::Analyzed {
                return_origins: vec![
                    ValueProvenance::Fresh,
                    ValueProvenance::DependencyReturn {
                        callable_id: truncate_callable_id.as_str().to_string(),
                    },
                ],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            }
        );
        assert_eq!(facts.resolved_call_targets.len(), 1);
        assert!(facts.resolved_call_targets.values().any(|target| {
            matches!(
                target,
                CallableTargetFact::PackageDirect {
                    package_callable_id,
                } if package_callable_id == truncate_callable_id.as_str()
            )
        }));
        assert!(matches!(
            project.package.artifact.boundary_projections[&callable_id],
            BoundaryCallableProjection::Available { .. }
        ));

        let main = module_artifact(&project.package, "main");
        assert!(file_contains_call(main, &|target| {
            matches!(
                target,
                CallTargetIr::PackageCallable {
                    package_ref: PackageRefIr::Dependency { dependency_ref },
                    package_callable_id,
                } if dependency_ref == "std" && package_callable_id == &truncate_callable_id
            )
        }));
    }

    #[test]
    fn http_request_native_helpers_project_available_from_imported_source() {
        let temp = TestDir::new("skiff-compiler", "http-request-native-projection");
        temp.write(
            "package.yml",
            "id: example.com/http-request-native-projection\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "handler: main.handler\n");
        temp.write(
            "main.skiff",
            r#"import std

function cookieValue(request: std.http.HttpRequest) -> string? {
  return std.http.cookie(request, "session")
}

function headerValues(request: std.http.HttpRequest) -> Array<string> {
  return std.http.headers(request, "x-trace")
}

function handler(request: std.http.HttpRequest) -> std.http.HttpResponse {
  const values = headerValues(request)
  const session = cookieValue(request)
  return std.http.HttpResponse {
    status: 200,
    headers: Array.empty<std.http.HttpHeader>(),
    body: bytes.fromUtf8("ok"),
  }
}
"#,
        );

        let project = compile_package_project(temp.path())
            .expect("HTTP request native handler should compile");
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("std should be in the canonical dependency closure");
        let callable_id = public_callable_id(&project.package, "handler");
        let facts = &project.package.artifact.callable_semantic_facts[&callable_id];
        assert_eq!(
            facts.effects,
            CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: false,
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                },
            }
        );
        assert_eq!(
            facts.provenance,
            CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh, ValueProvenance::Constant],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            }
        );
        assert!(matches!(
            project.package.artifact.boundary_projections[&callable_id],
            BoundaryCallableProjection::Available { .. }
        ));

        let main = module_artifact(&project.package, "main");
        for public_path in ["std.http.headers", "std.http.cookie"] {
            let package_callable_id = public_callable_id(std, public_path);
            assert!(file_contains_call(main, &|target| {
                matches!(
                    target,
                    CallTargetIr::PackageCallable {
                        package_ref: PackageRefIr::Dependency { dependency_ref },
                        package_callable_id: actual_callable_id,
                    } if dependency_ref == "std" && actual_callable_id == &package_callable_id
                )
            }));
        }
    }

    #[test]
    fn std_root_import_materializes_exact_requirement_and_typed_log_call() {
        let temp = TestDir::new("skiff-compiler", "std-root-import");
        temp.write(
            "package.yml",
            "id: example.com/std-consumer\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "Marker: main.Marker\nrun: main.run\n");
        temp.write(
            "main.skiff",
            r#"import std

function run() -> void {
  std.log.info("hello", null)
}

type Marker { request: std.http.HttpRequest }
"#,
        );

        let project = compile_package_project(temp.path()).expect("std consumer should compile");
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("std should be in the canonical dependency closure");
        assert_eq!(project.dependency_packages.len(), 1);

        let requirement = std_requirement(&project.package);
        assert_eq!(requirement.alias, "std");
        assert_eq!(requirement.package_id, std.artifact.package_id);
        assert_eq!(requirement.exact_version, std.artifact.package_version);
        assert_eq!(
            requirement.expected_local_abi,
            std.artifact.package_local_abi.local_abi_identity
        );

        let log_callable_id = public_callable_id(std, "std.log.info");
        let main = module_artifact(&project.package, "main");
        assert!(main
            .unit
            .external_refs
            .package_callables
            .iter()
            .any(|reference| {
                reference.package_ref
                    == (PackageRefIr::Dependency {
                        dependency_ref: "std".to_string(),
                    })
                    && reference.package_callable_id == log_callable_id
            }));
        assert!(file_contains_call(main, &|target| {
            matches!(
                target,
                CallTargetIr::PackageCallable {
                    package_ref: PackageRefIr::Dependency { dependency_ref },
                    package_callable_id,
                } if dependency_ref == "std" && package_callable_id == &log_callable_id
            )
        }));

        let run_callable_id = public_callable_id(&project.package, "run");
        let run_facts = &project.package.artifact.callable_semantic_facts[&run_callable_id];
        assert!(matches!(
            run_facts.effects,
            CallableEffectSummary::Analyzed { .. }
        ));

        assert_eq!(std.artifact.package_local_abi.public_symbols.len(), 91);
        for public_path in [
            "std.bytes.DecodeError",
            "std.crypto.sha256",
            "std.db.ConflictError",
            "std.db.ConstraintError",
            "std.file.ImmutableFile",
            "std.http.HttpRequest",
            "std.http.json",
            "std.json.DecodeError",
            "std.json.decode",
            "std.log.info",
            "std.service.ProtocolError",
            "std.telemetry.emit",
            "std.time.sleep",
            "std.websocket.WebSocketConnectRequest",
            "std.websocket.WebSocketConnectionPolicy",
            "std.websocket.WebSocketConnectResult",
        ] {
            assert!(
                std.artifact
                    .package_local_abi
                    .public_symbols
                    .contains_key(public_path),
                "std local ABI should contain {public_path}"
            );
        }
        for removed in [
            "std.websocket.TextConnectionMessage",
            "std.websocket.BinaryConnectionMessage",
            "std.websocket.ConnectionMessage",
            "std.websocket.WebSocketConnection",
            "std.websocket.WebSocketReceiveEvent",
            "std.websocket.WebSocketIngressEvent",
            "std.websocket.WebSocketCloseEvent",
        ] {
            assert!(
                !std.artifact
                    .package_local_abi
                    .public_symbols
                    .contains_key(removed),
                "std local ABI must not contain removed {removed}"
            );
        }
        assert!(std.artifact.callable_links.contains_key(&log_callable_id));

        let log = source_artifact(std, "log.skiff");
        let telemetry = source_artifact(std, "telemetry.skiff");
        assert!(file_contains_call(log, &|target| {
            matches!(
                target,
                CallTargetIr::Builtin { op } if op == "root.telemetry.emit"
            )
        }));

        assert_native_wrapper_type_args(
            source_artifact(std, "json.skiff"),
            "decode",
            &[("T0", "T")],
        );
        assert_native_wrapper_type_args(
            source_artifact(std, "json.skiff"),
            "encode",
            &[("T0", "T")],
        );
        assert_native_wrapper_type_args(
            source_artifact(std, "http.skiff"),
            "decodeJson",
            &[("T0", "T")],
        );
        assert_native_wrapper_type_args(source_artifact(std, "http.skiff"), "json", &[("T0", "T")]);
        assert_native_wrapper_type_args(source_artifact(std, "http.skiff"), "noContent", &[]);
        assert_native_wrapper_type_args(telemetry, "emit", &[]);
    }

    #[test]
    fn standard_error_catch_types_are_public_package_symbols() {
        let temp = TestDir::new("skiff-compiler", "std-error-catch-types");
        temp.write(
            "package.yml",
            "id: example.com/std-errors\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "check: main.check\n");
        temp.write(
            "main.skiff",
            r#"import std

function check() -> bool {
  const jsonResult = catch<std.json.DecodeError>(std.json.decode<string>("{}"))
  const numberResult = catch<std.number.DecodeError>(number.assertSafeInteger(1.5))
  const exactNumber = std.number.assertSafeInteger(1)
  const timeResult = catch<std.time.DecodeError>(null)
  const dbResult = catch<std.db.ConflictError>(null)
  const constraintResult = catch<std.db.ConstraintError>(null)
  return exactNumber == 1
}
"#,
        );

        let project = compile_package_project(temp.path()).expect("std catch types should compile");
        let main = module_artifact(&project.package, "main");
        assert!(file_contains_call(main, &|target| {
            matches!(
                target,
                CallTargetIr::Native { target }
                    if target.binding_key.as_deref() == Some("core.number.assertSafeInteger")
            )
        }));
        for symbol_path in [
            "std.json.DecodeError",
            "std.number.DecodeError",
            "std.time.DecodeError",
            "std.db.ConflictError",
            "std.db.ConstraintError",
        ] {
            assert!(file_contains_std_package_type(main, symbol_path));
        }
    }

    #[test]
    fn std_normal_types_use_package_symbols_and_internal_direct_refs() {
        let temp = TestDir::new("skiff-compiler", "std-normal-types");
        temp.write("package.yml", "id: example.com/std-types\nversion: 1.0.0\n");
        temp.write("api.yml", "Envelope: types.Envelope\n");
        temp.write(
            "types.skiff",
            r#"import std

type Envelope {
  request: std.http.HttpRequest,
  event: std.http.HttpResponseStreamEvent,
  file: std.file.ImmutableFile,
  gateway: std.websocket.WebSocketConnectResult,
  connect: std.websocket.WebSocketConnectRequest,
  policy: std.websocket.WebSocketConnectionPolicy,
  raw: Json,
  bytesValue: bytes,
}
"#,
        );

        let project = compile_package_project(temp.path()).expect("std types should compile");
        let consumer = module_artifact(&project.package, "types");
        for symbol_path in [
            "std.http.HttpRequest",
            "std.http.HttpResponseStreamEvent",
            "std.file.ImmutableFile",
            "std.websocket.WebSocketConnectResult",
            "std.websocket.WebSocketConnectRequest",
            "std.websocket.WebSocketConnectionPolicy",
        ] {
            assert!(file_contains_std_package_type(consumer, symbol_path));
        }

        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("std should be in the dependency closure");
        let http = source_artifact(std, "http.skiff");
        for symbol in [
            "HttpClientRequest",
            "HttpClientResponse",
            "HttpResponseStreamEvent",
            "HttpSseEvent",
        ] {
            assert_direct_type_ref(http, http, symbol);
        }
        let websocket = source_artifact(std, "websocket.skiff");
        for symbol in ["HttpHeader", "HttpQueryParam"] {
            assert_direct_type_ref(websocket, http, symbol);
        }
    }

    #[test]
    fn implicit_std_types_close_requirements_while_prelude_json_object_stays_local() {
        let schema = TestDir::new("skiff-compiler", "implicit-std-schema");
        schema.write("package.yml", "id: example.com/schema\nversion: 1.0.0\n");
        schema.write("api.yml", "Schema: schema.Schema\n");
        schema.write(
            "schema.skiff",
            "type Schema { request: std.http.HttpRequest }\n",
        );

        let project =
            compile_package_project(schema.path()).expect("implicit std type should compile");
        let std = project
            .dependency(SKIFF_STD_PUBLICATION_ID, "1.0.0")
            .expect("implicit std type should close over std");
        let requirement = std_requirement(&project.package);
        assert_eq!(requirement.exact_version, std.artifact.package_version);
        assert_eq!(
            requirement.expected_local_abi,
            std.artifact.package_local_abi.local_abi_identity
        );
        assert!(file_contains_std_package_type(
            module_artifact(&project.package, "schema"),
            "std.http.HttpRequest"
        ));

        let prelude = TestDir::new("skiff-compiler", "prelude-json-object");
        prelude.write(
            "package.yml",
            "id: example.com/prelude-json\nversion: 1.0.0\n",
        );
        prelude.write("api.yml", "Output: types.Output\n");
        prelude.write("types.skiff", "type Output { raw: JsonObject }\n");

        let project =
            compile_package_project(prelude.path()).expect("prelude JsonObject should compile");
        assert!(project.package.artifact.package_requirements.is_empty());
        assert!(project.dependency_packages.is_empty());
        assert!(file_contains_type_ref(
            module_artifact(&project.package, "types"),
            &|ty| matches!(ty, TypeRefIr::Builtin { name, args } if name == "JsonObject" && args.is_empty())
        ));
    }
}
