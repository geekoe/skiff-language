use crate::error::{Result, RuntimeError};
use crate::{boundary::NativeBoundaryAdapter, runtime_value_facade::RuntimeTypePlan};
use skiff_runtime_native_contract::{NativeCallPlan, NativeRequiredContext};

pub struct RuntimeNativeInvocation {
    target_name: String,
    binding_key: String,
    plan: Option<NativeCallPlan>,
    actor_metadata: Option<RuntimeActorNativeMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeActorNativeMetadata {
    actor_type_identity: String,
    actor_id_type_identity: String,
}

impl RuntimeActorNativeMetadata {
    pub fn new(actor_type_identity: String, actor_id_type_identity: String) -> Self {
        Self {
            actor_type_identity,
            actor_id_type_identity,
        }
    }

    pub fn actor_type_identity(&self) -> &str {
        &self.actor_type_identity
    }

    pub fn actor_id_type_identity(&self) -> &str {
        &self.actor_id_type_identity
    }
}

impl RuntimeNativeInvocation {
    pub fn new(
        target_name: String,
        binding_key: &str,
        plan: Option<NativeCallPlan>,
        actor_metadata: Option<RuntimeActorNativeMetadata>,
    ) -> Self {
        Self {
            target_name,
            binding_key: binding_key.to_string(),
            plan,
            actor_metadata,
        }
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn binding_key(&self) -> &str {
        self.plan
            .as_ref()
            .map(|plan| plan.binding_key.as_str())
            .unwrap_or(self.binding_key.as_str())
    }

    pub fn required_context(&self) -> NativeRequiredContext {
        self.plan
            .as_ref()
            .map(|plan| plan.required_context)
            .unwrap_or(NativeRequiredContext::None)
    }

    pub fn plan(&self) -> Option<&NativeCallPlan> {
        self.plan.as_ref()
    }

    pub fn require_plan(&self) -> Result<&NativeCallPlan> {
        self.plan.as_ref().ok_or_else(|| {
            RuntimeError::Unsupported(format!("unsupported native target {}", self.binding_key()))
        })
    }

    pub fn arg_count(&self) -> Result<usize> {
        Ok(self.require_plan()?.arg_plans.len())
    }

    pub fn arg_plans(&self) -> Result<&[RuntimeTypePlan]> {
        Ok(&self.require_plan()?.arg_plans)
    }

    pub fn arg_plan(&self, index: usize) -> Result<&RuntimeTypePlan> {
        self.require_plan()?.arg_plans.get(index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{} resolved native call is missing argument plan {index}",
                self.target_name
            ))
        })
    }

    pub fn return_plan(&self) -> Result<&RuntimeTypePlan> {
        Ok(&self.require_plan()?.return_plan)
    }

    pub(crate) fn native_boundary(&self) -> Result<NativeBoundaryAdapter<'_>> {
        Ok(NativeBoundaryAdapter::new(
            self.target_name(),
            self.require_plan()?,
        ))
    }

    pub fn actor_metadata(&self) -> Result<&RuntimeActorNativeMetadata> {
        self.actor_metadata.as_ref().ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{} resolved actor native call is missing actor metadata",
                self.target_name
            ))
        })
    }
}
