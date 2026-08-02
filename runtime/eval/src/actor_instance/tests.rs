use std::{
    sync::{Arc, Barrier},
    thread,
};

use serde_json::json;
use skiff_artifact_model::ActorMethodIdentity;
use skiff_runtime_linked_program::{
    ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorCreateMethod,
    LinkedActorField, LinkedFileUnit, RuntimeExecutionPackage, RuntimeTypeContext,
    ServiceSymbolRef, SourceMapDto,
};

use super::*;
use crate::{actor_executor::ActorExecutionFrame, heap_access::HeapAccess};

struct ProgramFixture {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    overlay: LinkOverlay,
    types: RuntimeTypeContext,
}

impl ProgramFixture {
    fn view(&self) -> ProgramTypeView<'_> {
        ProgramTypeView::new(
            &self.service_files,
            &self.packages,
            &self.overlay,
            &self.types,
        )
    }
}

fn owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity("file:actors".to_string()),
        actor_symbol: "DocHub".to_string(),
    }
}

fn abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:doc-hub")
}

fn implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:doc-hub")
}

fn fixture() -> ProgramFixture {
    let declaration_owner = owner();
    ProgramFixture {
        service_files: vec![Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:actors".to_string(),
            source_ast_hash: "source:actors".to_string(),
            module_path: "actors".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: vec![LinkedActorDeclaration {
                actor_type: ServiceSymbolRef {
                    module_path: "actors".to_string(),
                    symbol: "DocHub".to_string(),
                },
                implementation_owner: Some(declaration_owner),
                actor_abi_identity: abi(),
                actor_implementation_identity: implementation(),
                actor_name: "DocHub".to_string(),
                actor_id_type: builtin("string"),
                key_field: "id".to_string(),
                fields: vec![
                    LinkedActorField {
                        name: "id".to_string(),
                        ty: builtin("string"),
                        encoding: ActorFieldEncodingIr::CanonicalValueV1,
                    },
                    LinkedActorField {
                        name: "count".to_string(),
                        ty: builtin("integer"),
                        encoding: ActorFieldEncodingIr::CanonicalValueV1,
                    },
                    LinkedActorField {
                        name: "title".to_string(),
                        ty: builtin("string"),
                        encoding: ActorFieldEncodingIr::CanonicalValueV1,
                    },
                ],
                create: Some(LinkedActorCreateMethod {
                    method_identity: ActorMethodIdentity::new("skiff-actor-method-v1:create"),
                    parameters: vec![
                        skiff_runtime_linked_program::LinkedFunctionTypeParamIr {
                            name: "count".to_string(),
                            ty: builtin("integer"),
                        },
                        skiff_runtime_linked_program::LinkedFunctionTypeParamIr {
                            name: "title".to_string(),
                            ty: builtin("string"),
                        },
                    ],
                    implementation:
                        skiff_runtime_linked_program::LinkedActorMethodImplementation::LocalExecutable {
                            executable_index: 0,
                        },
                }),
                public_methods: Vec::new(),
                actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
            }],
            types: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        })],
        packages: Vec::new(),
        overlay: LinkOverlay::default(),
        types: RuntimeTypeContext::default(),
    }
}

fn builtin(name: &str) -> skiff_runtime_linked_program::LinkedTypeRef {
    skiff_runtime_linked_program::LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn logical_key() -> ActorLogicalKey {
    let canonical_actor_id_key_bytes = br#""doc-1""#.to_vec();
    ActorLogicalKey {
        service_id: "skiff.run/docs".to_string(),
        actor_type_identity: "service-symbol:actors.DocHub".to_string(),
        actor_id_type_identity: "builtin:string".to_string(),
        actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
        actor_id_hash: format!(
            "sha256:{}",
            hex::encode(Sha256::digest(&canonical_actor_id_key_bytes))
        ),
        canonical_actor_id_key_bytes,
    }
}

fn fence(epoch: u64) -> ActorInstanceFence {
    ActorInstanceFence {
        incarnation: ActorIncarnationKey {
            logical_key: logical_key(),
            epoch,
        },
        actor_abi_identity: abi(),
        actor_implementation_identity: implementation(),
        declaration_owner: owner(),
    }
}

fn payload() -> Vec<u8> {
    br#"[7,"first"]"#.to_vec()
}

fn request<'a>(
    program: ProgramTypeView<'a>,
    fence: ActorInstanceFence,
    payload: &'a [u8],
) -> ActorActivationRequest<'a> {
    ActorActivationRequest {
        fence,
        bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
        bootstrap_payload: payload,
        program,
    }
}

