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
use crate::actor_executor::ActorExecutionFrame;

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
async fn execution_lease_serializes_one_instance_but_not_another() {
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

    let first_lease = store.acquire_execution(&authority, &first).await.unwrap();
    assert!(first.instance.scheduler.try_lock().is_err());
    let second_lease = store.acquire_execution(&authority, &second).await.unwrap();
    assert!(second.instance.scheduler.try_lock().is_err());

    drop(first_lease);
    assert!(first.instance.scheduler.try_lock().is_ok());
    assert!(second.instance.scheduler.try_lock().is_err());
    drop(second_lease);
}

#[tokio::test]
async fn failed_execution_snapshot_does_not_change_live_fields() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let lease = store.acquire_execution(&authority, &handle).await.unwrap();
    let execution_fields = lease.fields();
    execution_fields.lock().unwrap()[1].value = RuntimeValue::Number(99.0);
    drop(lease);

    let count = store
        .with_fields_for_executor(&authority, &handle, |fields, _| fields[0].value.clone())
        .unwrap();
    assert_eq!(count, RuntimeValue::String("doc-1".to_string()));
}

#[tokio::test]
async fn execution_frame_rejects_wrong_field_type_and_expires_with_lease() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
    let mut heap = lease.take_heap();
    let plan = RuntimeTypePlan::from_linked(
        &builtin("integer"),
        &PlanContext::from_type_view(fixture.view(), &ExecutableAddr::service(0, 0)),
    )
    .unwrap();
    let frame = ActorExecutionFrame::new(
        store.clone(),
        handle,
        lease,
        vec![("count".to_string(), plan)],
        false,
    );
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
            &mut heap,
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
            &mut heap,
        )
        .unwrap();
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(7.0)
    );
    frame.suspend(&heap).unwrap();
    assert!(frame.read_field("count").is_err());
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
async fn upgrade_fence_allows_owned_sync_segment_to_commit_but_blocks_next_acquire() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
    lease.fields().lock().unwrap()[0].value = RuntimeValue::Number(12.0);
    let heap = lease.take_heap();

    assert!(store.begin_upgrade_exact(&handle));
    store.commit_execution(&handle, lease, heap).unwrap();
    assert!(matches!(
        store.acquire_execution(&authority, &handle).await,
        Err(ActorInstanceStoreError::InstanceReplaced)
    ));
    assert_eq!(
        store
            .with_fields_for_executor(&authority, &handle, |fields, _| { fields[0].value.clone() })
            .unwrap(),
        RuntimeValue::Number(12.0)
    );
}

#[tokio::test]
async fn suspended_continuation_cannot_resume_after_upgrade_fence() {
    let fixture = fixture();
    let bytes = payload();
    let store = ActorInstanceStore::new();
    let handle = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
    admitted(&store, &handle);
    let authority = ActorExecutorAuthority::new();
    let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
    let heap = lease.take_heap();
    store.commit_execution(&handle, lease, heap).unwrap();

    assert!(store.begin_upgrade_exact(&handle));
    assert!(matches!(
        store.acquire_execution(&authority, &handle).await,
        Err(ActorInstanceStoreError::InstanceReplaced)
    ));
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
    tracker.track("old-session", old).unwrap();

    assert_eq!(tracker.discard_session("old-session"), 1);
    assert_eq!(
        tracker
            .track("new-session", delayed_old_handle)
            .unwrap_err(),
        ActorInstanceSessionTrackError::AlreadyTracked {
            owner_session_id: "old-session".to_string()
        }
    );
    let current = store
        .activate(request(fixture.view(), fence(1), &bytes))
        .unwrap();
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
    tracker
        .track("replacement-session", replacement)
        .expect("upgrade discard releases old tracker ownership");
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
