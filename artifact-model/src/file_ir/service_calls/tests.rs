use std::collections::BTreeMap;

use serde_json::json;

use crate::{
    CallIr, ContractOperationId, ExprIr, ExternalRefTable, FileIrUnit, InstructionSourceSite,
    ServiceProtocolIdentity, SyntheticInstructionSiteReason,
};

use super::*;

#[test]
fn external_ref_table_and_service_call_target_have_strict_wire_shapes() {
    let table = ExternalRefTable::default();
    assert_eq!(
        serde_json::to_value(&table).unwrap(),
        json!({ "serviceCallRefs": [] })
    );
    assert!(serde_json::from_value::<ExternalRefTable>(json!({})).is_err());

    let target = CallTargetIr::ServiceCall {
        service_call_ref_index: ServiceCallRefIndex::new(3),
    };
    let wire = json!({ "kind": "serviceCall", "serviceCallRefIndex": 3 });
    assert_eq!(serde_json::to_value(&target).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<CallTargetIr>(wire).unwrap(),
        target
    );
    for invalid in [
        json!({ "kind": "serviceCall" }),
        json!({
            "kind": "serviceCall",
            "serviceCallRefIndex": 3,
            "serviceCallRef": service_call_ref(0, "echo", "protocol")
        }),
        json!({
            "kind": "serviceCall",
            "serviceCallRefIndex": 3,
            "serviceDependencySymbol": "legacy"
        }),
    ] {
        assert!(serde_json::from_value::<CallTargetIr>(invalid).is_err());
    }
}

#[test]
fn validator_accepts_typed_sites_and_exposes_validated_refs() {
    let mut unit = FileIrUnit::empty("api", "source");
    unit.external_refs.service_call_refs = vec![service_call_ref(2, "echo", "protocol")];
    push_service_call(&mut unit, 0);

    let sites = file_ir_service_call_sites(&unit).collect::<Vec<_>>();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].service_call_ref_index.index(), 0);
    assert_eq!(
        unit.external_refs
            .service_call_ref(sites[0].service_call_ref_index),
        unit.external_refs.service_call_refs.first()
    );
    assert_eq!(
        validated_file_ir_service_call_refs(&unit).unwrap(),
        unit.external_refs.service_call_refs.as_slice()
    );
}

#[test]
fn validator_rejects_out_of_range_duplicate_and_orphan_refs() {
    let mut out_of_range = FileIrUnit::empty("api", "source");
    push_service_call(&mut out_of_range, 0);
    assert!(matches!(
        validate_file_ir_service_calls(&out_of_range),
        Err(FileIrServiceCallValidationError::IndexOutOfRange { .. })
    ));

    let mut duplicate = FileIrUnit::empty("api", "source");
    duplicate.external_refs.service_call_refs = vec![
        service_call_ref(2, "echo", "protocol"),
        service_call_ref(2, "echo", "protocol"),
    ];
    push_service_call(&mut duplicate, 0);
    push_service_call(&mut duplicate, 1);
    assert!(matches!(
        validate_file_ir_service_calls(&duplicate),
        Err(FileIrServiceCallValidationError::DuplicateRef { .. })
    ));

    let mut orphan = FileIrUnit::empty("api", "source");
    orphan.external_refs.service_call_refs = vec![service_call_ref(2, "echo", "protocol")];
    assert_eq!(
        validate_file_ir_service_calls(&orphan),
        Err(FileIrServiceCallValidationError::OrphanRef { index: 0 })
    );
}

fn push_service_call(unit: &mut FileIrUnit, index: u32) {
    if unit.constants.is_empty() {
        unit.constants.push(crate::ConstIr {
            name: "calls".to_string(),
            ty: crate::TypeRefIr::builtin("void"),
            body: crate::ExecutableBody::default(),
            source_span: None,
        });
    }
    unit.constants[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: ServiceCallRefIndex::new(index),
            },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
}

fn service_call_ref(slot: u32, operation: &str, protocol: &str) -> ServiceCallRef {
    ServiceCallRef {
        service_requirement_slot: slot,
        contract_operation_id: ContractOperationId::new(operation),
        expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
    }
}
