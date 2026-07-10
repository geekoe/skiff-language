use super::*;

#[derive(Clone)]
pub(super) struct RuntimeOwnedActorParts {
    pub(super) runtime_id: String,
    pub(super) service_id: String,
    pub(super) service_version: String,
    pub(super) request_id: String,
    pub(super) request_target: String,
    pub(super) request_build_id: String,
    pub(super) request_service_protocol_identity: String,
    pub(super) operation_service_protocol_identity: Option<String>,
    pub(super) activation_identity: Option<String>,
    pub(super) trace_id: Option<String>,
    pub(super) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(super) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(super) cancellation: CancellationToken,
}

pub(super) fn actor<'a>(
    context: concrete::ActorCapabilityContext<'a>,
    owned: RuntimeOwnedActorParts,
) -> eval_capabilities::ActorCapabilityContext<'a> {
    eval_capabilities::ActorCapabilityContext::new(RuntimeActorCapabilityContext { context, owned })
}

#[derive(Clone)]
pub(super) struct RuntimeActorCapabilityContext<'a> {
    context: concrete::ActorCapabilityContext<'a>,
    owned: RuntimeOwnedActorParts,
}

impl capability_contract::ActorCapabilityApi for RuntimeActorCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedActorCapabilityContext(
            self.owned.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        actor(self.context.clone(), self.owned.clone())
    }

    fn runtime_id(&self) -> &str {
        self.context.runtime_id()
    }
    fn service_id(&self) -> &str {
        self.context.service_id()
    }
    fn service_version(&self) -> &str {
        self.context.service_version()
    }
    fn request_id(&self) -> &str {
        self.context.request_id()
    }
    fn request_target(&self) -> &str {
        self.context.request_target()
    }
    fn request_build_id(&self) -> &str {
        self.context.request_build_id()
    }
    fn spawn_service_protocol_identity(&self) -> &str {
        self.context.spawn_service_protocol_identity()
    }
    fn request_service_protocol_identity(&self) -> &str {
        self.context.request_service_protocol_identity()
    }
    fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.context.operation_service_protocol_identity()
    }
    fn activation_identity(&self) -> Option<&str> {
        self.context.activation_identity()
    }
    fn trace_id(&self) -> Option<&str> {
        self.context.trace_id()
    }

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .put(request, object_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .find(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .remove(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .submit_spawn(request, args_payload)
                .await
                .map(|_| ())
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }
}

struct RuntimeOwnedActorCapabilityContext(RuntimeOwnedActorParts);

impl capability_contract::ActorCapabilityApi for RuntimeOwnedActorCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedActorCapabilityContext(
            self.0.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        let context = concrete::ActorCapabilityContext::from_parts(
            &self.0.runtime_id,
            &self.0.service_id,
            &self.0.service_version,
            &self.0.request_id,
            &self.0.request_target,
            &self.0.request_build_id,
            &self.0.request_service_protocol_identity,
            self.0.operation_service_protocol_identity.as_deref(),
            self.0.activation_identity.as_deref(),
            self.0.trace_id.as_deref(),
            self.0.router_sender.as_ref(),
            self.0.outbound_requests.as_ref(),
            self.0.cancellation.clone(),
        );
        actor(context, self.0.clone())
    }

    fn runtime_id(&self) -> &str {
        &self.0.runtime_id
    }
    fn service_id(&self) -> &str {
        &self.0.service_id
    }
    fn service_version(&self) -> &str {
        &self.0.service_version
    }
    fn request_id(&self) -> &str {
        &self.0.request_id
    }
    fn request_target(&self) -> &str {
        &self.0.request_target
    }
    fn request_build_id(&self) -> &str {
        &self.0.request_build_id
    }
    fn spawn_service_protocol_identity(&self) -> &str {
        self.0
            .operation_service_protocol_identity
            .as_deref()
            .unwrap_or(&self.0.request_service_protocol_identity)
    }
    fn request_service_protocol_identity(&self) -> &str {
        &self.0.request_service_protocol_identity
    }
    fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.0.operation_service_protocol_identity.as_deref()
    }
    fn activation_identity(&self) -> Option<&str> {
        self.0.activation_identity.as_deref()
    }
    fn trace_id(&self) -> Option<&str> {
        self.0.trace_id.as_deref()
    }

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .put(request, object_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .find(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .remove(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .submit_spawn(request, args_payload)
                .await
                .map(|_| ())
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }
}