fn admitted(store: &ActorInstanceStore, handle: &ActorInstanceHandle) {
    store
        .mark_admitted(&ActorExecutorAuthority::new(), handle)
        .expect("test instance must be admitted");
}

fn fence_for_id(id: &str, epoch: u64) -> ActorInstanceFence {
    let mut result = fence(epoch);
    let value = serde_json::to_value(id).unwrap();
    let bytes = canonical_json_bytes(&value).unwrap();
    result.incarnation.logical_key.actor_id_hash =
        format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    result.incarnation.logical_key.canonical_actor_id_key_bytes = bytes;
    result
}

#[test]
fn real_linked_declaration_materializes_key_and_unassigned_frame_in_declaration_order() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .expect("valid creation inputs materialize");

    let fields = store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
            fields.to_vec()
        })
        .unwrap();
    assert_eq!(
        fields,
        vec![
            ActorFieldValue {
                name: "id".to_string(),
                value: RuntimeValue::String("doc-1".to_string()),
                assigned: true,
            },
            ActorFieldValue {
                name: "count".to_string(),
                value: RuntimeValue::Null,
                assigned: false,
            },
            ActorFieldValue {
                name: "title".to_string(),
                value: RuntimeValue::Null,
                assigned: false,
            },
        ]
    );
}

#[test]
fn concurrent_activation_publishes_exactly_one_instance() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let barrier = Arc::new(Barrier::new(12));
    let pointers = thread::scope(|scope| {
        let joins = (0..12)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let program = fixture.view();
                let bytes = bytes.as_slice();
                scope.spawn(move || {
                    barrier.wait();
                    let handle = store
                        .activate(request(program, fence(1), bytes))
                        .expect("concurrent activation succeeds");
                    Arc::as_ptr(&handle.instance) as usize
                })
            })
            .collect::<Vec<_>>();
        joins
            .into_iter()
            .map(|join| join.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(pointers.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(store.len(), 1);
}

#[tokio::test]
async fn segment_arena_serializes_one_instance_but_not_another() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let first = store
        .activate(request(fixture.view(), fence_for_id("first", 1), &bytes))
        .unwrap();
    admitted(&store, &first);
    let second = store
        .activate(request(fixture.view(), fence_for_id("second", 1), &bytes))
        .unwrap();
    admitted(&store, &second);
    let authority = ActorExecutorAuthority::new();

    let first_segment = store.acquire_segment(&authority, &first).await.unwrap();
    assert!(first_segment.arena().clone().try_lock_owned().is_err());
    let second_segment = store.acquire_segment(&authority, &second).await.unwrap();
    assert!(second_segment.arena().clone().try_lock_owned().is_err());
    assert_eq!(store.segment_counters_for_test(&first).unwrap(), (1, 0));

    drop(first_segment);
    assert!(store.segment_counters_for_test(&first).unwrap() == (0, 0));
    assert!(second_segment.arena().clone().try_lock_owned().is_err());
    drop(second_segment);
}

#[tokio::test]
async fn abandoned_segment_releases_counters_and_leaves_arena_writes_in_place() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    segment
        .heap_mut()
        .alloc_array(vec![RuntimeValue::Number(99.0)])
        .unwrap();
    store
        .set_field_root(&handle, "count", RuntimeValue::Number(99.0))
        .unwrap();
    drop(segment);

    assert_eq!(
        store.segment_counters_for_test(&handle).unwrap(),
        (0, 0),
        "abandoning a segment must release its continuation counters"
    );
    let fields = store
        .with_fields_for_executor(&authority, &handle, |fields, heap| {
            (fields[1].value.clone(), heap.len())
        })
        .unwrap();
    assert_eq!(
        fields,
        (RuntimeValue::Number(99.0), 1),
        "already-executed arena writes stay in place after the segment ends"
    );
}

