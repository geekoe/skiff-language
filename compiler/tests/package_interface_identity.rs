mod common;

use std::collections::BTreeMap;

use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_identity::{interface_instantiation_ref, package_artifact_ref};
use skiff_artifact_model::{
    InterfaceInstantiationRef, PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_compiler_source::{
    PackageDependency, PublicationTypeSymbolIndex, ResolvedTypeRef, TypeResolutionModel,
};

const DIRECT_PROVIDER_ID: &str = "example.com/interface-provider";
const DIRECT_PROVIDER_VERSION: &str = "1.0.0";
const DIRECT_PROVIDER_BUILD_ID: &str =
    "skiff-package-build-v10:sha256:7b862c58c4a51f2ed8c3b1871487210044adddc42ac77f95ab4376b4d21b41c7";
const DIRECT_PROVIDER_LOCAL_ABI: &str =
    "skiff-package-local-abi-v7:sha256:a303a83d48a2eaa49c34aff990c866a4a4c135ced8a19a43f6831efed25badee";

const INTERFACE_BASE_ID: &str = "example.com/interface-base";
const INTERFACE_FACADE_ID: &str = "example.com/interface-facade";

fn write_direct_provider(root: &TestDir, prefix: &str) {
    root.write(
        fixture_path(prefix, "package.yml"),
        format!("id: {DIRECT_PROVIDER_ID}\nversion: {DIRECT_PROVIDER_VERSION}\n"),
    );
    root.write(
        fixture_path(prefix, "api.yml"),
        r#"Handler: api.Handler
GenericHandler: api.GenericHandler
accept: api.accept
echo: api.echo
acceptNullable: api.acceptNullable
acceptArray: api.acceptArray
acceptRecord: api.acceptRecord
acceptGeneric: api.acceptGeneric
"#,
    );
    root.write(
        fixture_path(prefix, "api.skiff"),
        r#"
interface Handler {
  function handle(self: Self, input: string) -> string
}

interface GenericHandler<T> {
  function handle(self: Self, input: T) -> T
}

function accept(handler: any Handler) -> string {
  return "accepted"
}

function echo(handler: any Handler) -> any Handler {
  return handler
}

function acceptNullable(handler: any Handler?) -> string {
  return "accepted"
}

function acceptArray(handlers: Array<any Handler>) -> string {
  return "accepted"
}

function acceptRecord(bindings: {
  direct: any Handler,
  maybe: any Handler?,
  many: Array<any Handler>,
}) -> string {
  return "accepted"
}

function acceptGeneric(handler: any GenericHandler<string>) -> string {
  return "accepted"
}
"#,
    );
}

fn write_interface_base(root: &TestDir, prefix: &str) {
    root.write(
        fixture_path(prefix, "package.yml"),
        format!("id: {INTERFACE_BASE_ID}\nversion: 1.0.0\n"),
    );
    root.write(
        fixture_path(prefix, "api.yml"),
        "Handler: interfaces.Handler\nGenericHandler: interfaces.GenericHandler\n",
    );
    root.write(
        fixture_path(prefix, "interfaces.skiff"),
        r#"
interface Handler {
  function handle(self: Self, input: string) -> string
}

interface GenericHandler<T> {
  function handle(self: Self, input: T) -> T
}
"#,
    );
}

fn write_interface_facade(root: &TestDir, prefix: &str, interface_version: &str) {
    root.write(
        fixture_path(prefix, "package.yml"),
        format!(
            "id: {INTERFACE_FACADE_ID}\nversion: 1.0.0\npackages:\n  - id: {INTERFACE_BASE_ID}\n    version: {interface_version}\n    alias: iface\n"
        ),
    );
    root.write(
        fixture_path(prefix, "api.yml"),
        r#"accept: api.accept
echo: api.echo
acceptNested: api.acceptNested
acceptGeneric: api.acceptGeneric
"#,
    );
    root.write(
        fixture_path(prefix, "api.skiff"),
        r#"
import iface

function accept(handler: any iface.Handler) -> string {
  return "accepted"
}

function echo(handler: any iface.Handler) -> any iface.Handler {
  return handler
}

function acceptNested(handlers: Array<any iface.Handler?>) -> string {
  return "accepted"
}

function acceptGeneric(handler: any iface.GenericHandler<string>) -> string {
  return "accepted"
}
"#,
    );
}

fn write_negative_provider(root: &TestDir, prefix: &str, package_id: &str, full: bool) {
    root.write(
        fixture_path(prefix, "package.yml"),
        format!("id: {package_id}\nversion: 1.0.0\n"),
    );
    let api = if full {
        r#"Handler: api.Handler
OtherHandler: api.OtherHandler
GenericHandler: api.GenericHandler
accept: api.accept
acceptGeneric: api.acceptGeneric
"#
    } else {
        "Handler: api.Handler\n"
    };
    root.write(fixture_path(prefix, "api.yml"), api);
    let source = if full {
        r#"
interface Handler {
  function handle(self: Self, input: string) -> string
}

interface OtherHandler {
  function handle(self: Self, input: string) -> string
}

interface GenericHandler<T> {
  function handle(self: Self, input: T) -> T
}

function accept(handler: any Handler) -> string {
  return "accepted"
}

function acceptGeneric(handler: any GenericHandler<string>) -> string {
  return "accepted"
}
"#
    } else {
        r#"
interface Handler {
  function handle(self: Self, input: string) -> string
}
"#
    };
    root.write(fixture_path(prefix, "api.skiff"), source);
}

fn interface_type(
    package: PackageRefIr,
    symbol_path: &str,
    abi_expectation: Option<&str>,
    args: Vec<TypeRefIr>,
) -> ResolvedTypeRef {
    let identity = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package,
            symbol_path: symbol_path.to_string(),
            abi_expectation: abi_expectation.map(str::to_string),
        },
    };
    ResolvedTypeRef::with_text(
        TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(identity, args),
        },
        format!("any {symbol_path}"),
    )
}

