use std::sync::Arc;

use skiff_runtime_model::runtime_value::ActorRef;

use crate::{
    ActorFindControlRequest, ActorPutControlRequest, ActorRemoveControlRequest, CapabilityFuture,
    CapabilityResult, SpawnSubmitControlRequest,
};

pub trait ActorCapabilityApi: Send + Sync {
    fn owned(&self) -> OwnedActorCapabilityContext;
    fn borrow(&self) -> ActorCapabilityContext<'_>;

    // Request/invocation metadata consumed by eval when assembling actor and spawn control requests.
    fn runtime_id(&self) -> &str;
    fn service_id(&self) -> &str;
    fn service_version(&self) -> &str;
    fn request_id(&self) -> &str;
    fn request_target(&self) -> &str;
    fn request_build_id(&self) -> &str;
    fn spawn_service_protocol_identity(&self) -> &str;
    fn request_service_protocol_identity(&self) -> &str;
    fn operation_service_protocol_identity(&self) -> Option<&str>;
    fn activation_identity(&self) -> Option<&str>;
    fn trace_id(&self) -> Option<&str>;

    // Actor storage and spawn control operations provided by the host/runtime.
    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef>;

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>>;

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool>;

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()>;
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

    pub fn spawn_service_protocol_identity(&self) -> &str {
        self.inner.spawn_service_protocol_identity()
    }

    pub fn request_service_protocol_identity(&self) -> &str {
        self.inner.request_service_protocol_identity()
    }

    pub fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.inner.operation_service_protocol_identity()
    }

    pub fn activation_identity(&self) -> Option<&str> {
        self.inner.activation_identity()
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.inner.trace_id()
    }

    pub async fn put_actor(
        &self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> CapabilityResult<ActorRef> {
        self.inner.put_actor(request, object_payload).await
    }

    pub async fn find_actor(
        &self,
        request: ActorFindControlRequest,
    ) -> CapabilityResult<Option<ActorRef>> {
        self.inner.find_actor(request).await
    }

    pub async fn remove_actor(
        &self,
        request: ActorRemoveControlRequest,
    ) -> CapabilityResult<bool> {
        self.inner.remove_actor(request).await
    }

    pub async fn submit_spawn(
        &self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> CapabilityResult<()> {
        self.inner.submit_spawn(request, args_payload).await
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

    pub async fn put_actor(
        &self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> CapabilityResult<ActorRef> {
        self.context.put_actor(request, object_payload).await
    }

    pub async fn find_actor(
        &self,
        request: ActorFindControlRequest,
    ) -> CapabilityResult<Option<ActorRef>> {
        self.context.find_actor(request).await
    }

    pub async fn remove_actor(
        &self,
        request: ActorRemoveControlRequest,
    ) -> CapabilityResult<bool> {
        self.context.remove_actor(request).await
    }

    pub async fn submit_spawn(
        &self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> CapabilityResult<()> {
        self.context.submit_spawn(request, args_payload).await
    }
}
