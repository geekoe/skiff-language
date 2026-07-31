use std::sync::Arc;

use skiff_runtime_linked_program::{
    anonymous_type_decl, ExecutableAddr, FileAddr, LinkOverlay, LinkedTypeDescriptor,
    PackageSymbolKey, ResolvedSymbol, RuntimeExecutionPackage, RuntimeTypeContext, TypeAddr,
    UnitAddr,
};
use skiff_runtime_model::service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity};

use crate::type_plan::test_runtime_package;

use super::{
    native_builtin_plan, native_named_union_error_owner, native_package_type_addr, ProgramTypeView,
    STD_PACKAGE_ID, WEBSOCKET_REQUEST_BINDING, WEBSOCKET_REQUEST_ERROR_TYPE,
};

fn package_addr(slot: usize, index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(slot),
        file: FileAddr::loaded_file(0),
        type_index: index,
    }
}

fn view<'a>(
    overlay: &'a LinkOverlay,
    types: &'a RuntimeTypeContext,
    packages: &'a [Arc<RuntimeExecutionPackage>],
) -> ProgramTypeView<'a> {
    ProgramTypeView::new(&[], packages, overlay, types)
}

#[test]
fn native_signature_package_type_uses_exact_package_id_and_public_path() {
    let left_addr = package_addr(0, 0);
    let right_addr = package_addr(1, 0);
    let mut overlay = LinkOverlay {
        package_slots_by_id: [
            ("example.left".to_string(), 0),
            ("example.right".to_string(), 1),
        ]
        .into_iter()
        .collect(),
        ..LinkOverlay::default()
    };
    overlay.symbols.insert_package(
        PackageSymbolKey::new(0, "api.Options"),
        ResolvedSymbol::Type {
            addr: left_addr.clone(),
        },
    );
    overlay.symbols.insert_package(
        PackageSymbolKey::new(1, "api.Options"),
        ResolvedSymbol::Type {
            addr: right_addr.clone(),
        },
    );
    let mut types = RuntimeTypeContext::default();
    types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "api.Options"), left_addr.clone());
    types
        .exported_types
        .insert_package(PackageSymbolKey::new(1, "api.Options"), right_addr.clone());
    let packages = Vec::new();
    let program = view(&overlay, &types, &packages);

    assert_eq!(
        native_package_type_addr(program, "example.left", "api.Options").unwrap(),
        left_addr
    );
    assert_eq!(
        native_package_type_addr(program, "example.right", "api.Options").unwrap(),
        right_addr
    );
}

#[test]
fn native_signature_package_type_fails_closed_for_missing_or_wrong_kind_facts() {
    let addr = package_addr(0, 0);
    let mut overlay = LinkOverlay {
        package_slots_by_id: [("example.pkg".to_string(), 0)].into_iter().collect(),
        ..LinkOverlay::default()
    };
    overlay.symbols.insert_package(
        PackageSymbolKey::new(0, "api.NotAType"),
        ResolvedSymbol::Executable {
            addr: ExecutableAddr::package(0, 0, 0),
        },
    );
    let mut types = RuntimeTypeContext::default();
    types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "api.NotAType"), addr);
    let packages = Vec::new();
    let program = view(&overlay, &types, &packages);

    for (package_id, public_path, expected) in [
        ("missing.pkg", "api.NotAType", "unknown package"),
        ("example.pkg", "api.Missing", "is not a public type"),
        ("example.pkg", "api.NotAType", "does not name a type"),
    ] {
        let error = native_package_type_addr(program, package_id, public_path).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn package_public_path_cannot_masquerade_as_builtin() {
    let error = native_builtin_plan("std.file.CreateOptions").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown builtin type std.file.CreateOptions"),
        "{error}"
    );
}

