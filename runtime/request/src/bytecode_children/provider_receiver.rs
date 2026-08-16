//! Exact provider-operation receiver plan validation shared by child lanes.

use skiff_artifact_model::{ParamModeIr, ReceiverCallAbi};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, LinkedCallableSignature, LinkedOperationReceiver, LinkedValueTransferPlan,
    TypeIndex,
};
use skiff_runtime_scheduler::BytecodeSchedulerError;

#[derive(Debug)]
pub(crate) struct ProviderReceiverPlan {
    pub(crate) parameter_offset: usize,
    pub(crate) constant: Option<ConstantIndex>,
    pub(crate) receiver_type: Option<TypeIndex>,
    pub(crate) receiver_plan: Option<LinkedValueTransferPlan>,
}

pub(crate) fn provider_receiver_plan(
    signature: &LinkedCallableSignature,
    boundary_argument_count: usize,
    boundary_result_count: usize,
    receiver: Option<&LinkedOperationReceiver>,
) -> Result<ProviderReceiverPlan, BytecodeSchedulerError> {
    let parameter_count = signature.parameter_types().len();
    let has_receiver_parameter = parameter_count == boundary_argument_count.saturating_add(1);
    if (parameter_count != boundary_argument_count && !has_receiver_parameter)
        || signature.result_types().len() != boundary_result_count
    {
        return Err(BytecodeSchedulerError::Port(
            "provider signature and linked service boundary plan disagree".to_string(),
        ));
    }
    if !has_receiver_parameter {
        if receiver.is_some() {
            return Err(BytecodeSchedulerError::Port(
                "provider operation carries receiver facts without a receiver parameter"
                    .to_string(),
            ));
        }
        return Ok(ProviderReceiverPlan {
            parameter_offset: 0,
            constant: None,
            receiver_type: None,
            receiver_plan: None,
        });
    }
    let receiver = receiver.ok_or_else(|| {
        BytecodeSchedulerError::Port("provider operation receiver facts are missing".to_string())
    })?;
    if receiver.receiver_call_abi() != ReceiverCallAbi::ExplicitSelfFirst {
        return Err(BytecodeSchedulerError::Port(
            "provider operation receiver ABI is not explicit-self-first".to_string(),
        ));
    }
    if signature.parameter_modes().first() != Some(&ParamModeIr::Value) {
        return Err(BytecodeSchedulerError::Port(
            "provider operation receiver is not a value parameter".to_string(),
        ));
    }
    Ok(ProviderReceiverPlan {
        parameter_offset: 1,
        constant: Some(receiver.constant()),
        receiver_type: Some(signature.parameter_types()[0]),
        receiver_plan: Some(signature.parameter_plans()[0].clone()),
    })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{CallableEffectSummary, CallableMayEffects, ParamModeIr};
    use skiff_runtime_linked_bytecode::{
        LinkedCallableSignature, LinkedOperationReceiver, LinkedValueDropPlan,
        LinkedValueTransferPlan, TypeIndex,
    };

    use super::*;

    fn plan() -> LinkedValueTransferPlan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    }

    fn signature(parameter_count: usize, result_count: usize) -> LinkedCallableSignature {
        LinkedCallableSignature::new(
            (0..parameter_count)
                .map(|index| TypeIndex::new(index as u32))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            vec![ParamModeIr::Value; parameter_count].into_boxed_slice(),
            (0..parameter_count)
                .map(|_| plan())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            (0..result_count)
                .map(|index| TypeIndex::new(index as u32))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            (0..result_count)
                .map(|_| plan())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                },
            },
        )
        .expect("test signature is canonical")
    }

    #[test]
    fn receiverless_provider_plan_uses_zero_parameter_offset() {
        let plan = provider_receiver_plan(&signature(2, 1), 2, 1, None)
            .expect("receiverless provider plan must be valid");
        assert_eq!(plan.parameter_offset, 0);
        assert_eq!(plan.constant, None);
    }

    #[test]
    fn explicit_self_first_receiver_plan_is_exact() {
        let receiver =
            LinkedOperationReceiver::new(ConstantIndex::new(7), ReceiverCallAbi::ExplicitSelfFirst);
        let plan = provider_receiver_plan(&signature(3, 1), 2, 1, Some(&receiver))
            .expect("receiver provider plan must be valid");
        assert_eq!(plan.parameter_offset, 1);
        assert_eq!(plan.constant, Some(ConstantIndex::new(7)));
        assert_eq!(plan.receiver_type, Some(TypeIndex::new(0)));
    }

    #[test]
    fn missing_receiver_facts_fail_closed() {
        let error = provider_receiver_plan(&signature(3, 1), 2, 1, None)
            .expect_err("receiver parameter without facts must fail");
        assert!(error.to_string().contains("receiver facts are missing"));
    }

    #[test]
    fn receiver_facts_without_receiver_parameter_fail_closed() {
        let receiver =
            LinkedOperationReceiver::new(ConstantIndex::new(3), ReceiverCallAbi::ExplicitSelfFirst);
        let error = provider_receiver_plan(&signature(2, 1), 2, 1, Some(&receiver))
            .expect_err("receiver facts without a receiver parameter must fail");
        assert!(error
            .to_string()
            .contains("receiver facts without a receiver parameter"));
    }

    #[test]
    fn provider_plan_rejects_signature_drift() {
        assert!(provider_receiver_plan(&signature(2, 1), 2, 2, None).is_err());
        assert!(provider_receiver_plan(&signature(2, 1), 3, 1, None).is_err());
    }
}
