use super::*;

#[derive(Clone)]
pub(super) struct TestWebsocketCapability {
    terminal: ConnectionRequestTerminal,
    first_pending: bool,
    request_calls: Arc<AtomicUsize>,
    send_calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl NativeWebsocketCapability for TestWebsocketCapability {
    fn request_json_to_connection<'a>(
        &'a self,
        _connection_id: String,
        _method: String,
        _payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ConnectionRequestTerminal> {
        self.request_calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(PendingOnce::new(
            self.terminal.clone(),
            self.first_pending,
            Arc::clone(&self.polls),
            Arc::clone(&self.drops),
        ))
    }

    fn send_connection_text_to_business_identity(
        &self,
        _business_identity: String,
        _text: String,
    ) -> Result<()> {
        self.send_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn send_connection_binary_to_business_identity(
        &self,
        _business_identity: String,
        _payload: Vec<u8>,
    ) -> Result<()> {
        self.send_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn send_connection_text_to_connection(
        &self,
        _connection_id: String,
        _text: String,
    ) -> Result<()> {
        self.send_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn send_connection_binary_to_connection(
        &self,
        _connection_id: String,
        _payload: Vec<u8>,
    ) -> Result<()> {
        self.send_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn websocket_error_owner() -> NamedUnionOwnerIdentity {
    NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::loaded_file(0),
            type_index: 9,
        },
        type_arguments: Vec::new(),
    })
}

fn websocket_request_invocation() -> RuntimeNativeInvocation {
    let plan = NativeCallPlan::new(
        NativeBindingKey::from_static("std.websocket.requestJsonToConnection"),
        vec![
            scalar_plan("string", RuntimeTypeNode::String),
            scalar_plan("string", RuntimeTypeNode::String),
            RuntimeTypePlan::synthetic_request_record(Vec::new()),
        ],
        scalar_plan("number", RuntimeTypeNode::Number),
        NativeRequiredContext::Websocket,
    )
    .with_named_union_error_owner(websocket_error_owner())
    .expect("request binding admits exact owner");
    RuntimeNativeInvocation::new(
        "std.websocket.requestJsonToConnection".to_string(),
        "std.websocket.requestJsonToConnection",
        Some(plan),
        None,
        None,
    )
}

#[test]
fn prepared_websocket_request_wait_is_heap_free_and_starts_once() {
    let request_calls = Arc::new(AtomicUsize::new(0));
    let send_calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let websocket = TestWebsocketCapability {
        terminal: ConnectionRequestTerminal::Success(b"11".to_vec()),
        first_pending: true,
        request_calls: Arc::clone(&request_calls),
        send_calls,
        polls: Arc::clone(&polls),
        drops,
    };
    let mut heap = RequestHeap::default();
    let payload = RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(Default::default()))
            .expect("request payload"),
    );
    let prepared = WebsocketNativeDispatch::prepare(
        websocket,
        websocket_request_invocation(),
        "std.websocket.requestJsonToConnection".to_string(),
        vec![
            RuntimeValue::String("connection-1".to_string()),
            RuntimeValue::String("status.get".to_string()),
            payload,
        ],
        &mut heap,
    )
    .expect("WebSocket request should prepare");
    heap.alloc_bytes(b"caller remains mutable".to_vec())
        .expect("request wait does not borrow caller heap");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("requestJsonToConnection must expose a wait");
    };
    let (mut wait, finalize) = operation.into_parts();
    assert!(matches!(poll_external_wait(&mut wait), Poll::Pending));
    assert_eq!(request_calls.load(Ordering::Acquire), 1);
    let Poll::Ready(outcome) = poll_external_wait(&mut wait) else {
        panic!("second poll completes the request fixture");
    };
    assert_eq!(request_calls.load(Ordering::Acquire), 1);
    assert_eq!(polls.load(Ordering::Acquire), 2);
    assert_eq!(
        finalize
            .finalize(outcome.expect("request succeeds"), &mut heap)
            .expect("request response finalizes"),
        RuntimeValue::Number(11.0)
    );
}

#[test]
fn all_four_websocket_sends_are_synchronous_ready_calls() {
    let send_calls = Arc::new(AtomicUsize::new(0));
    let websocket = TestWebsocketCapability {
        terminal: ConnectionRequestTerminal::ProtocolError,
        first_pending: false,
        request_calls: Arc::new(AtomicUsize::new(0)),
        send_calls: Arc::clone(&send_calls),
        polls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
    };
    let mut heap = RequestHeap::default();
    for (target, binary) in [
        ("std.websocket.sendTextToBusinessIdentity", false),
        ("std.websocket.sendBinaryToBusinessIdentity", true),
        ("std.websocket.sendTextToConnection", false),
        ("std.websocket.sendBinaryToConnection", true),
    ] {
        let payload_plan = if binary {
            scalar_plan("bytes", RuntimeTypeNode::Bytes)
        } else {
            scalar_plan("string", RuntimeTypeNode::String)
        };
        let invocation = RuntimeNativeInvocation::new(
            target.to_string(),
            target,
            Some(NativeCallPlan::new(
                NativeBindingKey::from_static(target),
                vec![scalar_plan("string", RuntimeTypeNode::String), payload_plan],
                scalar_plan("null", RuntimeTypeNode::Null),
                NativeRequiredContext::Websocket,
            )),
            None,
            None,
        );
        let payload = if binary {
            RuntimeValue::Heap(
                heap.alloc_bytes(b"payload".to_vec())
                    .expect("binary payload"),
            )
        } else {
            RuntimeValue::String("payload".to_string())
        };
        let prepared = WebsocketNativeDispatch::prepare(
            websocket.clone(),
            invocation,
            target.to_string(),
            vec![RuntimeValue::String("destination".to_string()), payload],
            &mut heap,
        )
        .unwrap_or_else(|error| panic!("{target} should prepare: {error}"));
        assert!(
            matches!(prepared, PreparedNativeCall::Ready(RuntimeValue::Null)),
            "{target} must not be wrapped as an external wait"
        );
    }
    assert_eq!(send_calls.load(Ordering::Acquire), 4);
}
