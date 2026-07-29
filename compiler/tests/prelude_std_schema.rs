use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryStreamContract, BoundaryUnavailableReason,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
    PackageLocalAbiSymbol, PackageRefIr, TypeDescriptorIr, TypeRefIr,
};

mod common;
use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};

#[test]
fn prelude_types_compile_on_the_canonical_package_owner() {
    let temp = TestDir::new("skiff-compiler", "prelude-package-owner");
    temp.write(
        "package.yml",
        "id: example.com/prelude-owner\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "inspect: prelude.inspect\n");
    temp.write(
        "prelude.skiff",
        r#"function inspect(
  flag: bool,
  count: integer,
  request: HttpRequest
) -> bool {
  return flag
}
"#,
    );

    let project = compile_package_project(temp.path()).expect("prelude types should compile");
    assert_eq!(
        project.package.artifact.package_id,
        "example.com/prelude-owner"
    );
    assert!(module_artifact(&project.package, "prelude")
        .unit
        .declarations
        .executables
        .contains_key("inspect"));
    assert!(project
        .artifacts()
        .all(|package| package.artifact.package_id != "skiff.run/core"));
}

#[test]
fn explicit_std_import_materializes_a_canonical_dependency() {
    let temp = TestDir::new("skiff-compiler", "explicit-std-package");
    temp.write(
        "package.yml",
        "id: example.com/std-consumer\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "RequestBox: main.RequestBox\n");
    temp.write(
        "main.skiff",
        r#"import std

type RequestBox { request: std.http.HttpClientRequest }
"#,
    );

    let project = compile_package_project(temp.path()).expect("explicit std import should compile");
    let std = project
        .dependency("skiff.run/std", "1.0.0")
        .expect("std should be in the canonical dependency closure");
    assert!(!std.artifact.package_local_abi.public_symbols.is_empty());
    assert!(
        project
            .package
            .artifact
            .package_requirements
            .iter()
            .any(|requirement| requirement.package_id == "skiff.run/std"
                && requirement.alias == "std")
    );
    assert!(project
        .artifacts()
        .all(|package| package.artifact.package_id != "skiff.run/core"));
}

#[test]
fn builtin_types_reach_the_package_boundary_projection() {
    let temp = TestDir::new("skiff-compiler", "prelude-boundary-builtins");
    temp.write(
        "package.yml",
        "id: example.com/prelude-builtins\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "check: builtins.check\n");
    temp.write(
        "builtins.skiff",
        r#"function check(flag: bool, count: integer) -> integer {
  return 1
}
"#,
    );

    let project = compile_package_project(temp.path()).expect("builtin boundary should compile");
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &project.package.artifact.package_local_abi.public_symbols["check"]
    else {
        panic!("check must be a canonical package callable");
    };
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = &project.package.artifact.boundary_projections[callable_id]
    else {
        panic!("bool/integer package callable must be boundary-available");
    };

    assert_eq!(
        operation_contract.parameters[0].ty,
        ContractTypeRef::builtin("bool")
    );
    assert_eq!(
        operation_contract.parameters[1].ty,
        ContractTypeRef::builtin("integer")
    );
    assert_eq!(
        operation_contract.return_value.ty,
        ContractTypeRef::builtin("integer")
    );

    let wire = serde_json::to_value(&project.package.artifact.boundary_projections).unwrap();
    let text = wire.to_string();
    for forbidden in [
        "operationId",
        "stableKey",
        "serviceProtocolIdentity",
        "providerPackageId",
        "deploymentRevision",
    ] {
        assert!(
            !text.contains(forbidden),
            "package boundary projection leaked {forbidden}: {wire}"
        );
    }
}

#[test]
fn imported_http_types_reach_unary_and_stream_boundary_projection() {
    let temp = TestDir::new("skiff-compiler", "imported-http-boundary-types");
    temp.write(
        "package.yml",
        "id: example.com/http-boundary\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "handle: main.handle\nstream: main.stream\n");
    temp.write(
        "main.skiff",
        r#"import std

function handle(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}

function stream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  emit(std.http.streamChunk(request.body))
  emit(std.http.streamEnd())
  return null
}
"#,
    );

    let project =
        compile_package_project(temp.path()).expect("imported HTTP boundary source should compile");
    for public_name in ["handle", "stream"] {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } =
            &project.package.artifact.package_local_abi.public_symbols[public_name]
        else {
            panic!("{public_name} must be a canonical package callable")
        };
        let projection = &project.package.artifact.boundary_projections[callable_id];
        if let BoundaryCallableProjection::Unavailable { reasons } = projection {
            assert!(
                !reasons.contains(&BoundaryUnavailableReason::UnsupportedBoundaryType),
                "{public_name} must admit its imported HTTP types before independent semantic eligibility closes: {reasons:?}"
            );
        }
    }
}

#[test]
fn callback_type_is_explicitly_boundary_unavailable() {
    let temp = TestDir::new("skiff-compiler", "callback-boundary-package");
    temp.write("package.yml", "id: example.com/callbacks\nversion: 1.0.0\n");
    temp.write("api.yml", "run: callback.run\n");
    temp.write(
        "callback.skiff",
        r#"function run(callback: fn(value: string) -> string) -> void {
  return
}
"#,
    );

    let project = compile_package_project(temp.path()).expect("callback package should compile");
    let projection = project
        .package
        .artifact
        .boundary_projections
        .values()
        .next()
        .expect("exported callback callable should have a projection");
    assert!(matches!(
        projection,
        BoundaryCallableProjection::Unavailable { reasons }
            if reasons.contains(&BoundaryUnavailableReason::CallbackAdapterUnavailable)
    ));
}

