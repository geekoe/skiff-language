use serde_json::json;
use skiff_artifact_model::{AssemblyActivationControl, AssemblyIdentity, RuntimeAssemblyRef};
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    protocol::encode_binary_frame,
};

fn assembly_ref(byte: char) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            byte.to_string().repeat(64)
        )),
    }
}

#[test]
fn two_replicas_register_one_exact_assembly_identity_independently() {
    let assembly = assembly_ref('a');
    let first = AssemblyActivationControl::Register {
        environment: "prod".to_string(),
        generation: 42,
        assembly: assembly.clone(),
        replica_id: "runtime-a".to_string(),
    };
    let second = AssemblyActivationControl::Register {
        environment: "prod".to_string(),
        generation: 42,
        assembly,
        replica_id: "runtime-b".to_string(),
    };

    let first = decode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RuntimeToRouter,
        &encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            &first,
        )
        .unwrap(),
    )
    .unwrap();
    let second = decode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RuntimeToRouter,
        &encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            &second,
        )
        .unwrap(),
    )
    .unwrap();

    assert_ne!(first, second, "replica identity is not assembly identity");
    let (first_assembly, first_replica) = register_parts(first);
    let (second_assembly, second_replica) = register_parts(second);
    assert_eq!(first_assembly, second_assembly);
    assert_eq!(first_replica, "runtime-a");
    assert_eq!(second_replica, "runtime-b");
}

#[test]
fn activation_control_rejects_payload_and_legacy_service_fields() {
    let prepare = AssemblyActivationControl::Prepare {
        environment: "prod".to_string(),
        activation_id: "activation-42".to_string(),
        expected_generation: 41,
        candidate_generation: 42,
        assembly: assembly_ref('b'),
        replica_id: "runtime-a".to_string(),
        service_db: None,
    };
    let payload_frame = encode_binary_frame(&prepare, b"legacy payload").unwrap();
    assert!(decode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &payload_frame
    )
    .is_err());

    let legacy = json!({
        "type": "prepare",
        "environment": "prod",
        "activationId": "activation-42",
        "expectedGeneration": 41,
        "candidateGeneration": 42,
        "assembly": {
            "assemblyIdentity": format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                "b".repeat(64)
            )
        },
        "replicaId": "runtime-a",
        "artifactRoots": ["/legacy"],
        "serviceConfig": []
    });
    let legacy_frame = encode_binary_frame(&legacy, &[]).unwrap();
    assert!(decode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &legacy_frame
    )
    .is_err());
}

fn register_parts(control: AssemblyActivationControl) -> (RuntimeAssemblyRef, String) {
    match control {
        AssemblyActivationControl::Register {
            assembly,
            replica_id,
            ..
        } => (assembly, replica_id),
        other => panic!("expected register, got {other:?}"),
    }
}
