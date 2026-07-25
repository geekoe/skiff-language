use skiff_runtime_linked_program::{ExecutableAddr, LinkedExecutable, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{service_error::CatchIdentity, type_plan::RuntimeTypePlan};

use crate::{
    assembly_execution::RuntimeExecutionProjection,
    error::{Result, RuntimeError},
};

use super::type_descriptor::TypeSubstitutions;
use super::{
    exceptions::{annotate_runtime_type_plan, catch_type_leaves},
    invocation::{EvalProgramProjection, ResolvedEvalExecutable},
    program_execution::ProgramExecutionContext,
    program_types::{call_type_substitutions, normalize_program_type_ref, program_type_ref_kind},
    Interpreter,
};

pub struct EvalTypeProjection<'a> {
    program: RuntimeExecutionProjection<'a>,
}

impl Interpreter {
    pub fn type_projection(&self) -> Result<EvalTypeProjection<'_>> {
        Ok(EvalTypeProjection::new(self.program_projection()?))
    }

    pub(crate) fn type_projection_for_context(
        &self,
        context: &ProgramExecutionContext<'_>,
    ) -> Result<EvalTypeProjection<'_>> {
        Ok(EvalTypeProjection::from_execution_projection(
            RuntimeExecutionProjection::for_context(self, context)?,
        ))
    }
}

impl<'a> EvalTypeProjection<'a> {
    pub fn new(program: EvalProgramProjection<'a>) -> Self {
        Self {
            program: RuntimeExecutionProjection::Legacy(program),
        }
    }

    pub(crate) fn from_execution_projection(program: RuntimeExecutionProjection<'a>) -> Self {
        Self { program }
    }

    pub fn plan_from_linked_nested_ref(
        &self,
        type_ref: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
    ) -> Result<RuntimeTypePlan> {
        let canonical_type_ref;
        let type_ref =
            if let (RuntimeExecutionProjection::Assembly(_), LinkedTypeRef::Address { addr }) =
                (&self.program, type_ref)
            {
                canonical_type_ref = LinkedTypeRef::Address {
                    addr: self.program.canonical_type_addr(addr)?,
                };
                &canonical_type_ref
            } else {
                type_ref
            };
        let normalized = normalize_program_type_ref(
            self.program.type_view(),
            current_addr,
            type_ref,
            &TypeSubstitutions::new(),
        );
        let mut plan = RuntimeTypePlan::from_linked_nested_ref(
            &normalized,
            &PlanContext::from_type_view(self.program.type_view(), current_addr),
        )?;
        annotate_runtime_type_plan(&mut plan, &normalized, self.program.type_view())?;
        Ok(plan)
    }

    pub fn plan_from_linked_nested_ref_with_substitutions(
        &self,
        type_ref: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
        substitutions: &TypeSubstitutions,
    ) -> Result<RuntimeTypePlan> {
        let normalized = normalize_program_type_ref(
            self.program.type_view(),
            current_addr,
            type_ref,
            substitutions,
        );
        let mut plan = RuntimeTypePlan::from_linked_nested_ref(
            &normalized,
            &PlanContext::with_substitutions_from_type_view(
                self.program.type_view(),
                current_addr,
                substitutions.as_linked_map(),
            ),
        )?;
        annotate_runtime_type_plan(&mut plan, &normalized, self.program.type_view())?;
        Ok(plan)
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

    pub fn catch_type_leaves(
        &self,
        catch_type: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
        substitutions: &TypeSubstitutions,
    ) -> Result<Vec<CatchIdentity>> {
        let catch_type = normalize_program_type_ref(
            self.program.type_view(),
            current_addr,
            catch_type,
            substitutions,
        );
        catch_type_leaves(&catch_type, self.program.type_view())
    }

    pub fn resolve_executable(&self, addr: &ExecutableAddr) -> Result<ResolvedEvalExecutable<'a>> {
        self.program
            .legacy("public type-projection executable")?
            .resolve_executable(addr)
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
