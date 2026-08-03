use std::sync::Arc;

use skiff_runtime_model::runtime_value::ActorRef;

use crate::{
    ActorFindControlRequest, ActorGetOrCreateControlRequest, ActorInvocationOutcome,
    ActorInvocationRequest, ActorRemoveControlRequest, ActorReplaceControlRequest,
    CapabilityFuture, CapabilityResult, OwnedExecutionControl,
};

/// Actor storage and invocation operations provided by the host/runtime.
///
/// Request/invocation metadata and `submit_task` live on
/// [`crate::RequestCapabilityApi`] so actor-model consumers do not need them.
pub trait ActorCapabilityApi: Send + Sync {
    fn owned(&self) -> OwnedActorCapabilityContext;
    fn borrow(&self) -> ActorCapabilityContext<'_>;

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef>;

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef>;

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>>;

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool>;

    fn invoke_actor<'a>(
        &'a self,
        request: ActorInvocationRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome>;
}

#[derive(Clone)]
pub struct ActorCapabilityContext<'a> {
    inner: Arc<dyn ActorCapabilityApi + 'a>,
}

impl<'a> ActorCapabilityContext<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: ActorCapabilityApi + 'a,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn owned(&self) -> OwnedActorCapabilityContext {
        self.inner.owned()
    }

    pub fn borrow(&self) -> ActorCapabilityContext<'_> {
        self.inner.borrow()
    }

    pub async fn get_or_create_actor(
        &self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorRef> {
        self.inner
            .get_or_create_actor(request, bootstrap_payload, execution_control)
            .await
    }

    pub async fn replace_actor(
        &self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorRef> {
        self.inner
            .replace_actor(request, bootstrap_payload, execution_control)
            .await
    }

    pub async fn find_actor(
        &self,
        request: ActorFindControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<Option<ActorRef>> {
        self.inner.find_actor(request, execution_control).await
    }

    pub async fn remove_actor(
        &self,
        request: ActorRemoveControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<bool> {
        self.inner.remove_actor(request, execution_control).await
    }

    pub async fn invoke_actor(
        &self,
        request: ActorInvocationRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorInvocationOutcome> {
        self.inner.invoke_actor(request, execution_control).await
    }
}

pub type OwnedActorCapabilityContext = ActorCapabilityContext<'static>;

pub struct ActorClient<'a> {
    context: ActorCapabilityContext<'a>,
}

impl<'a> ActorClient<'a> {
    pub fn new(context: ActorCapabilityContext<'a>) -> Self {
        Self { context }
    }

    pub async fn get_or_create_actor(
        &self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorRef> {
        self.context
            .get_or_create_actor(request, bootstrap_payload, execution_control)
            .await
    }

    pub async fn replace_actor(
        &self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorRef> {
        self.context
            .replace_actor(request, bootstrap_payload, execution_control)
            .await
    }

    pub async fn find_actor(
        &self,
        request: ActorFindControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<Option<ActorRef>> {
        self.context.find_actor(request, execution_control).await
    }

    pub async fn remove_actor(
        &self,
        request: ActorRemoveControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<bool> {
        self.context.remove_actor(request, execution_control).await
    }

    pub async fn invoke_actor(
        &self,
        request: ActorInvocationRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<ActorInvocationOutcome> {
        self.context.invoke_actor(request, execution_control).await
    }
}
