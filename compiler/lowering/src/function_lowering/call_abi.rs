//! Source-owned direct-call ABI validation.

use std::collections::BTreeSet;

use skiff_compiler_source::{ExpressionKey, ResolvedCallTarget, SourceExecutableReceiver};
use skiff_syntax::{
    ast::CallArg,
    error::{CompileError, Result},
};

use crate::file_ir::{CallTargetIr, ParamModeIr, TypeRefIr};

use super::{resolved_call_target_kind, FunctionLowerer};

impl FunctionLowerer<'_> {
    pub(super) fn validate_direct_call_modes(
        &self,
        expression_key: Option<&ExpressionKey>,
        target: &CallTargetIr,
        concrete_receiver: Option<&TypeRefIr>,
        args: &[CallArg],
    ) -> Result<()> {
        let is_direct = matches!(
            target,
            CallTargetIr::LocalExecutable { .. }
                | CallTargetIr::PublicationExecutable { .. }
                | CallTargetIr::PackageCallable { .. }
        );
        if !is_direct {
            if concrete_receiver.is_some() {
                return Err(CompileError::Semantic(
                    "non-direct call carries a concrete receiver fact".to_string(),
                ));
            }
            if args
                .iter()
                .any(|arg| matches!(arg, CallArg::InOutPlace { .. }))
            {
                return Err(CompileError::Semantic(
                    "inout arguments require an exact direct call target".to_string(),
                ));
            }
            return Ok(());
        }

        let resolved = expression_key
            .and_then(|key| self.resolved_call_targets.target(key))
            .ok_or_else(|| {
                CompileError::Semantic(
                    "direct call has no exact source-owned target fact".to_string(),
                )
            })?;
        let parameter_modes = match resolved {
            ResolvedCallTarget::LocalFunction {
                source_callable, ..
            }
            | ResolvedCallTarget::LocalImplMethod {
                source_callable, ..
            } => {
                let signature = self
                    .exact_executable_signatures
                    .signature(source_callable)
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "direct call target `{source_callable}` has no exact source signature"
                        ))
                    })?;
                let mut modes = signature
                    .parameters
                    .iter()
                    .map(|parameter| parameter.mode)
                    .collect::<Vec<_>>();
                match signature.receiver {
                    SourceExecutableReceiver::None => {
                        if concrete_receiver.is_some() {
                            return Err(CompileError::Semantic(format!(
                                "non-receiver direct target `{source_callable}` carries concreteReceiver"
                            )));
                        }
                    }
                    SourceExecutableReceiver::Implicit { .. } => {
                        if concrete_receiver.is_none() {
                            return Err(CompileError::Semantic(format!(
                                "receiver-bound direct target `{source_callable}` has no concreteReceiver"
                            )));
                        }
                        modes.insert(0, ParamModeIr::Value);
                    }
                    SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 } => {
                        if concrete_receiver.is_none() {
                            return Err(CompileError::Semantic(format!(
                                "receiver-bound direct target `{source_callable}` has no concreteReceiver"
                            )));
                        }
                        if modes.first() != Some(&ParamModeIr::Value) {
                            return Err(CompileError::Semantic(format!(
                                "receiver-bound direct target `{source_callable}` has a non-Value explicit receiver"
                            )));
                        }
                    }
                    SourceExecutableReceiver::ExplicitParameter { parameter_index } => {
                        return Err(CompileError::Semantic(format!(
                            "receiver-bound direct target `{source_callable}` uses explicit receiver parameter {parameter_index}, expected 0"
                        )));
                    }
                }
                modes
            }
            ResolvedCallTarget::DependencyPackageFunction {
                package_callable_id,
                exact_signature,
                inout_parameters,
                ..
            } => {
                let signature = exact_signature.as_ref().ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "package-direct target `{package_callable_id}` has no exact signature"
                    ))
                })?;
                let modes = signature
                    .parameters
                    .iter()
                    .map(|parameter| parameter.mode)
                    .collect::<Vec<_>>();
                let receiver_offset = usize::from(concrete_receiver.is_some());
                let exact_inout = modes
                    .iter()
                    .enumerate()
                    .filter(|(_, mode)| **mode == ParamModeIr::InOut)
                    .map(|(index, _)| index.checked_sub(receiver_offset))
                    .collect::<Option<BTreeSet<_>>>()
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "package-direct target `{package_callable_id}` marks its receiver parameter inout"
                        ))
                    })?;
                let declared_inout = inout_parameters.keys().copied().collect::<BTreeSet<_>>();
                if exact_inout != declared_inout {
                    return Err(CompileError::Semantic(format!(
                        "package-direct target `{package_callable_id}` has inconsistent exact parameter modes and inout positions"
                    )));
                }
                if receiver_offset == 1
                    && (signature
                        .parameters
                        .first()
                        .map(|parameter| parameter.name.as_str())
                        != Some("self")
                        || modes.first() != Some(&ParamModeIr::Value))
                {
                    return Err(CompileError::Semantic(format!(
                        "receiver-bound package-direct target `{package_callable_id}` has no leading Value self parameter"
                    )));
                }
                modes
            }
            other => {
                return Err(CompileError::Semantic(format!(
                    "File IR direct target disagrees with source target kind `{}`",
                    resolved_call_target_kind(other)
                )));
            }
        };

        let receiver_offset = usize::from(concrete_receiver.is_some());
        if parameter_modes.len() != args.len() + receiver_offset {
            return Err(CompileError::Semantic(format!(
                "direct call has {} source arguments and receiver offset {receiver_offset}, but exact target has {} ABI parameters",
                args.len(),
                parameter_modes.len()
            )));
        }
        if receiver_offset == 1 && parameter_modes.first() != Some(&ParamModeIr::Value) {
            return Err(CompileError::Semantic(
                "direct receiver call ABI parameter zero is not Value".to_string(),
            ));
        }
        for (source_ordinal, arg) in args.iter().enumerate() {
            let parameter_ordinal = source_ordinal + receiver_offset;
            let actual = match arg {
                CallArg::Value(_) => ParamModeIr::Value,
                CallArg::InOutPlace { .. } => ParamModeIr::InOut,
            };
            if parameter_modes[parameter_ordinal] != actual {
                return Err(CompileError::Semantic(format!(
                    "direct call argument {source_ordinal} maps to parameter {parameter_ordinal} with mode {actual:?}, expected {:?}",
                    parameter_modes[parameter_ordinal]
                )));
            }
        }
        Ok(())
    }
}
