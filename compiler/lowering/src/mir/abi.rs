//! Self-contained receiver and direct-call ABI facts.
//!
//! File IR keeps value arguments compact, while inout places carry explicit
//! parameter ordinals. MIR merges both channels into one dense parameter-order
//! table using an exact callee mode vector supplied during construction.

use std::collections::BTreeMap;

use skiff_artifact_model::{CallIr, CallTargetIr, ExprRefIr, ReceiverCallAbi, TypeRefIr};

use super::{MirCallWritableFacts, MirInOutLoan, MirParamMode};

/// Unified declaration-side receiver fact. Both implicit `SelfValue` and an
/// explicit leading `self` use ABI parameter ordinal zero.
#[derive(Debug, Clone, PartialEq)]
pub struct MirReceiverFacts {
    pub ty: TypeRefIr,
    pub slot: u32,
    pub parameter_ordinal: u32,
    pub call_abi: ReceiverCallAbi,
}

/// One direct-call argument at its vector-position parameter ordinal.
#[derive(Debug, Clone, PartialEq)]
pub enum MirCallArgument {
    Value { value: ExprRefIr },
    InOut { loan: MirInOutLoan },
}

/// Exact direct-call ABI copied into MIR. `arguments` and `parameter_modes`
/// are dense and aligned; a receiver, when present, is the Value argument at
/// ordinal zero.
#[derive(Debug, Clone, PartialEq)]
pub struct MirDirectCallFacts {
    pub concrete_receiver: Option<TypeRefIr>,
    pub receiver_call_abi: Option<ReceiverCallAbi>,
    pub parameter_modes: Vec<MirParamMode>,
    pub arguments: Vec<MirCallArgument>,
}

impl MirDirectCallFacts {
    pub fn argument(&self, parameter_ordinal: u32) -> Option<&MirCallArgument> {
        self.arguments.get(parameter_ordinal as usize)
    }
}

pub(super) fn is_direct_target(target: &CallTargetIr) -> bool {
    matches!(
        target,
        CallTargetIr::LocalExecutable { .. }
            | CallTargetIr::PublicationExecutable { .. }
            | CallTargetIr::PackageCallable { .. }
    )
}

pub(super) fn direct_call_facts(
    call: &CallIr,
    parameter_modes: &[MirParamMode],
    writable: Option<&MirCallWritableFacts>,
) -> Result<MirDirectCallFacts, String> {
    if !is_direct_target(&call.target) {
        return Err("direct-call facts require a direct target".to_string());
    }
    let loans = writable
        .map(|facts| facts.inout_loans.as_slice())
        .unwrap_or_default();
    let mut loans_by_parameter = BTreeMap::new();
    for loan in loans {
        if loans_by_parameter
            .insert(loan.parameter_ordinal, loan)
            .is_some()
        {
            return Err(format!(
                "direct call repeats inout parameter ordinal {}",
                loan.parameter_ordinal
            ));
        }
    }

    if call.concrete_receiver.is_some()
        && (parameter_modes.first() != Some(&MirParamMode::Value) || call.args.first().is_none())
    {
        return Err(
            "direct receiver call requires a leading Value argument at parameter zero".to_string(),
        );
    }

    let mut values = call.args.iter().copied();
    let mut arguments = Vec::with_capacity(parameter_modes.len());
    for (parameter_ordinal, mode) in parameter_modes.iter().enumerate() {
        let parameter_ordinal = u32::try_from(parameter_ordinal)
            .map_err(|_| "direct call parameter count exceeds u32::MAX".to_string())?;
        match mode {
            MirParamMode::Value => {
                let value = values.next().ok_or_else(|| {
                    format!(
                        "direct call is missing compact Value argument for parameter {parameter_ordinal}"
                    )
                })?;
                if loans_by_parameter.contains_key(&parameter_ordinal) {
                    return Err(format!(
                        "direct call parameter {parameter_ordinal} is both Value and inout"
                    ));
                }
                arguments.push(MirCallArgument::Value { value });
            }
            MirParamMode::InOut => {
                let loan = loans_by_parameter
                    .remove(&parameter_ordinal)
                    .ok_or_else(|| {
                        format!(
                            "direct call is missing inout loan for parameter {parameter_ordinal}"
                        )
                    })?;
                arguments.push(MirCallArgument::InOut { loan: loan.clone() });
            }
        }
    }
    if values.next().is_some() {
        return Err("direct call has extra compact Value arguments".to_string());
    }
    if let Some((parameter_ordinal, _)) = loans_by_parameter.first_key_value() {
        return Err(format!(
            "direct call has an inout loan for non-InOut parameter {parameter_ordinal}"
        ));
    }
    if call.inout_args.len() != loans.len() {
        return Err(
            "direct call raw inout table disagrees with its checked writable loans".to_string(),
        );
    }

    Ok(MirDirectCallFacts {
        concrete_receiver: call.concrete_receiver.clone(),
        receiver_call_abi: call
            .concrete_receiver
            .as_ref()
            .map(|_| ReceiverCallAbi::ExplicitSelfFirst),
        parameter_modes: parameter_modes.to_vec(),
        arguments,
    })
}
