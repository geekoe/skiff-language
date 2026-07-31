use super::*;

#[derive(Clone)]
pub(super) struct TestActorCapability {
    activation: ActivationIdentityControl,
    actor_ref: ActorRef,
    calls: Arc<AtomicUsize>,
}

impl NativeActorCapability for TestActorCapability {
    fn service_id(&self) -> &str {
        "service.test"
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        Some(&self.activation)
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let actor_ref = self.actor_ref.clone();
        Box::pin(async move { Ok(actor_ref) })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let actor_ref = self.actor_ref.clone();
        Box::pin(async move { Ok(actor_ref) })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> NativeCapabilityFuture<'a, Option<ActorRef>> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let actor_ref = self.actor_ref.clone();
        Box::pin(async move { Ok(Some(actor_ref)) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> NativeCapabilityFuture<'a, bool> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async { Ok(true) })
    }
}

fn actor_activation() -> ActivationIdentityControl {
    ActivationIdentityControl {
        assembly_identity: AssemblyIdentity::new(
            "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        generation: 1,
        runtime_replica_id: "runtime-test".to_string(),
        deployment_revision: DeploymentRevision::new("revision-test"),
    }
}

fn actor_invocation(
    target: &'static str,
    arg_count: usize,
    return_plan: RuntimeTypePlan,
) -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        target.to_string(),
        target,
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static(target),
            (0..arg_count)
                .map(|_| scalar_plan("number", RuntimeTypeNode::Number))
                .collect(),
            return_plan,
            NativeRequiredContext::Actor,
        )),
        Some(RuntimeActorNativeMetadata::new(
            "actor:Counter".to_string(),
            "number".to_string(),
            "actor-abi".to_string(),
            "actor-implementation".to_string(),
        )),
        None,
    )
}

#[test]
fn actor_get_route_is_an_owned_external_wait() {
    let calls = Arc::new(AtomicUsize::new(0));
    let actor_ref = ActorRef::new(
        "service.test",
        "actor:Counter",
        "number",
        "skiff-canonical-v1",
        b"1".to_vec(),
        "sha256:test",
        Some(1),
    );
    let actor = TestActorCapability {
        activation: actor_activation(),
        actor_ref: actor_ref.clone(),
        calls: Arc::clone(&calls),
    };
    let mut heap = RequestHeap::default();
    let target = "std.actor.get";
    let args = vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)];
    let prepared = ActorNativeDispatch::prepare(
        actor.clone(),
        actor_invocation(target, 2, scalar_plan("unknown", RuntimeTypeNode::Unknown)),
        target.to_string(),
        args,
        &mut heap,
    )
    .unwrap_or_else(|error| panic!("{target} should prepare: {error}"));
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("{target} is an external registry operation");
    };
    let (mut wait, finalize) = operation.into_parts();
    let Poll::Ready(outcome) = poll_external_wait(&mut wait) else {
        panic!("fixture is immediately ready");
    };
    let value = finalize
        .finalize(
            outcome.unwrap_or_else(|error| panic!("{target} wait should succeed: {error}")),
            &mut heap,
        )
        .unwrap_or_else(|error| panic!("{target} should finalize: {error}"));
    assert_eq!(value, RuntimeValue::ActorRef(actor_ref.clone()));
    assert_eq!(calls.load(Ordering::Acquire), 1);
}