fn std_owner_program(
    addr: TypeAddr,
    symbol: Option<ResolvedSymbol>,
    descriptor: LinkedTypeDescriptor,
) -> (
    LinkOverlay,
    RuntimeTypeContext,
    Vec<Arc<RuntimeExecutionPackage>>,
) {
    let mut overlay = LinkOverlay {
        package_slots_by_id: [(STD_PACKAGE_ID.to_string(), 0)].into_iter().collect(),
        ..LinkOverlay::default()
    };
    if let Some(symbol) = symbol {
        overlay.symbols.insert_package(
            PackageSymbolKey::new(0, WEBSOCKET_REQUEST_ERROR_TYPE),
            symbol,
        );
    }
    let mut types = RuntimeTypeContext::default();
    types.exported_types.insert_package(
        PackageSymbolKey::new(0, WEBSOCKET_REQUEST_ERROR_TYPE),
        addr.clone(),
    );
    types.descriptors.insert(
        addr,
        anonymous_type_decl("WebSocketRequestError", descriptor),
    );
    let packages = vec![test_runtime_package(0, STD_PACKAGE_ID, Vec::new())];
    (overlay, types, packages)
}

#[test]
fn websocket_request_owner_is_exact_current_linked_std_union() {
    let addr = package_addr(0, 7);
    let (overlay, types, packages) = std_owner_program(
        addr.clone(),
        Some(ResolvedSymbol::Type { addr: addr.clone() }),
        LinkedTypeDescriptor::Union {
            branches: Vec::new(),
        },
    );
    let program = view(&overlay, &types, &packages);

    assert_eq!(
        native_named_union_error_owner(WEBSOCKET_REQUEST_BINDING, program).unwrap(),
        Some(NamedUnionOwnerIdentity::LocalExecution(
            LocalExecutionTypeIdentity {
                addr,
                type_arguments: Vec::new(),
            }
        ))
    );
    assert_eq!(
        native_named_union_error_owner("std.websocket.sendTextToConnection", program).unwrap(),
        None
    );
}

#[test]
fn websocket_request_owner_fails_closed_before_dispatch_for_bad_link_facts() {
    let exact_addr = package_addr(0, 7);
    let other_addr = package_addr(0, 8);
    for (symbol, descriptor, expected) in [
        (
            None,
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
            "missing from the executable symbol overlay",
        ),
        (
            Some(ResolvedSymbol::Executable {
                addr: ExecutableAddr::package(0, 0, 0),
            }),
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
            "wrong symbol kind executable",
        ),
        (
            Some(ResolvedSymbol::Type {
                addr: other_addr.clone(),
            }),
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
            "ambiguous across type addresses",
        ),
        (
            Some(ResolvedSymbol::Type {
                addr: exact_addr.clone(),
            }),
            LinkedTypeDescriptor::Record {
                fields: Default::default(),
            },
            "must be an exact non-generic named union",
        ),
    ] {
        let (overlay, types, packages) = std_owner_program(exact_addr.clone(), symbol, descriptor);
        let program = view(&overlay, &types, &packages);
        let error = native_named_union_error_owner(WEBSOCKET_REQUEST_BINDING, program).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn websocket_request_owner_is_never_reused_across_linked_programs() {
    let left_addr = package_addr(0, 7);
    let right_addr = package_addr(0, 9);
    let (left_overlay, left_types, left_packages) = std_owner_program(
        left_addr.clone(),
        Some(ResolvedSymbol::Type {
            addr: left_addr.clone(),
        }),
        LinkedTypeDescriptor::Union {
            branches: Vec::new(),
        },
    );
    let (right_overlay, right_types, right_packages) = std_owner_program(
        right_addr.clone(),
        Some(ResolvedSymbol::Type {
            addr: right_addr.clone(),
        }),
        LinkedTypeDescriptor::Union {
            branches: Vec::new(),
        },
    );
    let left = native_named_union_error_owner(
        WEBSOCKET_REQUEST_BINDING,
        view(&left_overlay, &left_types, &left_packages),
    )
    .unwrap();
    let right = native_named_union_error_owner(
        WEBSOCKET_REQUEST_BINDING,
        view(&right_overlay, &right_types, &right_packages),
    )
    .unwrap();

    assert_ne!(left, right);
}