#[test]
fn stream_type_projects_an_explicit_server_stream_boundary() {
    let temp = TestDir::new("skiff-compiler", "stream-boundary-package");
    temp.write("package.yml", "id: example.com/stream\nversion: 1.0.0\n");
    temp.write("api.yml", "events: stream.events\n");
    temp.write(
        "stream.skiff",
        "function events() -> Stream<string> { return }\n",
    );

    let project = compile_package_project(temp.path()).expect("stream package should compile");
    let projection = project
        .package
        .artifact
        .boundary_projections
        .values()
        .next()
        .expect("exported stream callable should have a projection");
    assert!(matches!(
        projection,
        BoundaryCallableProjection::Available {
            operation_contract,
            ..
        } if matches!(
            &operation_contract.stream,
            BoundaryStreamContract::ServerStream {
                item_type,
                item_value_plan: BoundaryValuePlan::Linkable {
                    owner: BoundaryValueOwner::Provider,
                    lifetime: BoundaryValueLifetime::Stream,
                    ..
                },
            } if item_type == &ContractTypeRef::builtin("string")
        )
    ));
}

#[test]
fn package_local_transport_names_remain_ordinary_types() {
    for reserved in ["HttpRequest", "ConnectionMessage"] {
        let temp = TestDir::new("skiff-compiler", &format!("package-local-{reserved}"));
        temp.write(
            "package.yml",
            "id: example.com/local-types\nversion: 1.0.0\n",
        );
        temp.write(
            "api.yml",
            &format!("run: main.run\n{reserved}: main.{reserved}\n"),
        );
        temp.write(
            "main.skiff",
            &format!("type {reserved} {{}}\nfunction run() -> void {{}}\n"),
        );

        let project = compile_package_project(temp.path())
            .expect("package-local names must not infer a transport contract");
        assert!(matches!(
            project
                .package
                .artifact
                .package_local_abi
                .public_symbols
                .get(reserved),
            Some(PackageLocalAbiSymbol::Type { .. })
        ));
    }
}

#[test]
fn configured_api_yml_is_the_only_package_schema_surface() {
    let temp = TestDir::new("skiff-compiler", "configured-api-surface");
    temp.write(
        "package.yml",
        "id: example.com/configured-api\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "run: internal.handler.run\n");
    temp.write(
        "api/ignored.skiff",
        "type UnconfiguredType { value: string }\n",
    );
    temp.write("internal/handler.skiff", "function run() -> void {}\n");

    let project = compile_package_project(temp.path()).expect("configured API should compile");
    let public_symbols = &project.package.artifact.package_local_abi.public_symbols;
    assert!(public_symbols.contains_key("run"));
    assert!(!public_symbols.contains_key("UnconfiguredType"));
    assert!(!public_symbols.contains_key("api.ignored.UnconfiguredType"));
}

#[test]
fn prelude_builtin_schema_is_typed_in_file_ir() {
    let temp = TestDir::new("skiff-compiler", "prelude-file-ir-schema");
    temp.write(
        "package.yml",
        "id: example.com/prelude-schema\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "Builtins: schema.Builtins\n");
    temp.write(
        "schema.skiff",
        "type Builtins { flag: bool, count: integer }\n",
    );

    let project = compile_package_project(temp.path()).expect("prelude schema should compile");
    let file = module_artifact(&project.package, "schema");
    let builtins = file
        .unit
        .type_table
        .iter()
        .find(|ty| ty.name == "Builtins")
        .expect("Builtins should be typed in File IR");
    let TypeDescriptorIr::Record { fields } = &builtins.descriptor else {
        panic!("Builtins should remain a record");
    };
    assert_eq!(fields["flag"], TypeRefIr::builtin("bool"));
    assert_eq!(fields["count"], TypeRefIr::builtin("integer"));
}

#[test]
fn qualified_std_schema_refs_reach_file_ir_and_dependency_closure() {
    let temp = TestDir::new("skiff-compiler", "qualified-std-file-ir-schema");
    temp.write(
        "package.yml",
        "id: example.com/http-client-schema\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "Exchange: schema.Exchange\n");
    temp.write(
        "schema.skiff",
        r#"import std

type Exchange {
  request: std.http.HttpClientRequest,
  response: std.http.HttpClientResponse,
}
"#,
    );

    let project = compile_package_project(temp.path()).expect("std schema refs should compile");
    let schema = module_artifact(&project.package, "schema");
    for expected in ["std.http.HttpClientRequest", "std.http.HttpClientResponse"] {
        assert!(
            schema
                .unit
                .external_refs
                .package_symbols
                .iter()
                .any(|symbol| {
                    symbol.symbol_path == expected
                        && matches!(
                            &symbol.package,
                            PackageRefIr::PackageId { package_id }
                                if package_id == "skiff.run/std"
                        )
                }),
            "File IR should reference canonical std symbol {expected}"
        );
    }
    assert!(project.dependency("skiff.run/std", "1.0.0").is_some());
}