#[tokio::test]
async fn execution_frame_rejects_wrong_field_type_and_suspends() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    let mut access = HeapAccess::Shared {
        arena: segment.arena().clone(),
        guard: Some(segment.take_guard()),
    };
    let frame = ActorExecutionFrame::new(store.clone(), handle, segment, false);
    assert!(frame
        .read_field("count")
        .unwrap_err()
        .to_string()
        .contains("not assigned yet"));
    let error = frame
        .write_field(
            "count",
            &builtin("integer"),
            fixture.view(),
            &ExecutableAddr::service(0, 0),
            &RuntimeValue::String("wrong".to_string()),
            access.heap_mut(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("Actor self field count"));
    frame
        .write_field(
            "count",
            &builtin("integer"),
            fixture.view(),
            &ExecutableAddr::service(0, 0),
            &RuntimeValue::Number(7.0),
            access.heap_mut(),
        )
        .unwrap();
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(7.0)
    );
    frame.suspend().unwrap();
    assert!(frame.read_field("count").is_err());
    assert!(frame.is_suspended());
    drop(frame);
    drop(access);
}

#[test]
fn malformed_creation_inputs_fail_without_caching() {
    let fixture = fixture();
    let store = ActorInstanceStore::new();
    for malformed in [
        br#"{"count":7,"title":"first"}"#.as_slice(),
        br#"[7]"#.as_slice(),
        br#"[7,"first",true]"#.as_slice(),
        br#"not json"#.as_slice(),
    ] {
        assert!(store
            .activate(request(fixture.view(), fence(1), malformed))
            .is_err());
        assert!(store.is_empty());
    }

    let bytes = payload();
    assert!(store
        .activate(request(fixture.view(), fence(1), &bytes))
        .is_ok());
}

#[test]
fn declaration_and_identity_fences_fail_closed() {
    let fixture = fixture();
    let bytes = payload();
    let cases = [
        {
            let mut value = fence(1);
            value.actor_abi_identity = ActorAbiIdentity::new("wrong");
            value
        },
        {
            let mut value = fence(1);
            value.actor_implementation_identity = ActorImplementationIdentity::new("wrong");
            value
        },
        {
            let mut value = fence(1);
            value.declaration_owner.actor_symbol = "Other".to_string();
            value
        },
        fence(0),
    ];
    for bad_fence in cases {
        let store = ActorInstanceStore::new();
        assert!(store
            .activate(request(fixture.view(), bad_fence, &bytes))
            .is_err());
        assert!(store.is_empty());
    }

    let store = ActorInstanceStore::new();
    store
        .activate(request(fixture.view(), fence(2), &bytes))
        .unwrap();
    assert_eq!(
        store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap_err(),
        ActorInstanceStoreError::StaleEpoch {
            requested: 1,
            latest: 2
        }
    );
}

#[test]
fn existing_incarnation_rejects_different_owner_abi_or_implementation() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();

    for bad_fence in [
        {
            let mut value = fence(1);
            value.actor_abi_identity = ActorAbiIdentity::new("different");
            value
        },
        {
            let mut value = fence(1);
            value.actor_implementation_identity = ActorImplementationIdentity::new("different");
            value
        },
        {
            let mut value = fence(1);
            value.declaration_owner.file = FileAddr::FileIrIdentity("file:other".to_string());
            value
        },
    ] {
        assert_eq!(
            store
                .activate(request(fixture.view(), bad_fence, &bytes))
                .unwrap_err(),
            ActorInstanceStoreError::FenceMismatch
        );
    }
    assert_eq!(store.len(), 1);
}

#[test]
fn unknown_encoding_and_invalid_json_leave_no_partial_instance() {
    let fixture = fixture();
    let store = ActorInstanceStore::new();
    assert_eq!(
        store
            .activate(ActorActivationRequest {
                fence: fence(1),
                bootstrap_encoding_version: "unknown",
                bootstrap_payload: &payload(),
                program: fixture.view(),
            })
            .unwrap_err(),
        ActorInstanceStoreError::UnsupportedBootstrapEncoding {
            actual: "unknown".to_string()
        }
    );
    assert!(store
        .activate(request(fixture.view(), fence(1), b"{"))
        .is_err());
    assert!(store.is_empty());
}

#[test]
fn discard_requires_exact_fence_and_old_fence_cannot_remove_new_incarnation() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let old = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    let new = store
        .activate(request(fixture.view(), fence(2), &bytes))
        .unwrap();
    assert_eq!(
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &old, |fields, _| fields
                .len())
            .unwrap_err(),
        ActorInstanceStoreError::StaleEpoch {
            requested: 1,
            latest: 2
        }
    );

    let mut forged = new.clone();
    forged.fence.actor_implementation_identity = ActorImplementationIdentity::new("different");
    assert!(!store.discard_exact(&forged));
    assert!(store.discard_exact(&old));
    assert_eq!(store.len(), 1);
    assert!(!store.discard_exact(&old));
    assert!(store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &new, |fields, _| fields
            .len())
        .is_ok());
}

