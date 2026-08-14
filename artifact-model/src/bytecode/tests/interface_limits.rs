//! Whole-schema resource bounds for local and remote interface method rows.

use crate::bytecode::dto::{
    BytecodeRelocation, LocalInterfaceMethod, LocalInterfaceRef, ParameterSlotDecl,
    RemoteInterfaceMethod, RemoteInterfaceRef,
};
use crate::types::{FunctionTypeParamIr, TypeRefIr};

use super::*;

fn nested_type(depth: usize) -> TypeRefIr {
    let mut ty = string_type();
    for _ in 1..depth {
        ty = TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![ty],
        };
    }
    ty
}

fn signature(
    parameter_count: usize,
    parameter_type: TypeRefIr,
    return_type: TypeRefIr,
) -> crate::InterfaceMethodSlotSignatureIr {
    crate::InterfaceMethodSlotSignatureIr {
        params: (0..parameter_count)
            .map(|index| FunctionTypeParamIr {
                name: format!("parameter{index}"),
                ty: parameter_type.clone(),
            })
            .collect(),
        return_type,
    }
}

fn bind_helper_receiver(artifact: &mut BytecodeArtifact) {
    let helper = artifact
        .image
        .functions
        .get_mut("module::helper")
        .expect("helper fixture");
    helper.self_type_ref = Some(0);
    helper.frame_layout.parameter_slots = vec![ParameterSlotDecl {
        slot: 0,
        mode: crate::ParamModeIr::Value,
        plan: snapshot_share(),
        dense_record_shape_ref: None,
    }];

    let main = artifact
        .image
        .functions
        .get_mut("module::main")
        .expect("main fixture");
    let BytecodeRelocation::LocalExecutableRef { specialization, .. } = &mut main.relocations[0]
    else {
        unreachable!()
    };
    specialization.concrete_receiver = Some(string_type());
}

fn artifact_with_local_method(
    slot: u32,
    signature: crate::InterfaceMethodSlotSignatureIr,
) -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    bind_helper_receiver(&mut artifact);
    artifact
        .image
        .functions
        .get_mut("module::main")
        .expect("main fixture")
        .relocations
        .push(BytecodeRelocation::LocalInterfaceRef {
            interface: LocalInterfaceRef {
                interface: crate::InterfaceInstantiationRef {
                    interface_abi_id: "interface:limit:local".to_string(),
                    canonical_type_args: Vec::new(),
                },
                concrete_type: string_type(),
                methods: vec![LocalInterfaceMethod {
                    slot,
                    method_name: "read".to_string(),
                    method_abi_id: "method:limit:local:read".to_string(),
                    signature,
                    function_key: "module::helper".to_string(),
                    receiver_call_abi: crate::ReceiverCallAbi::ExplicitSelfFirst,
                }],
            },
        });
    artifact
}

fn artifact_with_remote_method(
    slot: u32,
    signature: crate::InterfaceMethodSlotSignatureIr,
) -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .functions
        .get_mut("module::main")
        .expect("main fixture")
        .relocations
        .push(BytecodeRelocation::RemoteInterfaceRef {
            interface: RemoteInterfaceRef {
                service_requirement_slot: 0,
                public_instance_key: "readers/default".to_string(),
                interface: crate::InterfaceInstantiationRef {
                    interface_abi_id: "interface:limit:remote".to_string(),
                    canonical_type_args: Vec::new(),
                },
                methods: vec![RemoteInterfaceMethod {
                    slot,
                    method_abi_id: "method:limit:remote:read".to_string(),
                    signature,
                    contract_operation_id: crate::ContractOperationId::new(
                        "operation:limit:remote:read",
                    ),
                }],
                callee_protocol_identity: crate::ServiceProtocolIdentity::new(
                    "protocol:limit:remote",
                ),
            },
        });
    artifact
}

fn assert_limit_error(artifact: &BytecodeArtifact, limit: &str, location: &str) {
    let error = assert_rejected(artifact);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains(limit), "{error}");
    assert!(error.to_string().contains(location), "{error}");
}

#[test]
fn local_interface_method_signature_and_slot_use_production_limits() {
    assert_validates(&artifact_with_local_method(
        255,
        signature(256, nested_type(64), nested_type(64)),
    ));

    assert_limit_error(
        &artifact_with_local_method(256, signature(0, string_type(), string_type())),
        "MAX_ARITY",
        "interface.methods[0].slot",
    );
    assert_limit_error(
        &artifact_with_local_method(0, signature(257, string_type(), string_type())),
        "MAX_ARITY",
        "interface.methods[0].signature.params",
    );
    assert_limit_error(
        &artifact_with_local_method(0, signature(1, nested_type(65), string_type())),
        "MAX_NESTING_DEPTH",
        "interface.methods[0].signature.params[0].ty",
    );
    assert_limit_error(
        &artifact_with_local_method(0, signature(0, string_type(), nested_type(65))),
        "MAX_NESTING_DEPTH",
        "interface.methods[0].signature.returnType",
    );
}

#[test]
fn remote_interface_method_signature_and_slot_use_production_limits() {
    assert_validates(&artifact_with_remote_method(
        255,
        signature(256, nested_type(64), nested_type(64)),
    ));

    assert_limit_error(
        &artifact_with_remote_method(256, signature(0, string_type(), string_type())),
        "MAX_ARITY",
        "interface.methods[0].slot",
    );
    assert_limit_error(
        &artifact_with_remote_method(0, signature(257, string_type(), string_type())),
        "MAX_ARITY",
        "interface.methods[0].signature.params",
    );
    assert_limit_error(
        &artifact_with_remote_method(0, signature(1, nested_type(65), string_type())),
        "MAX_NESTING_DEPTH",
        "interface.methods[0].signature.params[0].ty",
    );
    assert_limit_error(
        &artifact_with_remote_method(0, signature(0, string_type(), nested_type(65))),
        "MAX_NESTING_DEPTH",
        "interface.methods[0].signature.returnType",
    );
}
