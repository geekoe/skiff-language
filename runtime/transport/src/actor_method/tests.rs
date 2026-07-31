use super::*;

fn identity(prefix: &str, byte: char) -> String {
    format!("{prefix}:{}", byte.to_string().repeat(64))
}

fn actor_ref() -> ActorLogicalRefFrameHeader {
    ActorLogicalRefFrameHeader {
        service_id: "svc".into(),
        actor_type_identity: "actor-type".into(),
        actor_id_type_identity: "id-type".into(),
        actor_id_encoding_version: "v1".into(),
        canonical_actor_id_key_bytes_base64: "AQ==".into(),
        actor_id_hash: format!("sha256:{}", "d".repeat(64)),
        epoch: 7,
    }
}

fn invoke() -> ActorMethodInvokeFrameHeader {
    ActorMethodInvokeFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: "actor.method.invoke".into(),
        invocation_id: "inv:1".into(),
        actor_ref: actor_ref(),
        declaration_owner: ActorDeclarationOwnerFrameHeader {
            unit: ActorOwnerUnitFrameHeader::Service,
            file: ActorOwnerFileFrameHeader::FileIrIdentity("file:1".into()),
            actor_symbol: "Counter".into(),
        },
        actor_abi_identity: ActorAbiIdentity::new(identity("skiff-actor-abi-v1:sha256", 'a')),
        actor_implementation_identity: ActorImplementationIdentity::new(identity(
            "skiff-actor-implementation-v1:sha256",
            'b',
        )),
        method_identity: ActorMethodIdentity::new(identity("skiff-actor-method-v1:sha256", 'c')),
        arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.into(),
        deadline: ActorMethodDeadlineFrameHeader {
            timeout_ms: 100,
            expires_at: "2026-07-25T00:00:00Z".into(),
        },
        cancellation_correlation: "cancel:1".into(),
    }
}

#[test]
fn invocation_round_trips_all_identity_and_payload_fields() {
    let expected = ActorMethodFrame::Invoke(invoke(), vec![1, 2, 3]);
    let wire = encode_actor_method_frame(&expected).unwrap();
    assert_eq!(decode_actor_method_frame(&wire).unwrap(), expected);
}

#[test]
fn encoding_invalid_in_memory_headers_fails_closed() {
    let mut unsafe_epoch = invoke();
    unsafe_epoch.actor_ref.epoch = JAVASCRIPT_MAX_SAFE_INTEGER + 1;
    assert!(encode_actor_method_frame(&ActorMethodFrame::Invoke(unsafe_epoch, vec![])).is_err());

    let mut noncanonical_base64 = invoke();
    noncanonical_base64
        .actor_ref
        .canonical_actor_id_key_bytes_base64 = "AB==".into();
    assert!(
        encode_actor_method_frame(&ActorMethodFrame::Invoke(noncanonical_base64, vec![])).is_err()
    );
}

#[test]
fn missing_extra_bad_identity_and_truncated_frames_fail_closed() {
    let header = serde_json::to_value(invoke()).unwrap();
    for field in [
        "actorRef",
        "declarationOwner",
        "actorAbiIdentity",
        "actorImplementationIdentity",
        "methodIdentity",
        "invocationId",
        "deadline",
    ] {
        let mut invalid = header.clone();
        invalid.as_object_mut().unwrap().remove(field);
        let wire = encode_binary_frame(&invalid, &[]).unwrap();
        assert!(decode_actor_method_frame(&wire).is_err(), "{field}");
    }
    let mut extra = header.clone();
    extra["extra"] = Value::Bool(true);
    assert!(decode_actor_method_frame(&encode_binary_frame(&extra, &[]).unwrap()).is_err());
    let mut bad = header;
    bad["methodIdentity"] = Value::String("bad".into());
    assert!(decode_actor_method_frame(&encode_binary_frame(&bad, &[]).unwrap()).is_err());
    let mut truncated =
        encode_actor_method_frame(&ActorMethodFrame::Invoke(invoke(), vec![1])).unwrap();
    truncated.pop();
    assert!(decode_actor_method_frame(&truncated).is_err());
}

#[test]
fn typed_errors_and_cancel_remain_dedicated_frames() {
    for error in [
        ActorMethodErrorFramePayload::ActorUpgradingError {
            actor_ref: actor_ref(),
            retry_after_ms: 10,
        },
        ActorMethodErrorFramePayload::ActorVersionRejectedError {
            actor_ref: actor_ref(),
            requested_implementation_identity: ActorImplementationIdentity::new(identity(
                "skiff-actor-implementation-v1:sha256",
                'a',
            )),
            accepted_implementation_identity: ActorImplementationIdentity::new(identity(
                "skiff-actor-implementation-v1:sha256",
                'b',
            )),
        },
        ActorMethodErrorFramePayload::ActorIncarnationReplacedError {
            actor_ref: actor_ref(),
            current_epoch: 8,
        },
    ] {
        let frame = ActorMethodFrame::Error(ActorMethodErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
            envelope_type: "actor.method.error".into(),
            invocation_id: "inv:1".into(),
            error,
        });
        assert_eq!(
            decode_actor_method_frame(&encode_actor_method_frame(&frame).unwrap()).unwrap(),
            frame
        );
    }
}

#[test]
fn shared_rust_typescript_parity_corpus() {
    let corpus: Vec<Value> =
        serde_json::from_str(include_str!("../../testdata/actor-method-wire-parity.json")).unwrap();
    for case in corpus {
        let header = case["header"].clone();
        let payload = base64::engine::general_purpose::STANDARD
            .decode(case["payloadBase64"].as_str().unwrap())
            .unwrap();
        let wire = encode_binary_frame(&header, &payload).unwrap();
        assert_eq!(
            decode_actor_method_frame(&wire).is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}