#[tokio::test]
async fn upgrade_fence_requires_zero_segments_and_blocks_next_acquire() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    store
        .set_field_root(&handle, "count", RuntimeValue::Number(12.0))
        .unwrap();
    assert!(
        !store.begin_upgrade_exact(&handle),
        "an active segment must gate upgrade"
    );
    segment.heap_mut().alloc_array(Vec::new()).unwrap();
    store.commit_segment(&handle, &mut segment).unwrap();
    drop(segment);
    assert_eq!(store.segment_counters_for_test(&handle).unwrap(), (0, 0));

    assert!(store.begin_upgrade_exact(&handle));
    assert!(matches!(
        store.acquire_segment(&authority, &handle).await,
        Err(ActorInstanceStoreError::InstanceReplaced)
    ));
    assert_eq!(
        store
            .with_fields_for_executor(&authority, &handle, |fields, _| { fields[0].value.clone() })
            .unwrap(),
        RuntimeValue::String("doc-1".to_string())
    );
}

#[tokio::test]
async fn suspended_continuation_gates_upgrade_until_released() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    store.suspend_segment(&handle, &mut segment).unwrap();
    assert_eq!(store.segment_counters_for_test(&handle).unwrap(), (0, 1));
    assert!(
        !store.begin_upgrade_exact(&handle),
        "a suspended continuation must gate upgrade"
    );
    store.resume_segment(&handle, &mut segment).unwrap();
    store.commit_segment(&handle, &mut segment).unwrap();

    assert!(store.begin_upgrade_exact(&handle));
    assert!(matches!(
        store.acquire_segment(&authority, &handle).await,
        Err(ActorInstanceStoreError::InstanceReplaced)
    ));
}

#[tokio::test]
async fn discard_requires_zero_segments_and_pending_mark_reclaims_on_abandon() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    store.suspend_segment(&handle, &mut segment).unwrap();

    assert!(
        !store.discard_exact(&handle),
        "a live continuation must gate discard"
    );
    assert_eq!(store.len(), 1);
    drop(segment);
    assert!(
        store.is_empty(),
        "abandoning the last segment must reclaim the pending-discard instance"
    );
}

#[tokio::test]
async fn compaction_requires_quiescence_and_no_pending_discard() {
    let fixture = fixture();
    let bytes = payload();
    let mut store = ActorInstanceStore::new();
    store.arena_limits = RequestHeapLimits {
        max_nodes: 16,
        ..RequestHeapLimits::default()
    };
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut segment = store.acquire_segment(&authority, &handle).await.unwrap();
    for _ in 0..10 {
        segment.heap_mut().alloc_array(Vec::new()).unwrap();
    }

    assert!(
        !store
            .compact_if_quiescent(&handle)
            .await
            .expect("quiescence probe must not fail"),
        "an active segment must gate compaction"
    );
    store.suspend_segment(&handle, &mut segment).unwrap();
    assert!(
        !store
            .compact_if_quiescent(&handle)
            .await
            .expect("quiescence probe must not fail"),
        "a suspended continuation must gate compaction"
    );
    store.resume_segment(&handle, &mut segment).unwrap();
    store.commit_segment(&handle, &mut segment).unwrap();
    drop(segment);

    let epoch_before = store.arena_epoch_for_test(&handle).unwrap();
    assert!(
        store
            .compact_if_quiescent(&handle)
            .await
            .expect("quiescent compaction must succeed"),
        "quiescence must allow compaction"
    );
    assert_eq!(
        store.arena_epoch_for_test(&handle).unwrap(),
        epoch_before + 1
    );
}

