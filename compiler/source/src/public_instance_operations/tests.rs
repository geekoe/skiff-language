use skiff_artifact_identity::{canonical_interface_method_abi_id, interface_instantiation_ref};
use skiff_artifact_model::{InterfaceInstantiationRef, ServiceSymbolRef, TypeRefIr};

use super::*;

fn interface() -> InterfaceInstantiationRef {
    interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "api".to_string(),
                symbol: "PublicApi".to_string(),
            },
        },
        Vec::new(),
    )
}

#[test]
fn row_validation_rejects_noncanonical_public_root() {
    let error = SourcePublicInstanceInterfaceOperations::try_new(
        "api..handler".to_string(),
        interface(),
        Vec::new(),
    )
    .expect_err("public root must be a dotted identifier path");

    assert_eq!(
        error,
        SourcePublicInstanceOperationFactsError::InvalidPublicRoot {
            public_root: "api..handler".to_string()
        }
    );
}

#[test]
fn row_validation_binds_operation_key_to_its_public_root() {
    let interface = interface();
    let error = SourcePublicInstanceInterfaceOperations::try_new(
        "handler".to_string(),
        interface.clone(),
        vec![SourcePublicInstanceOperationSlot {
            method_abi_id: canonical_interface_method_abi_id(&interface, "run"),
            operation_stable_key: "other.run".to_string(),
        }],
    )
    .expect_err("operation key must be rooted under the selected public instance");

    assert_eq!(
        error,
        SourcePublicInstanceOperationFactsError::InvalidOperationStableKey {
            public_root: "handler".to_string(),
            slot: 0,
            operation_stable_key: "other.run".to_string(),
        }
    );
}
