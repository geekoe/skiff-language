//! Runtime config-view mapping for native config capability.

use std::borrow::Cow;

use serde_json::Value;
use skiff_runtime_linked_program::{ExecutableAddr, UnitAddr};
use skiff_runtime_model::type_plan::RuntimeTypePlan;

use crate::{
    config_view::{sanitize_wire_json, RuntimeConfigView},
    error::{Result, RuntimeError},
};

#[derive(Clone)]
pub struct ConfigCapabilityContext<'a> {
    service_config: Cow<'a, RuntimeConfigView>,
    package_configs: Cow<'a, [RuntimeConfigView]>,
}

impl<'a> ConfigCapabilityContext<'a> {
    pub fn new(
        service_config: &'a RuntimeConfigView,
        package_configs: &'a [RuntimeConfigView],
    ) -> Self {
        Self {
            service_config: Cow::Borrowed(service_config),
            package_configs: Cow::Borrowed(package_configs),
        }
    }

    pub fn owned(&self) -> ConfigCapabilityContext<'static> {
        ConfigCapabilityContext {
            service_config: Cow::Owned(self.service_config().clone()),
            package_configs: Cow::Owned(self.package_configs().to_vec()),
        }
    }

    pub fn borrow(&self) -> ConfigCapabilityContext<'_> {
        ConfigCapabilityContext::new(self.service_config(), self.package_configs())
    }

    pub fn service_config(&self) -> &RuntimeConfigView {
        self.service_config.as_ref()
    }

    pub fn package_configs(&self) -> &[RuntimeConfigView] {
        self.package_configs.as_ref()
    }

    pub fn config_for_executable(&self, addr: &ExecutableAddr) -> Result<&RuntimeConfigView> {
        match addr.unit {
            UnitAddr::Service => Ok(self.service_config()),
            UnitAddr::Package(slot) => self.package_configs().get(slot).ok_or_else(|| {
                RuntimeError::invalid_artifact(format!(
                    "RuntimeProgram package slot {slot} is missing scoped config"
                ))
            }),
        }
    }

    pub fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> Result<Value> {
        let value = self
            .config_for_executable(current_addr)?
            .dispatch_typed_config_target(target, args, type_arg)?;
        sanitize_wire_json(value)
    }
}