#[test]
fn upgrade_discard_is_exact_idempotent_and_new_epoch_rebuilds_from_bootstrap() {
    let fixture = fixture();
    let original_bootstrap = payload();
    let replacement_bootstrap = br#"[3,"replacement"]"#.to_vec();
    let store = ActorInstanceStore::new();
    let old = store
        .activate(request(fixture.view(), fence(1), &original_bootstrap))
        .unwrap();
    store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &old, |fields, _| {
            fields[0].value = RuntimeValue::Number(99.0);
        })
        .unwrap();

    let mut forged = old.clone();
    forged.fence.actor_implementation_identity = ActorImplementationIdentity::new("different");
    assert!(!store.begin_upgrade_exact(&forged));
    assert!(!store.discard_upgrading_exact(&old));
    assert!(store.begin_upgrade_exact(&old));
    assert!(store.discard_upgrading_exact(&old));
    assert!(!store.discard_upgrading_exact(&old));
    assert_eq!(
        store
            .activate(request(fixture.view(), fence(1), &original_bootstrap))
            .unwrap_err(),
        ActorInstanceStoreError::StaleEpoch {
            requested: 1,
            latest: 2
        }
    );

    let replacement = store
        .activate(request(fixture.view(), fence(2), &replacement_bootstrap))
        .unwrap();
    let fields = store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &replacement, |fields, _| {
            fields.to_vec()
        })
        .unwrap();
    assert_eq!(
        fields[0],
        ActorFieldValue {
            name: "id".to_string(),
            value: RuntimeValue::String("doc-1".to_string()),
            assigned: true,
        }
    );
    assert!(!fields[1].assigned);
    assert!(!fields[2].assigned);
}

#[test]
fn stale_cleanup_handle_cannot_remove_same_epoch_rematerialization() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let old = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    assert!(store.discard_exact(&old));
    let current = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    assert!(!store.discard_exact(&old));
    assert!(store.discard_exact(&current));
}

#[test]
fn stale_session_cleanup_cannot_remove_same_epoch_rematerialization() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    let old = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    let delayed_old_handle = old.clone();
    admitted(&store, &old);
    tracker.open_session("old-session").unwrap();
    tracker.track("old-session", old).unwrap();

    assert_eq!(tracker.discard_session("old-session"), 1);
    tracker.open_session("new-session").unwrap();
    assert!(matches!(
        tracker
            .track("new-session", delayed_old_handle)
            .unwrap_err(),
        ActorInstanceSessionTrackError::NotPublishable { .. }
    ));
    let current = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &current);
    tracker.track("new-session", current.clone()).unwrap();

    assert_eq!(tracker.discard_session("old-session"), 0);
    assert_eq!(store.len(), 1);
    assert!(store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &current, |fields, _| fields
            .len())
        .is_ok());
}

#[test]
fn session_tracker_rejects_duplicate_ownership_and_shutdown_discards_all() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    let first = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &first);
    tracker.open_session("session-a").unwrap();
    tracker.open_session("session-b").unwrap();
    tracker.track("session-a", first.clone()).unwrap();
    assert_eq!(
        tracker.track("session-b", first).unwrap_err(),
        ActorInstanceSessionTrackError::AlreadyTracked {
            owner_session_id: "session-a".to_string()
        }
    );

    let second = store
        .activate(request(fixture.view(), fence(2), &bytes))
        .unwrap();
    admitted(&store, &second);
    tracker.track("session-b", second).unwrap();
    assert_eq!(tracker.discard_all(), 2);
    assert_eq!(tracker.discard_all(), 0);
    assert!(store.is_empty());
}

#[test]
fn session_upgrade_control_is_exact_and_stale_notifications_are_inert() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    tracker.open_session("owner-session").unwrap();
    tracker.track("owner-session", handle.clone()).unwrap();

    let mut wrong_epoch = handle.fence().clone();
    wrong_epoch.incarnation.epoch = 2;
    assert!(!tracker.begin_upgrade_exact("stale-session", handle.fence()));
    assert!(!tracker.begin_upgrade_exact("owner-session", &wrong_epoch));
    assert!(tracker.begin_upgrade_exact("owner-session", handle.fence()));
    assert!(tracker.discard_upgrading_exact("owner-session", handle.fence()));
    assert!(!tracker.discard_upgrading_exact("owner-session", handle.fence()));
    assert!(!tracker.discard_upgrading_exact("stale-session", handle.fence()));
    assert!(store.is_empty());
    {
        let tracked = tracker.state.lock().unwrap();
        assert!(!tracked.by_session.contains_key("owner-session"));
        assert!(tracked.handle_owners.is_empty());
    }

    let replacement = store
        .activate(request(fixture.view(), fence(2), &bytes))
        .unwrap();
    admitted(&store, &replacement);
    tracker.open_session("replacement-session").unwrap();
    tracker
        .track("replacement-session", replacement)
        .expect("upgrade discard releases old tracker ownership");
}

