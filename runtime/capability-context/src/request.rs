use std::sync::Arc;

use crate::{
    ActivationIdentityControl, CapabilityFuture, CapabilityResult, OwnedExecutionControl,
    TaskCancelControlRequest, TaskCancelControlResponse, TaskStatusControlRequest,
    TaskStatusControlResponse, TaskSubmitControlRequest, TaskSubmitResponseControl,
};

/// Request/invocation metadata and `task.submit` operations provided by the host/runtime.
///
/// Actor model operations live on [`crate::ActorCapabilityApi`]; this trait is the
/// single entry point for request-wide metadata and task submission.
pub trait RequestCapabilityApi: Send + Sync {
    fn owned(&self) -> OwnedRequestCapabilityContext;
    fn borrow(&self) -> RequestCapabilityContext<'_>;

    fn runtime_id(&self) -> &str;
    fn service_id(&self) -> &str;
    fn service_version(&self) -> &str;
    fn request_id(&self) -> &str;
    fn request_target(&self) -> &str;
    fn request_build_id(&self) -> &str;
    fn task_service_protocol_identity(&self) -> &str;
    fn request_service_protocol_identity(&self) -> &str;
    fn operation_service_protocol_identity(&self) -> Option<&str>;
    fn activation_identity(&self) -> Option<&ActivationIdentityControl>;
    fn trace_id(&self) -> Option<&str>;

    fn submit_task<'a>(
        &'a self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl>;

    fn status_task<'a>(
        &'a self,
        request: TaskStatusControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskStatusControlResponse>;

    fn cancel_task<'a>(
        &'a self,
        request: TaskCancelControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskCancelControlResponse>;
}

#[derive(Clone)]
pub struct RequestCapabilityContext<'a> {
    inner: Arc<dyn RequestCapabilityApi + 'a>,
}

impl<'a> RequestCapabilityContext<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: RequestCapabilityApi + 'a,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn owned(&self) -> OwnedRequestCapabilityContext {
        self.inner.owned()
    }

    pub fn borrow(&self) -> RequestCapabilityContext<'_> {
        self.inner.borrow()
    }

    pub fn runtime_id(&self) -> &str {
        self.inner.runtime_id()
    }

    pub fn service_id(&self) -> &str {
        self.inner.service_id()
    }

    pub fn service_version(&self) -> &str {
        self.inner.service_version()
    }

    pub fn request_id(&self) -> &str {
        self.inner.request_id()
    }

    pub fn request_target(&self) -> &str {
        self.inner.request_target()
    }

    pub fn request_build_id(&self) -> &str {
        self.inner.request_build_id()
    }

    pub fn task_service_protocol_identity(&self) -> &str {
        self.inner.task_service_protocol_identity()
    }

    pub fn request_service_protocol_identity(&self) -> &str {
        self.inner.request_service_protocol_identity()
    }

    pub fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.inner.operation_service_protocol_identity()
    }

    pub fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        self.inner.activation_identity()
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.inner.trace_id()
    }

    pub async fn submit_task(
        &self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<TaskSubmitResponseControl> {
        self.inner
            .submit_task(request, args_payload, execution_control)
            .await
    }

    pub async fn status_task(
        &self,
        request: TaskStatusControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<TaskStatusControlResponse> {
        self.inner.status_task(request, execution_control).await
    }

    pub async fn cancel_task(
        &self,
        request: TaskCancelControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<TaskCancelControlResponse> {
        self.inner.cancel_task(request, execution_control).await
    }
}

pub type OwnedRequestCapabilityContext = RequestCapabilityContext<'static>;
