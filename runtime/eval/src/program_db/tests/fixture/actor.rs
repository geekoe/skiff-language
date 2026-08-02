use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use sha2::{Digest, Sha256};
use skiff_runtime_capability_context::DbCapabilityContext;
use skiff_runtime_model::{request_heap::RequestHeapLimits, runtime_value::RuntimeValue};

use crate::{
    actor_executor::ActorExecutionFrame,
    actor_executor_test_runtime as test_runtime,
    actor_instance::{
        ActorActivationRequest, ActorExecutorAuthority, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceHandle, ActorInstanceStore, ActorLogicalKey, SegmentLease,
        ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::TimeCapabilityContext,
    heap_access::HeapAccess,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
};

use super::{
    actor_abi, actor_implementation, actor_owner, FakeDbContext, FakeDbState, LinkedDbActorFixture,
    ACTOR_SERVICE_ID, ACTOR_TYPE_ID,
};

pub(in crate::program_db::tests) struct ActorFixture {
    pub store: ActorInstanceStore,
    pub handle: ActorInstanceHandle,
}

impl ActorFixture {
    fn new(linked: &LinkedDbActorFixture) -> Self {
        Self::new_with_arena_limits(linked, RequestHeapLimits::default())
    }

    fn new_with_arena_limits(
        linked: &LinkedDbActorFixture,
        arena_limits: RequestHeapLimits,
    ) -> Self {
        let mut store = ActorInstanceStore::new();
        store.arena_limits = arena_limits;
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
                bootstrap_payload: br#"[]"#,
                program: linked.program.projection().type_view(),
            })
            .expect("DB/Actor fixture activation");
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _| {
                fields[1].value = RuntimeValue::Number(1.0);
                fields[1].assigned = true;
                fields[2].value = RuntimeValue::Number(1.0);
                fields[2].assigned = true;
            })
            .unwrap();
        store
            .mark_admitted(&ActorExecutorAuthority::new(), &handle)
            .expect("DB/Actor fixture instance must be admitted");
        Self { store, handle }
    }

    pub(in crate::program_db::tests) async fn execution_frame(
        &self,
    ) -> (ActorExecutionFrame, HeapAccess<'static>) {
        let authority = ActorExecutorAuthority::new();
        let mut segment = self
            .store
            .acquire_segment(&authority, &self.handle)
            .await
            .expect("DB/Actor fixture execution segment");
        let access = HeapAccess::Shared {
            arena: segment.arena().clone(),
            guard: Some(segment.take_guard()),
        };
        (
            ActorExecutionFrame::new(self.store.clone(), self.handle.clone(), segment, false),
            access,
        )
    }

    pub(in crate::program_db::tests) fn competing_acquire(
        &self,
    ) -> impl Future<Output = Result<SegmentLease, crate::actor_instance::ActorInstanceStoreError>>
           + Send
           + 'static {
        let store = self.store.clone();
        let handle = self.handle.clone();
        async move {
            let authority = ActorExecutorAuthority::new();
            store.acquire_segment(&authority, &handle).await
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
        Self::new_with_arena_limits(state, RequestHeapLimits::default())
    }

    pub(in crate::program_db::tests) fn new_with_arena_limits(
        state: Arc<FakeDbState>,
        arena_limits: RequestHeapLimits,
    ) -> Self {
        let linked = LinkedDbActorFixture::new();
        let actor = ActorFixture::new_with_arena_limits(&linked, arena_limits);
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
        self.context_with_request(Some(frame), test_runtime::request_context())
    }

    pub(in crate::program_db::tests) fn ordinary_context(
        &self,
    ) -> ProgramExecutionContext<'static> {
        self.context_with_request(None, test_runtime::request_context())
    }

    pub(in crate::program_db::tests) fn ordinary_context_with_trace(
        &self,
        trace_id: &'static str,
    ) -> ProgramExecutionContext<'static> {
        self.context_with_request(None, test_runtime::request_context_with_trace(trace_id))
    }

    fn context_with_request(
        &self,
        frame: Option<ActorExecutionFrame>,
        request: skiff_runtime_capability_context::RequestCapabilityContext<'static>,
    ) -> ProgramExecutionContext<'static> {
        let execution = test_runtime::execution_control();
        let effects = test_runtime::effects_context();
        let actor = test_runtime::actor_context();
        let mut context = ProgramExecutionContext::new(ProgramExecutionInput {
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
            request,
            request_heap_limits: RequestHeapLimits::default(),
        });
        if let Some(frame) = frame {
            context = context.with_actor_execution_frame(frame);
        }
        context
    }
}

pub(in crate::program_db::tests) fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    future.poll(&mut Context::from_waker(waker))
}