#[test]
fn closed_session_rejects_late_activation_and_discards_only_the_untracked_orphan() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    tracker.open_session("closing-session").unwrap();

    let late = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    assert_eq!(tracker.discard_session("closing-session"), 0);
    assert_eq!(
        tracker.track("closing-session", late.clone()).unwrap_err(),
        ActorInstanceSessionTrackError::SessionNotOpen {
            router_session_id: "closing-session".to_string(),
        }
    );
    assert!(tracker.discard_if_untracked(&late));
    assert!(store.is_empty());
}

#[test]
fn closed_session_cleanup_never_discards_a_handle_owned_by_a_live_session() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    tracker.open_session("live-owner").unwrap();
    let shared = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &shared);
    tracker.track("live-owner", shared.clone()).unwrap();

    assert_eq!(
        tracker.track("already-closed", shared.clone()).unwrap_err(),
        ActorInstanceSessionTrackError::SessionNotOpen {
            router_session_id: "already-closed".to_string(),
        }
    );
    assert!(!tracker.discard_if_untracked(&shared));
    assert_eq!(store.len(), 1);
}

#[test]
fn evicting_last_actor_does_not_close_the_live_session() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
    tracker.open_session("live-session").unwrap();

    let first = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &first);
    tracker.track("live-session", first.clone()).unwrap();
    assert!(tracker.discard_exact("live-session", first.fence()));

    let second = store
        .activate(request(fixture.view(), fence(2), &bytes))
        .unwrap();
    admitted(&store, &second);
    tracker
        .track("live-session", second)
        .expect("evicting the last Actor must leave its Router session open");
}

#[tokio::test]
async fn session_close_before_wait_poll_is_observed_without_lost_wake() {
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = ActorInstanceSessionTracker::new(store);
    tracker.open_session("close-before-poll").unwrap();
    let lease = tracker.session_lease("close-before-poll").unwrap();
    tracker.discard_session("close-before-poll");
    tokio::time::timeout(std::time::Duration::from_secs(1), lease.wait_closed())
        .await
        .expect("a close before first wait poll must remain observable");
}

#[tokio::test]
async fn admission_before_first_notified_poll_is_observed_without_lost_wake() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    store.install_admission_wait_before_poll_test_action(
        &handle,
        AdmissionWaitBeforePollTestAction::Admit,
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.await_admission(&handle),
    )
    .await
    .expect("admission after the exact recheck but before the first await poll must wake")
    .expect("the exact Actor instance was admitted");
}

#[tokio::test]
async fn discard_before_first_notified_poll_is_observed_without_lost_wake() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    store.install_admission_wait_before_poll_test_action(
        &handle,
        AdmissionWaitBeforePollTestAction::Discard,
    );

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.await_admission(&handle),
    )
    .await
    .expect("discard after the exact recheck but before the first await poll must wake");
    assert_eq!(result, Err(ActorInstanceStoreError::InstanceNotFound));
}

#[test]
fn provisional_session_owner_blocks_cross_session_adoption_and_stale_guard_cleanup() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = Arc::new(ActorInstanceSessionTracker::new(Arc::clone(&store)));
    tracker.open_session("session-one").unwrap();
    tracker.open_session("session-two").unwrap();
    let first_lease = tracker.session_lease("session-one").unwrap();
    let second_lease = tracker.session_lease("session-two").unwrap();

    let first = tracker
        .begin_activation(&first_lease, request(fixture.view(), fence(1), &bytes))
        .expect("first session materializes with provisional ownership");
    assert!(matches!(first, ActorActivation::Materialized(_)));
    assert_eq!(store.len(), 1);
    let cross_session =
        tracker.begin_activation(&second_lease, request(fixture.view(), fence(1), &bytes));
    assert!(matches!(
        cross_session,
        Err(ActorInstanceSessionTrackError::AlreadyTracked { .. })
    ));

    assert_eq!(tracker.discard_session("session-one"), 1);
    assert!(store.is_empty());
    let second = tracker
        .begin_activation(&second_lease, request(fixture.view(), fence(1), &bytes))
        .expect("second session rematerializes only after exact first-session cleanup");
    drop(first);
    assert_eq!(
        store.len(),
        1,
        "stale first guard cannot remove replacement"
    );
    let ActorActivation::Materialized(second) = second else {
        panic!("second session must own a fresh materialization")
    };
    second
        .admit(&ActorExecutorAuthority::new())
        .expect("fresh second-session materialization admits");
    assert_eq!(store.len(), 1);
    assert!(tracker.state.lock().unwrap().handle_owners.len() == 1);
}

