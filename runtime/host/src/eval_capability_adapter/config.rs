use super::*;
use skiff_runtime_model::{addr::ExecutableAddr, type_plan::RuntimeTypePlan};

pub(super) struct RuntimeConfigCapabilityContext<'a>(
    pub(super) concrete::ConfigCapabilityContext<'a>,
);

impl capability_contract::ConfigCapabilityApi for RuntimeConfigCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedConfigCapabilityContext {
        capability_contract::ConfigCapabilityContext::new(RuntimeConfigCapabilityContext(
            self.0.owned(),
        ))
    }

    fn borrow(&self) -> capability_contract::ConfigCapabilityContext<'_> {
        config_context(self.0.borrow())
    }

    fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> capability_contract::CapabilityResult<Value> {
        self.0
            .read_config_target(current_addr, target, args, type_arg)
            .map_err(capability_contract::CapabilityError::opaque)
    }
}
