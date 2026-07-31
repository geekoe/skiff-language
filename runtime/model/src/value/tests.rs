use super::*;
use crate::{
    addr::{FileAddr, UnitAddr},
    service_error::{
        CatchIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, NominalTypeIdentity,
    },
};

fn local_nominal(type_index: usize) -> LocalExecutionTypeIdentity {
    LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::loaded_file(0),
            type_index,
        },
        type_arguments: Vec::new(),
    }
}

#[test]
fn runtime_value_carrier_distinguishes_equal_shapes_by_nominal_identity() {
    let payload = RuntimeValue::from("same");
    let first = RuntimeValueCarrier::identified(
        payload.clone(),
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local_nominal(1))),
    );
    let second = RuntimeValueCarrier::identified(
        payload,
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local_nominal(2))),
    );

    assert_ne!(first, second);
}

#[test]
fn runtime_value_carrier_keeps_representation_outer_identity() {
    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local_nominal(3)));
    let representation =
        RuntimeValueCarrier::identified(RuntimeValue::from("primitive payload"), identity.clone());

    assert_eq!(representation.catch_identity(), Some(&identity));
    assert_eq!(
        representation.value(),
        &RuntimeValue::from("primitive payload")
    );
}

#[test]
fn named_union_branch_identity_includes_enclosing_union_context() {
    let branch = NamedUnionBranchIdentity::SyntheticDiscriminator {
        discriminator_field: "kind".to_string(),
        discriminator_value: "retryable".to_string(),
    };
    let first = RuntimeValueCarrier::identified(
        RuntimeValue::Null,
        CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(local_nominal(10)),
            branch: branch.clone(),
        },
    );
    let second = RuntimeValueCarrier::identified(
        RuntimeValue::Null,
        CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(local_nominal(11)),
            branch,
        },
    );

    assert_ne!(first, second);
}

#[test]
fn carrier_clone_preserves_identity_for_slot_container_and_call_handoffs() {
    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local_nominal(12)));
    let assigned = RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
    let container = [assigned.clone()];
    let call_argument = container[0].clone();

    assert_eq!(call_argument.catch_identity(), Some(&identity));
}

#[test]
fn interface_value_local_carrier_keeps_method_table_and_payload() {
    let interface = "reader-interface<string>".to_string();
    let table = InterfaceMethodTable::new(
        "table:reader:string".to_string(),
        "reader-interface".to_string(),
        vec![InterfaceMethodSlot::new(
            0,
            "method:reader:read".to_string(),
            InterfaceMethodTarget::LocalExecutable {
                executable: ExecutableAddr::service(0, 7),
                receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
            },
        )],
    );
    let value = InterfaceValue::new(
        interface.clone(),
        InterfaceCarrier::Local {
            concrete_type: "root.ReaderImpl".to_string(),
            method_table: table,
            payload: RuntimeValue::Null,
        },
    );

    let InterfaceCarrier::Local {
        method_table,
        payload,
        ..
    } = value.carrier()
    else {
        panic!("expected local interface carrier");
    };
    assert_eq!(value.interface(), interface);
    assert_eq!(
        value.diagnostic_label(),
        "any interface reader-interface<string> (local)"
    );
    assert_eq!(method_table.slots()[0].slot(), 0);
    assert_eq!(payload, &RuntimeValue::Null);
}

#[test]
fn callback_capability_carrier_is_opaque_and_labeled() {
    let carrier = CallbackCapabilityCarrier::new(
        "runtime-a",
        "activation-a",
        17,
        "contract:reader",
        "capability-1",
    );
    let value = InterfaceValue::new(
        "contract:reader".to_string(),
        InterfaceCarrier::CallbackCapability(carrier.clone()),
    );

    assert_eq!(carrier.owner_runtime_replica_id(), "runtime-a");
    assert_eq!(carrier.owner_activation_id(), "activation-a");
    assert_eq!(carrier.request_generation(), 17);
    assert_eq!(carrier.interface_or_adapter_contract(), "contract:reader");
    assert_eq!(carrier.opaque_capability_id(), "capability-1");
    assert_eq!(
        value.diagnostic_label(),
        "any interface contract:reader (callback capability)"
    );
}