#[test]
fn same_id_reconnect_rejects_stale_session_generation_and_preserves_new_handle() {
    let fixture = fixture();
    let bytes = payload();
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = Arc::new(ActorInstanceSessionTracker::new(Arc::clone(&store)));
    let session_id = "reused-session-id";

    tracker.open_session(session_id).unwrap();
    let stale_lease = tracker.session_lease(session_id).unwrap();
    let stale_activation = tracker
        .begin_activation(&stale_lease, request(fixture.view(), fence(1), &bytes))
        .expect("old generation materializes with exact provisional ownership");

    assert_eq!(tracker.discard_session(session_id), 1);
    tracker.open_session(session_id).unwrap();
    let current_lease = tracker.session_lease(session_id).unwrap();
    let ActorActivation::Materialized(current_activation) = tracker
        .begin_activation(&current_lease, request(fixture.view(), fence(1), &bytes))
        .expect("new same-id generation rematerializes")
    else {
        panic!("new same-id generation must own a fresh materialization")
    };
    let current = current_activation
        .admit(&ActorExecutorAuthority::new())
        .expect("new generation admits its exact provisional handle");

    assert_eq!(
        tracker
            .track_with_lease(&stale_lease, current.clone())
            .unwrap_err(),
        ActorInstanceSessionTrackError::SessionNotOpen {
            router_session_id: session_id.to_string(),
        },
        "the old connection generation cannot publish through a reused string id"
    );
    assert!(matches!(
        tracker.begin_activation(&stale_lease, request(fixture.view(), fence(1), &bytes)),
        Err(ActorInstanceSessionTrackError::SessionNotOpen { .. })
    ));

    drop(stale_activation);
    assert_eq!(store.len(), 1, "stale cleanup cannot remove the new Arc");
    assert!(store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &current, |fields, _| fields
            .len())
        .is_ok());
}

#[test]
fn live_field_mutation_never_changes_registry_bootstrap_bytes() {
    let fixture = fixture();
    let registry_payload = payload();
    let original = registry_payload.clone();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &registry_payload))
        .unwrap();
    store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
            fields[0].value = RuntimeValue::Number(99.0);
        })
        .unwrap();
    assert_eq!(registry_payload, original);
    let count = store
        .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
            fields[0].value.clone()
        })
        .unwrap();
    assert_eq!(count, RuntimeValue::Number(99.0));
}

#[test]
fn logical_key_is_part_of_the_incarnation_identity() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    let mut other = fence(1);
    other.incarnation.logical_key.canonical_actor_id_key_bytes =
        serde_json::to_vec(&json!("doc-2")).unwrap();
    other.incarnation.logical_key.actor_id_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(
            &other.incarnation.logical_key.canonical_actor_id_key_bytes
        ))
    );
    store
        .activate(request(fixture.view(), other, &bytes))
        .unwrap();
    assert_eq!(store.len(), 2);
}

#[test]
fn empty_logical_key_components_are_rejected() {
    let fixture = fixture();
    let bytes = payload();
    let mut invalid = fence(1);
    invalid.incarnation.logical_key.service_id.clear();
    assert!(matches!(
        ActorInstanceStore::new().activate(request(fixture.view(), invalid, &bytes)),
        Err(ActorInstanceStoreError::InvalidLogicalKey { .. })
    ));
}

#[test]
fn creation_inputs_must_be_a_json_array() {
    let fixture = fixture();
    let store = ActorInstanceStore::new();
    let mut object = serde_json::Map::new();
    object.insert("title".to_string(), json!("late"));
    object.insert("count".to_string(), json!(1));
    let non_canonical = serde_json::to_vec(&Value::Object(object)).unwrap();
    assert!(matches!(
        store.activate(request(fixture.view(), fence(1), &non_canonical)),
        Err(ActorInstanceStoreError::CreationInputsNotArray)
    ));
}

#[test]
fn declaration_without_exact_owner_is_rejected() {
    let mut fixture = fixture();
    fixture.service_files[0] = Arc::new({
        let mut file = fixture.service_files[0].as_ref().clone();
        file.actor_declarations[0].implementation_owner = None;
        file
    });
    let bytes = payload();
    assert_eq!(
        ActorInstanceStore::new()
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap_err(),
        ActorInstanceStoreError::DeclarationMissing
    );
}
