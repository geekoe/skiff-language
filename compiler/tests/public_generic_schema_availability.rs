mod common;

use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, NominalTypeRefBaseIr,
    PackageLocalAbiSymbol, PackageRefIr, PackageTypeRef, TypeRefIr,
};

#[test]
fn public_generic_declarations_remain_linkable_but_schema_ineligible() {
    let temp = TestDir::new("skiff-compiler", "public-generic-schema-availability");
    temp.write(
        "package.yml",
        "id: example.com/generic-api\nversion: 1.0.0\n",
    );
    temp.write(
        "api.yml",
        r#"GenericRecord: models.GenericRecord
GenericRepresentation: models.GenericRepresentation
GenericBranch: models.GenericBranch
GenericUnion: models.GenericUnion
GenericInterface: models.GenericInterface
TransitiveEnvelope: models.TransitiveEnvelope
Closed: models.Closed
roundTrip: models.roundTrip
"#,
    );
    temp.write(
        "models.skiff",
        r#"
type GenericRecord<T> {
  value: T,
}

type GenericRepresentation<T> = string

type GenericBranch<T> {
  value: T,
}

type GenericUnion<T> discriminator "tag" =
  GenericBranch<T>
  | { tag: "inline", value: T }

interface GenericInterface<T> {
  function read(self: Self, fallback: T) -> T
}

type TransitiveEnvelope {
  value: GenericRecord<string>,
}

type Closed {
  value: string,
}

function roundTrip(value: GenericRecord<string>) -> GenericRecord<string> {
  return value
}
"#,
    );

    let project = compile_package_project(temp.path())
        .expect("public generic declarations must not fail package publication");
    let package = &project.package;
    let generic_paths = [
        "GenericRecord",
        "GenericRepresentation",
        "GenericBranch",
        "GenericUnion",
        "GenericInterface",
    ];
    for path in generic_paths {
        let symbol = package
            .artifact
            .package_local_abi
            .public_symbols
            .get(path)
            .unwrap_or_else(|| panic!("{path} must remain in PackageLocalAbi"));
        let PackageLocalAbiSymbol::Type { type_params, .. } = symbol else {
            panic!("{path} must remain a type symbol");
        };
        assert_eq!(type_params, &["T".to_string()], "{path}");
        assert!(
            package
                .artifact
                .implementation_links
                .types
                .contains_key(path),
            "{path} must retain an exact implementation link"
        );
        assert!(
            !package.package_schema_index.types.contains_key(path),
            "{path} must not acquire a generic PackageSchema record"
        );
    }
    assert!(
        !package
            .package_schema_index
            .types
            .contains_key("TransitiveEnvelope"),
        "a non-generic owner that transitively uses an applied nominal must be omitted"
    );
    assert!(
        package
            .artifact
            .package_local_abi
            .public_symbols
            .contains_key("TransitiveEnvelope"),
        "schema ineligibility must not remove the transitive owner from PackageLocalAbi"
    );
    assert!(
        package
            .artifact
            .implementation_links
            .types
            .contains_key("TransitiveEnvelope"),
        "schema ineligibility must not remove the transitive owner implementation link"
    );
    assert_eq!(
        package
            .package_schema_index
            .types
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["Closed"]
    );
    assert_eq!(package.package_schema_type_records.len(), 1);

    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = &package.artifact.package_local_abi.public_symbols["roundTrip"]
    else {
        panic!("roundTrip must remain in PackageLocalAbi");
    };
    assert_applied_local_type(&signature.parameters[0].ty, "models", "GenericRecord");
    assert_applied_local_type(&signature.return_type, "models", "GenericRecord");
    assert!(matches!(
        &package.artifact.boundary_projections[callable_id],
        BoundaryCallableProjection::Unavailable { reasons }
            if reasons == &[BoundaryUnavailableReason::UnsupportedBoundaryType]
    ));

    let std = project
        .dependency("skiff.run/std", "1.0.0")
        .expect("canonical std artifact");
    for path in [
        "std.websocket.WebSocketConnection",
        "std.websocket.WebSocketReceiveEvent",
        "std.websocket.WebSocketIngressEvent",
        "std.websocket.WebSocketConnectResult",
    ] {
        assert!(matches!(
            std.artifact.package_local_abi.public_symbols.get(path),
            Some(PackageLocalAbiSymbol::Type { type_params, .. })
                if type_params == &["Context".to_string()]
        ));
        assert!(std.artifact.implementation_links.types.contains_key(path));
        assert!(!std.package_schema_index.types.contains_key(path));
    }
    assert!(
        std.package_schema_index
            .types
            .contains_key("std.service.InternalError"),
        "ordinary schema-closed std errors must retain their exact record"
    );
}

