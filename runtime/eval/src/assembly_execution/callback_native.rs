use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableCapabilityHooks, ServiceLinkableCapabilityProjection,
    ServiceLinkableCapabilityRequest, ServiceLinkableMaterializationError,
};
use skiff_runtime_linked_program::CallIr;
use skiff_runtime_model::runtime_value::{CallbackCapabilityCarrier, RuntimeValue};

use super::{AssemblyExecutionHandoffError, AssemblyExecutionLaneKind};
use crate::{error::Result, eval_context::EvalContext, program_execution::ProgramExecutionContext};

/// Frozen adapter passed by ordinary/stream materializers whenever a value plan requires an
/// explicit callback or native capability. T06 owns this module's concrete implementation.
pub(crate) struct CallbackNativeCapabilityHooks<'context, 'execution> {
    #[allow(dead_code)]
    context: &'context ProgramExecutionContext<'execution>,
}

impl<'context, 'execution> CallbackNativeCapabilityHooks<'context, 'execution> {
    pub(crate) fn new(context: &'context ProgramExecutionContext<'execution>) -> Self {
        Self { context }
    }
}

impl ServiceLinkableCapabilityHooks for CallbackNativeCapabilityHooks<'_, '_> {
    fn project_callback_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        Err(ServiceLinkableMaterializationError::CallbackHookRequired)
    }

    fn project_native_adapter_capability(
        &self,
        _request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        Err(ServiceLinkableMaterializationError::NativeAdapterHookRequired)
    }
}

pub(crate) async fn execute_interface_call(
    _context: &mut EvalContext<'_>,
    _call: &CallIr,
    _carrier: &CallbackCapabilityCarrier,
    _method_abi_id: &str,
    _slot: u32,
    _args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    Err(AssemblyExecutionHandoffError::unavailable_at(
        AssemblyExecutionLaneKind::CallbackNative,
        "callback-interface",
    ))
}
