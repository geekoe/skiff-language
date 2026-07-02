#![allow(dead_code)]

use std::collections::BTreeMap;

#[cfg(test)]
use skiff_artifact_model::NativeSignatureDef;
use skiff_runtime_linked_program::{CallIr, ExecutableAddr, LinkedTypeRef, NativeTarget};
use skiff_runtime_native_contract::NativeCallPlan;

use crate::{error::Result, type_descriptor::ProgramTypeView};

// N1/EVAL5 temporary adapter for legacy std native schema imports.
//
// Owner: EVAL5-type-plan-contract.
// Deletion/narrowing point: after remaining non-eval runtime users import
// `skiff_runtime_linked_type_plan` or `skiff_runtime_native_contract` directly.
#[allow(unused_imports)]
pub(crate) use skiff_runtime_native_contract::{NativeCallValidation, NativeSignatureRegistry};

#[allow(dead_code)]
pub(crate) fn validate_native_call_artifact(
    target: &NativeTarget,
    arg_count: usize,
    type_args: &BTreeMap<String, LinkedTypeRef>,
    enclosing_type_params: &[String],
) -> NativeCallValidation {
    skiff_runtime_linked_type_plan::validate_native_call_artifact(
        target,
        arg_count,
        type_args,
        enclosing_type_params,
    )
}

pub(crate) fn resolve_native_call_plan<'a>(
    binding_key: &str,
    diagnostic_target: &str,
    call: &CallIr,
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
) -> Result<Option<NativeCallPlan>> {
    Ok(skiff_runtime_linked_type_plan::resolve_native_call_plan(
        binding_key,
        diagnostic_target,
        call,
        program,
        current_addr,
        substitutions,
    )?)
}

pub(crate) fn program_call_first_type_arg_plan<'a>(
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    call: &CallIr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
) -> Result<Option<skiff_runtime_boundary::type_descriptor::RuntimeTypePlan>> {
    Ok(
        skiff_runtime_linked_type_plan::program_call_first_type_arg_plan(
            program,
            current_addr,
            call,
            substitutions,
        )?,
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn native_signature(binding_key: &str) -> Option<&'static NativeSignatureDef> {
    skiff_runtime_linked_type_plan::native_call_plan::native_signature(binding_key)
}