fn provider_store_root(package_id: &str, version: &str) -> String {
    format!(
        ".skiff-packages/{}/{version}",
        package_id.replace('.', "~").replace('/', "~~")
    )
}

fn fixture_path(prefix: &str, file: &str) -> String {
    if prefix.is_empty() {
        file.to_string()
    } else {
        format!("{prefix}/{file}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_package_interface_identity_normalizes_dependency_and_package_id_owners() {
        let standalone = TestDir::new("skiff-compiler", "interface-provider-standalone");
        write_direct_provider(&standalone, "");
        let standalone = compile_package_project(standalone.path())
            .expect("the independent interface provider should compile");
        assert_eq!(
            standalone.package.artifact.package_build_id.as_str(),
            DIRECT_PROVIDER_BUILD_ID,
            "provider build identity must include the current File IR format"
        );
        assert_eq!(
            standalone
                .package
                .artifact
                .package_local_abi
                .local_abi_identity
                .as_str(),
            DIRECT_PROVIDER_LOCAL_ABI,
            "provider Local ABI must remain at its pre-fix value"
        );
        assert_eq!(
            standalone.package.published.identity, DIRECT_PROVIDER_BUILD_ID,
            "the publication receipt identity must match the provider build"
        );

        let consumer = TestDir::new("skiff-compiler", "interface-consumer-direct");
        consumer.write(
        "package.yml",
        format!(
            "id: example.com/interface-consumer\nversion: 1.0.0\npackages:\n  - id: {DIRECT_PROVIDER_ID}\n    version: {DIRECT_PROVIDER_VERSION}\n    alias: provider\n"
        ),
    );
        consumer.write("api.yml", "{}\n");
        consumer.write(
            "main.skiff",
            r#"
import provider

function direct(handler: any provider.Handler) -> string {
  return provider/accept(handler)
}

function roundTrip(handler: any provider.Handler) -> any provider.Handler {
  let echoed: any provider.Handler = provider/echo(handler)
  return echoed
}

function nullable(handler: any provider.Handler?) -> string {
  return provider/acceptNullable(handler)
}

function array(handlers: Array<any provider.Handler>) -> string {
  return provider/acceptArray(handlers)
}

function record(bindings: {
  direct: any provider.Handler,
  maybe: any provider.Handler?,
  many: Array<any provider.Handler>,
}) -> string {
  return provider/acceptRecord(bindings)
}

function generic(handler: any provider.GenericHandler<string>) -> string {
  return provider/acceptGeneric(handler)
}
"#,
        );
        write_direct_provider(
            &consumer,
            &provider_store_root(DIRECT_PROVIDER_ID, DIRECT_PROVIDER_VERSION),
        );

        let project = compile_package_project(consumer.path())
            .expect("equivalent dependency and package-id interface owners should compile");
        let provider = project
            .dependency(DIRECT_PROVIDER_ID, DIRECT_PROVIDER_VERSION)
            .expect("provider dependency");
        assert_eq!(
            provider.artifact, standalone.package.artifact,
            "consumer comparison must not alter provider artifact bytes"
        );
        assert_eq!(
            provider.artifact.package_build_id.as_str(),
            DIRECT_PROVIDER_BUILD_ID
        );
        assert_eq!(
            provider
                .artifact
                .package_local_abi
                .local_abi_identity
                .as_str(),
            DIRECT_PROVIDER_LOCAL_ABI
        );
        assert_eq!(
            package_artifact_ref(&provider.artifact).expect("canonical provider receipt"),
            package_artifact_ref(&standalone.package.artifact)
                .expect("standalone provider receipt"),
            "path-free provider receipt identity must remain invariant"
        );
    }

    #[test]
    fn transitive_dependency_owned_interface_identity_uses_the_same_exact_owner() {
        let consumer = TestDir::new("skiff-compiler", "interface-consumer-transitive");
        consumer.write(
        "package.yml",
        format!(
            "id: example.com/transitive-interface-consumer\nversion: 1.0.0\npackages:\n  - id: {INTERFACE_BASE_ID}\n    version: 1.0.0\n    alias: interfaces\n  - id: {INTERFACE_FACADE_ID}\n    version: 1.0.0\n    alias: gateway\n"
        ),
    );
        consumer.write("api.yml", "{}\n");
        consumer.write(
            "main.skiff",
            r#"
import gateway
import interfaces

function direct(handler: any interfaces.Handler) -> string {
  return gateway/accept(handler)
}

function roundTrip(handler: any interfaces.Handler) -> any interfaces.Handler {
  let echoed: any interfaces.Handler = gateway/echo(handler)
  return echoed
}

function nested(handlers: Array<any interfaces.Handler?>) -> string {
  return gateway/acceptNested(handlers)
}

function generic(handler: any interfaces.GenericHandler<string>) -> string {
  return gateway/acceptGeneric(handler)
}
"#,
        );
        write_interface_base(&consumer, &provider_store_root(INTERFACE_BASE_ID, "1.0.0"));
        write_interface_facade(
            &consumer,
            &provider_store_root(INTERFACE_FACADE_ID, "1.0.0"),
            "1.0.0",
        );

        let project = compile_package_project(consumer.path()).expect(
            "dependency-owned interface signatures should match the direct exact dependency",
        );
        let facade = project
            .dependency(INTERFACE_FACADE_ID, "1.0.0")
            .expect("facade dependency");
        let PackageLocalAbiSymbol::Callable { signature, .. } = facade
            .artifact
            .package_local_abi
            .public_symbols
            .get("accept")
            .expect("facade accept signature")
        else {
            panic!("facade accept must remain a callable")
        };
        assert!(matches!(
            &signature.parameters[0].ty,
            skiff_artifact_model::PackageTypeRef::AnyInterface {
                interface,
                arguments,
            } if arguments.is_empty()
                && matches!(
                    interface.as_ref(),
                    skiff_artifact_model::PackageTypeRef::PackageSchema {
                        package_id,
                        stable_schema_key,
                        ..
                    } if package_id == INTERFACE_BASE_ID && stable_schema_key == "Handler"
                )
        ));
    }

    #[test]
    fn canonical_interface_identity_keeps_package_symbol_abi_and_arguments_exact() {
        let provider = TestDir::new("skiff-compiler", "interface-provider-identity-matrix");
        write_direct_provider(&provider, "");
        let provider = compile_package_project(provider.path())
            .expect("identity matrix provider should compile")
            .package;
        let mut dependency = PackageDependency::id(DIRECT_PROVIDER_ID);
        dependency.alias = Some("provider".to_string());
        let model = TypeResolutionModel::build(
            &[],
            &BTreeMap::new(),
            &[dependency],
            None,
            Some(std::slice::from_ref(&provider.artifact)),
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type-resolution matrix model");

        let dependency_owned = interface_type(
            PackageRefIr::Dependency {
                dependency_ref: "provider".to_string(),
            },
            "Handler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![],
        );
        let package_owned = interface_type(
            PackageRefIr::PackageId {
                package_id: DIRECT_PROVIDER_ID.to_string(),
            },
            "Handler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![],
        );
        assert!(
            model.assignable(&dependency_owned, &package_owned)
                && model.assignable(&package_owned, &dependency_owned),
            "the selected exact dependency and package-id forms must canonicalize symmetrically"
        );

        let other_package_same_symbol_and_abi = interface_type(
            PackageRefIr::PackageId {
                package_id: "example.com/other-interface-provider".to_string(),
            },
            "Handler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![],
        );
        assert!(
            !model.assignable(&package_owned, &other_package_same_symbol_and_abi),
            "a forged matching ABI string must not erase package ownership"
        );
        let other_symbol_same_package_and_abi = interface_type(
            PackageRefIr::PackageId {
                package_id: DIRECT_PROVIDER_ID.to_string(),
            },
            "OtherHandler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![],
        );
        assert!(
            !model.assignable(&package_owned, &other_symbol_same_package_and_abi),
            "symbol identity must remain exact"
        );
        let other_abi_same_package_and_symbol = interface_type(
            PackageRefIr::PackageId {
                package_id: DIRECT_PROVIDER_ID.to_string(),
            },
            "Handler",
            Some("skiff-package-local-abi-v7:sha256:different"),
            vec![],
        );
        assert!(
            !model.assignable(&package_owned, &other_abi_same_package_and_symbol),
            "ABI expectation must remain exact"
        );

        let generic_string = interface_type(
            PackageRefIr::PackageId {
                package_id: DIRECT_PROVIDER_ID.to_string(),
            },
            "GenericHandler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![TypeRefIr::builtin("string")],
        );
        let generic_integer = interface_type(
            PackageRefIr::Dependency {
                dependency_ref: "provider".to_string(),
            },
            "GenericHandler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![TypeRefIr::builtin("integer")],
        );
        assert!(
            !model.assignable(&generic_string, &generic_integer),
            "canonical generic arguments must remain ordered and exact"
        );

        let unbound_dependency = interface_type(
            PackageRefIr::Dependency {
                dependency_ref: "missing".to_string(),
            },
            "Handler",
            Some(DIRECT_PROVIDER_LOCAL_ABI),
            vec![],
        );
        assert!(
            !model.assignable(&package_owned, &unbound_dependency),
            "an unbound dependency alias must remain fail closed"
        );
        let malformed = ResolvedTypeRef::with_text(
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: "{malformed".to_string(),
                    canonical_type_args: vec![],
                },
            },
            "malformed interface".to_string(),
        );
        assert!(
            !model.assignable(&package_owned, &malformed),
            "a malformed embedded identity must not match a valid identity"
        );
    }

    #[test]
    fn source_package_symbol_and_generic_argument_mismatches_remain_rejected() {
        let consumer = TestDir::new("skiff-compiler", "interface-negative-source-matrix");
        consumer.write(
            "package.yml",
            r#"id: example.com/interface-negative-consumer
version: 1.0.0
packages:
  - id: example.com/interface-negative-provider
    version: 1.0.0
    alias: provider
  - id: example.com/interface-other-provider
    version: 1.0.0
    alias: other
"#,
        );
        consumer.write("api.yml", "{}\n");
        consumer.write(
            "main.skiff",
            r#"
import other
import provider

function wrongPackage(handler: any other.Handler) -> string {
  return provider/accept(handler)
}

function wrongSymbol(handler: any provider.OtherHandler) -> string {
  return provider/accept(handler)
}

function wrongGeneric(handler: any provider.GenericHandler<integer>) -> string {
  return provider/acceptGeneric(handler)
}
"#,
        );
        write_negative_provider(
            &consumer,
            &provider_store_root("example.com/interface-negative-provider", "1.0.0"),
            "example.com/interface-negative-provider",
            true,
        );
        write_negative_provider(
            &consumer,
            &provider_store_root("example.com/interface-other-provider", "1.0.0"),
            "example.com/interface-other-provider",
            false,
        );

        let error = compile_package_project(consumer.path())
            .expect_err("different package, symbol, and generic arguments must fail")
            .to_string();
        for expected in [
            "found any other.Handler",
            "found any provider.OtherHandler",
            "found any provider.GenericHandler<integer>",
        ] {
            assert!(
                error.contains(expected),
                "missing negative identity diagnostic `{expected}`: {error}"
            );
        }
    }
}