#[test]
fn public_generic_package_imports_preserve_owner_and_reject_wrong_arity() {
    let temp = TestDir::new("skiff-compiler", "public-generic-dependency-import");
    temp.write(
        "package.yml",
        r#"id: example.com/generic-consumer
version: 1.0.0
packages:
  - id: example.com/generic-provider
    version: 1.0.0
    alias: models
"#,
    );
    temp.write("api.yml", "echo: main.echo\n");
    temp.write(
        "main.skiff",
        r#"
import models

function echo(value: models.Box<string>) -> models.Box<string> {
  return value
}
"#,
    );
    temp.write(
        ".skiff-packages/example~com~~generic-provider/1.0.0/package.yml",
        "id: example.com/generic-provider\nversion: 1.0.0\n",
    );
    temp.write(
        ".skiff-packages/example~com~~generic-provider/1.0.0/api.yml",
        "Box: models.Box\n",
    );
    temp.write(
        ".skiff-packages/example~com~~generic-provider/1.0.0/models.skiff",
        "type Box<T> { value: T }\n",
    );

    let project = compile_package_project(temp.path())
        .expect("a real artifact dependency must expose its public generic declaration");
    let provider = project
        .dependency("example.com/generic-provider", "1.0.0")
        .expect("generic provider artifact");
    assert!(provider
        .artifact
        .package_local_abi
        .public_symbols
        .contains_key("Box"));
    assert!(provider
        .artifact
        .implementation_links
        .types
        .contains_key("Box"));
    assert!(!provider.package_schema_index.types.contains_key("Box"));

    let PackageLocalAbiSymbol::Callable { signature, .. } =
        &project.package.artifact.package_local_abi.public_symbols["echo"]
    else {
        panic!("echo must remain a callable");
    };
    assert_applied_dependency_type(&signature.parameters[0].ty, "models", "Box");
    assert_applied_dependency_type(&signature.return_type, "models", "Box");

    temp.write(
        "main.skiff",
        r#"
import models

function echo(value: models.Box<string, integer>) -> void {
}
"#,
    );
    let error = compile_package_project(temp.path())
        .expect_err("wrong generic arity must remain fail closed")
        .to_string();
    assert!(
        error.contains("expects 1 type arguments, found 2"),
        "unexpected wrong-arity diagnostic: {error}"
    );
}

fn assert_applied_local_type(ty: &PackageTypeRef, module: &str, symbol: &str) {
    let PackageTypeRef::Local {
        local_type:
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::ServiceSymbol { symbol: owner },
                arguments,
            },
    } = ty
    else {
        panic!("expected an applied package-local nominal, found {ty:#?}");
    };
    assert_eq!(owner.module_path, module);
    assert_eq!(owner.symbol, symbol);
    assert_eq!(arguments, &[TypeRefIr::builtin("string")]);
}

fn assert_applied_dependency_type(ty: &PackageTypeRef, alias: &str, symbol_path: &str) {
    let PackageTypeRef::Local {
        local_type:
            TypeRefIr::AppliedNominal {
                base:
                    NominalTypeRefBaseIr::PackageSymbol {
                        symbol:
                            skiff_artifact_model::PackageSymbolRef {
                                package: PackageRefIr::Dependency { dependency_ref },
                                symbol_path: actual_path,
                                ..
                            },
                    },
                arguments,
            },
    } = ty
    else {
        panic!("expected an applied dependency nominal, found {ty:#?}");
    };
    assert_eq!(dependency_ref, alias);
    assert_eq!(actual_path, symbol_path);
    assert_eq!(arguments, &[TypeRefIr::builtin("string")]);
}
