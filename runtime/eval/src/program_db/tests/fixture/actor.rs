use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use sha2::{Digest, Sha256};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::DbCapabilityContext;
use skiff_runtime_linked_program::ServiceMeta;
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlan, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::request_heap::{RequestHeap, RequestHeapLimits};

use crate::{
    actor_executor::ActorExecutionFrame,
    actor_executor_test_runtime as test_runtime,
    actor_instance::{
        ActorActivationRequest, ActorExecutorAuthority, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceHandle, ActorInstanceStore, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
};

use super::{
    actor_abi, actor_implementation, actor_owner, integer_type, FakeDbContext, FakeDbState,
    LinkedDbActorFixture, ACTOR_SERVICE_ID, ACTOR_TYPE_ID,
};

pub(in crate::program_db::tests) struct ActorFixture {
    pub store: ActorInstanceStore,
    pub handle: ActorInstanceHandle,
    field_plan: RuntimeTypePlan,
}

impl ActorFixture {
    fn new(linked: &LinkedDbActorFixture) -> Self {
        let store = ActorInstanceStore::new();
        let actor_id_bytes = br#""fixture-1""#.to_vec();
        let handle = store
            .activate(ActorActivationRequest {
                fence: ActorInstanceFence {
                    incarnation: ActorIncarnationKey {
                        logical_key: ActorLogicalKey {
                            service_id: ACTOR_SERVICE_ID.to_string(),
                            actor_type_identity: ACTOR_TYPE_ID.to_string(),
                            actor_id_type_identity: "builtin:string".to_string(),
                            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                            actor_id_hash: format!(
                                "sha256:{}",
                                hex::encode(Sha256::digest(&actor_id_bytes))
                            ),
                            canonical_actor_id_key_bytes: actor_id_bytes,
                        },
                        epoch: 1,
                    },
                    actor_abi_identity: actor_abi(),
                    actor_implementation_identity: actor_implementation(),
                    declaration_owner: actor_owner(),
                },
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: br#"{"count":1}"#,
                program: linked.program.projection().type_view(),
            })
            .expect("DB/Actor fixture activation");
        let field_plan = RuntimeTypePlan::from_linked(
            &integer_type(),
            &PlanContext::from_type_view(linked.program.projection().type_view(), &linked.addr),
        )
        .expect("DB/Actor fixture field plan");
        Self {
            store,
            handle,
            field_plan,
        }
    }

    pub(in crate::program_db::tests) async fn execution_frame(
        &self,
    ) -> (ActorExecutionFrame, RequestHeap) {
        let authority = ActorExecutorAuthority::new();
        let mut lease = self
            .store
            .acquire_execution(&authority, &self.handle)
            .await
            .expect("DB/Actor fixture execution lease");
        let heap = lease.take_heap();
        (
            ActorExecutionFrame::new(
                self.store.clone(),
                self.handle.clone(),
                lease,
                vec![("count".to_string(), self.field_plan.clone())],
            ),
            heap,
        )
    }

    pub(in crate::program_db::tests) fn competing_acquire(
        &self,
    ) -> impl Future<
        Output = Result<
            crate::actor_instance::ActorInstanceExecutionLease,
            crate::actor_instance::ActorInstanceStoreError,
        >,
    > + Send
           + 'static {
        let store = self.store.clone();
        let handle = self.handle.clone();
        async move {
            let authority = ActorExecutorAuthority::new();
            store.acquire_execution(&authority, &handle).await
        }
    }
}

pub(in crate::program_db::tests) struct DbActorFixture {
    pub state: Arc<FakeDbState>,
    pub linked: LinkedDbActorFixture,
    pub actor: ActorFixture,
    db: FakeDbContext,
}

impl DbActorFixture {
    pub(in crate::program_db::tests) fn new(state: Arc<FakeDbState>) -> Self {
        let linked = LinkedDbActorFixture::new();
        let actor = ActorFixture::new(&linked);
        let db = FakeDbContext::new(Arc::clone(&state));
        Self {
            state,
            linked,
            actor,
            db,
        }
    }

    pub(in crate::program_db::tests) fn context(
        &self,
        frame: ActorExecutionFrame,
    ) -> ProgramExecutionContext<'static> {
        let execution = test_runtime::execution_control();
        let effects = test_runtime::effects_context();
        let actor = test_runtime::actor_context();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: test_runtime::config_context(),
            db: DbCapabilityContext::new(self.db.clone()),
            file: test_runtime::file_context(),
            file_source_stream: test_runtime::file_source_stream_context(
                self.linked.interpreter.stream_runtime.clone(),
            ),
            time: TimeCapabilityContext::new(execution),
            websocket: test_runtime::websocket_context(),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                self.linked.interpreter.http_options.clone(),
                self.linked.interpreter.stream_runtime.clone(),
                self.linked.interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: self.linked.interpreter.test_effect_double_context(),
            actor: actor.clone(),
            spawn: actor,
            request_heap_limits: RequestHeapLimits::default(),
        })
        .with_actor_execution_frame(frame)
    }
}

pub(in crate::program_db::tests) fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    future.poll(&mut Context::from_waker(waker))
}
