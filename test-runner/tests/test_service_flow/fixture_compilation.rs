use std::{collections::BTreeSet, path::Path};

use skiff_artifact_model::{
    GatewayEntryKey, GatewayProtocolSurface, IngressProtocol, PackageLocalAbiSymbol,
    ServiceAuthoringKind,
};
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test, canonical_std_seed::seed_canonical_std,
    test_service_fixture::assemble_test_service_fixture,
};

use super::{platform_sources, TestRoot};

const CHECKED_IN_TEST_SERVICE_FIXTURES: [&str; 4] = [
    "package-service-websocket-smoke",
    "package-service-websocket-generation-a",
    "package-service-websocket-generation-b",
    "package-service-i02-spawn-submit",
];

#[test]
fn checked_in_test_services_compile_and_assemble_through_the_ordinary_service_flow() {
    let root = TestRoot::new("checked-in-fixture-compilation");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed canonical std");

    for fixture_name in CHECKED_IN_TEST_SERVICE_FIXTURES {
        let fixture_root = repository_root()
            .join("test-runner")
            .join("fixtures")
            .join(fixture_name);
        let project =
            compile_package_project_for_test(&platform_sources(), &fixture_root, &artifacts)
                .unwrap_or_else(|error| panic!("{fixture_name} must compile: {error}"));
        let profile = project
            .test_service_profile
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture_name} must be an ordinary kind:test service"));
        assert_eq!(
            profile.service_root.service.kind,
            ServiceAuthoringKind::Test,
            "{fixture_name}"
        );

        let implementation_symbols = &project.package.artifact.package_local_abi;
        let private_http_wrapper = implementation_symbols
            .implementation_symbols
            .get("main.__skiffHttpProbe")
            .unwrap_or_else(|| {
                panic!("{fixture_name} omitted its private HTTP wrapper implementation")
            });
        let PackageLocalAbiSymbol::Callable {
            callable_id: private_http_wrapper_id,
            ..
        } = private_http_wrapper
        else {
            panic!("{fixture_name} private HTTP wrapper must be callable")
        };
        assert!(
            !implementation_symbols
                .public_symbols
                .contains_key("main.__skiffHttpProbe"),
            "{fixture_name} leaked its private HTTP wrapper into publicSymbols"
        );
        assert!(
            !implementation_symbols
                .public_symbols
                .values()
                .any(|symbol| {
                    matches!(
                        symbol,
                        PackageLocalAbiSymbol::Callable { callable_id, .. }
                            if callable_id == private_http_wrapper_id
                    )
                }),
            "{fixture_name} exposed its private HTTP wrapper under a public alias"
        );

        let service_api = project
            .service_api
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture_name} omitted its ordinary service projection"));
        assert!(
            service_api.contract.operations.is_empty(),
            "{fixture_name} HTTP and WebSocket entries must not become service-call operations"
        );

        let cases = discover_test_service_cases(&fixture_root, &fixture_root, false)
            .unwrap_or_else(|error| panic!("{fixture_name} discovery failed: {error}"));
        assert_eq!(cases.len(), 1, "{fixture_name}");
        let assembled = assemble_test_service_fixture(&project, &cases, Default::default())
            .unwrap_or_else(|error| panic!("{fixture_name} assembly failed: {error}"));
        assert_eq!(assembled.cases.len(), 1, "{fixture_name}");
        let case = &assembled.cases[0];
        let [contract] = case.records.contracts.as_slice() else {
            panic!("{fixture_name} must assemble one specialized ordinary contract")
        };
        let [deployment] = case.records.deployments.as_slice() else {
            panic!("{fixture_name} must assemble one ordinary deployment")
        };

        assert_eq!(deployment.contract, case.contract, "{fixture_name}");
        assert_eq!(
            contract.service_id, case.contract.service_id,
            "{fixture_name}"
        );
        assert!(
            contract.service_id.starts_with("test.skiff/p-")
                && contract.service_id.ends_with("/case-0"),
            "{fixture_name} must specialize the authored service to a case service id"
        );
        assert_ne!(
            contract.service_id, profile.service_id,
            "{fixture_name} must keep authored and execution identities distinct"
        );
        assert_eq!(
            contract.contract_version, service_api.contract.contract_version,
            "{fixture_name}"
        );
        assert_eq!(
            contract.operations, service_api.contract.operations,
            "{fixture_name} specialization must preserve service operations"
        );
        let bound_operations = deployment
            .operation_bindings
            .iter()
            .map(|binding| binding.contract_operation_id.clone())
            .collect::<BTreeSet<_>>();
        let contract_operations = contract.operations.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            bound_operations, contract_operations,
            "{fixture_name} operation bindings must cover exactly the specialized contract"
        );
        assert!(
            deployment.operation_bindings.is_empty(),
            "{fixture_name} has no serviceCalls and must not bind its external gateways as operations"
        );

        let probe_key = GatewayEntryKey::parse("probe").expect("canonical probe key");
        let probe = deployment
            .gateway_entries
            .get(&probe_key)
            .unwrap_or_else(|| panic!("{fixture_name} omitted the HTTP probe projection"));
        assert_eq!(
            probe.handler.as_ref(),
            Some(private_http_wrapper_id),
            "{fixture_name} HTTP projection must target the private implementation callable"
        );
        assert!(
            matches!(
                &probe.protocol_surface.protocol,
                GatewayProtocolSurface::Http(_)
            ),
            "{fixture_name} probe must retain an HTTP protocol surface"
        );
        assert!(
            deployment.ingress.iter().any(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == "/probe"
                    && binding.gateway_entry_key == probe_key
            }),
            "{fixture_name} omitted its external HTTP projection"
        );

        let websocket_ingress = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::WebSocket
                    && binding.selector.method.is_none()
                    && binding.selector.path == "/socket"
            })
            .unwrap_or_else(|| panic!("{fixture_name} omitted its external WebSocket projection"));
        let websocket = deployment
            .gateway_entries
            .get(&websocket_ingress.gateway_entry_key)
            .unwrap_or_else(|| panic!("{fixture_name} WebSocket ingress has no gateway entry"));
        assert!(
            matches!(
                &websocket.protocol_surface.protocol,
                GatewayProtocolSurface::WebSocketConnect(_)
            ),
            "{fixture_name} must retain the WebSocket connect protocol surface"
        );
    }
}

