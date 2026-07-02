use skiff_runtime_linked_program::{ExecutableAddr, LinkedExecutable, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::type_plan::RuntimeTypePlan;

use crate::error::{Result, RuntimeError, TypeIdentity};

use super::type_descriptor::TypeSubstitutions;
use super::{
    exceptions::{catch_type_leaves, throw_payload_actual_type},
    invocation::{EvalProgramProjection, ResolvedEvalExecutable},
    program_types::{call_type_substitutions, normalize_program_type_ref, program_type_ref_kind},
    Interpreter,
};

pub struct EvalTypeProjection<'a> {
    program: EvalProgramProjection<'a>,
}

impl Interpreter {
    pub fn type_projection(&self) -> Result<EvalTypeProjection<'_>> {
        Ok(EvalTypeProjection::new(self.program_projection()?))
    }
}

impl<'a> EvalTypeProjection<'a> {
    pub fn new(program: EvalProgramProjection<'a>) -> Self {
        Self { program }
    }

    pub fn plan_from_linked_nested_ref(
        &self,
        type_ref: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
    ) -> Result<RuntimeTypePlan> {
        Ok(RuntimeTypePlan::from_linked_nested_ref(
            type_ref,
            &PlanContext::from_type_view(self.program.type_view(), current_addr),
        )?)
    }

    pub fn plan_from_linked_nested_ref_with_substitutions(
        &self,
        type_ref: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
        substitutions: &TypeSubstitutions,
    ) -> Result<RuntimeTypePlan> {
        Ok(RuntimeTypePlan::from_linked_nested_ref(
            type_ref,
            &PlanContext::with_substitutions_from_type_view(
                self.program.type_view(),
                current_addr,
                substitutions.as_linked_map(),
            ),
        )?)
    }

    pub fn validate_construct_type_ref(
        &self,
        current_addr: &ExecutableAddr,
        type_ref: &LinkedTypeRef,
        substitutions: &TypeSubstitutions,
    ) -> Result<()> {
        let normalized = normalize_program_type_ref(
            self.program.type_view(),
            current_addr,
            type_ref,
            substitutions,
        );
        match normalized {
            LinkedTypeRef::Address { addr } => {
                self.program.canonical_type_addr(&addr)?;
                Ok(())
            }
            LinkedTypeRef::LocalType { .. }
            | LinkedTypeRef::ServiceSymbol { .. }
            | LinkedTypeRef::PackageSymbol { .. } => Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram construct type_ref did not resolve to a concrete type address: {}",
                program_type_ref_kind(type_ref)
            ))),
            _ => Ok(()),
        }
    }

    pub fn throw_payload_actual_type(&self, payload_type: &LinkedTypeRef) -> Result<TypeIdentity> {
        throw_payload_actual_type(payload_type, self.program.type_view())
    }

    pub fn catch_type_leaves(&self, catch_type: &LinkedTypeRef) -> Result<Vec<TypeIdentity>> {
        catch_type_leaves(catch_type, self.program.type_view())
    }

    pub fn resolve_executable(&self, addr: &ExecutableAddr) -> Result<ResolvedEvalExecutable<'a>> {
        self.program.resolve_executable(addr)
    }

    pub fn call_type_substitutions(
        &self,
        caller_addr: &ExecutableAddr,
        caller_substitutions: &TypeSubstitutions,
        callee: &LinkedExecutable,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
    ) -> TypeSubstitutions {
        call_type_substitutions(
            self.program.type_view(),
            caller_addr,
            caller_substitutions,
            callee,
            type_args,
        )
    }
}
