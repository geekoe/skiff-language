use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::Value;
use skiff_runtime_model::type_plan::RuntimeTypePlan;

use crate::CapabilityResult;

pub const HTTP_REQUEST_ADMIN_OVERRIDE_ENV: &str = "SKIFF_HTTP_ADMIN_ALLOW_UNSAFE";

#[derive(Debug, Clone)]
pub struct HttpRuntimeOptions {
    allow_unsafe_targets: bool,
    egress_proxy: Option<String>,
}

impl HttpRuntimeOptions {
    pub fn from_env() -> Self {
        Self {
            allow_unsafe_targets: matches!(
                std::env::var(HTTP_REQUEST_ADMIN_OVERRIDE_ENV)
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
                "1" | "true" | "yes" | "on"
            ),
            egress_proxy: None,
        }
    }

    pub fn explicit(allow_unsafe_targets: bool) -> Self {
        Self {
            allow_unsafe_targets,
            egress_proxy: None,
        }
    }

    pub fn allow_unsafe_targets(&self) -> bool {
        self.allow_unsafe_targets
    }

    pub fn with_allow_unsafe_targets(mut self, allow_unsafe_targets: bool) -> Self {
        self.allow_unsafe_targets = allow_unsafe_targets;
        self
    }

    pub fn with_egress_proxy(mut self, egress_proxy: Option<String>) -> Self {
        self.egress_proxy = egress_proxy;
        self
    }

    pub fn egress_proxy(&self) -> Option<&str> {
        self.egress_proxy.as_deref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn allowing_unsafe_targets_for_tests() -> Self {
        Self {
            allow_unsafe_targets: true,
            egress_proxy: None,
        }
    }
}

pub type HttpCapabilityFuture<'a, T> =
    Pin<Box<dyn Future<Output = CapabilityResult<T>> + Send + 'a>>;

pub trait HttpClientCapabilityApi: Send + Sync {
    fn dispatch_http_request<'a>(&'a self, input: &'a Value) -> HttpCapabilityFuture<'a, Value>;

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value>;

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value>;
}

#[derive(Clone)]
pub struct HttpClientCapabilityContext {
    inner: Arc<dyn HttpClientCapabilityApi>,
}

impl HttpClientCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: HttpClientCapabilityApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub async fn dispatch_http_request(&self, input: &Value) -> CapabilityResult<Value> {
        self.inner.dispatch_http_request(input).await
    }

    pub async fn dispatch_http_stream(
        &self,
        input: &Value,
        expected_body_item_type: Option<&RuntimeTypePlan>,
    ) -> CapabilityResult<Value> {
        self.inner
            .dispatch_http_stream(input, expected_body_item_type)
            .await
    }

    pub async fn dispatch_http_sse(
        &self,
        input: &Value,
        expected_item_type: Option<&RuntimeTypePlan>,
    ) -> CapabilityResult<Value> {
        self.inner.dispatch_http_sse(input, expected_item_type).await
    }
}
