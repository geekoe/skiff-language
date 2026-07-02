use std::sync::Arc;

use serde_json::Value;

use crate::CapabilityResult;

pub trait TelemetryCapabilityApi: Send + Sync {
    fn emit_native(&self, target: &str, args: &[Value]) -> CapabilityResult<Value>;
}

#[derive(Clone)]
pub struct TelemetryCapabilityContext {
    inner: Arc<dyn TelemetryCapabilityApi>,
}

impl TelemetryCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: TelemetryCapabilityApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn emit_native(&self, target: &str, args: &[Value]) -> CapabilityResult<Value> {
        self.inner.emit_native(target, args)
    }
}