#[test]
fn std_kind_test_service_uses_one_exact_compiler_owned_std_closure() {
    let root = TestRoot::new("compiler-owned-std-closure");
    let artifacts = root.path().join("artifacts");
    seed_canonical_std(&platform_sources(), &artifacts).expect("seed canonical std");
    let fixture_root = repository_root().join("test-services").join("std");

    let project = compile_package_project_for_test(&platform_sources(), &fixture_root, &artifacts)
        .expect("compile ordinary std kind:test service");
    assert_eq!(
        project
            .test_service_profile
            .as_ref()
            .expect("std tests must be a kind:test service")
            .service_root
            .service
            .kind,
        ServiceAuthoringKind::Test
    );
    let [requirement] = project.package.artifact.package_requirements.as_slice() else {
        panic!("std test sources must select exactly one compiler-owned package requirement")
    };
    let [std] = project.dependency_packages.as_slice() else {
        panic!("std test service closure must contain exactly one canonical dependency")
    };
    assert_eq!(requirement.alias, "std");
    assert_eq!(requirement.package_id, "skiff.run/std");
    assert_eq!(requirement.package_id, std.package_id);
    assert_eq!(requirement.exact_version, std.package_version);
    assert_eq!(
        requirement.expected_local_abi,
        std.package_local_abi.local_abi_identity
    );
    assert!(
        requirement.expected_package_build.is_none(),
        "compiler-owned std must remain a public-ABI dependency, not a private top-level binding"
    );
}

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner must live directly below the repository root")
}
