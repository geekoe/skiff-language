use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_model::{addr::ExecutableAddr, type_plan::RuntimeTypePlan};

use crate::CapabilityResult;

pub trait ConfigCapabilityApi: Send + Sync {
    fn owned(&self) -> OwnedConfigCapabilityContext;
    fn borrow(&self) -> ConfigCapabilityContext<'_>;
    fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> CapabilityResult<Value>;
}

#[derive(Clone)]
pub struct ConfigCapabilityContext<'a> {
    inner: Arc<dyn ConfigCapabilityApi + 'a>,
}

impl<'a> ConfigCapabilityContext<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: ConfigCapabilityApi + 'a,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn owned(context: &Self) -> OwnedConfigCapabilityContext {
        context.inner.owned()
    }

    pub fn borrow(&self) -> ConfigCapabilityContext<'_> {
        self.inner.borrow()
    }

    pub fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> CapabilityResult<Value> {
        self.inner
            .read_config_target(current_addr, target, args, type_arg)
    }
}

pub type OwnedConfigCapabilityContext = ConfigCapabilityContext<'static>;
