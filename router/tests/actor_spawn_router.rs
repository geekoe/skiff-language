//! `SpawnSubmitRouter` sequence tests: the ten frozen spawn-wire parent
//! scenarios (collision / parent terminal / replacement / connection /
//! authority / legacy-cut) driven through the real stateless router, plus
//! canonical-codec corpus consumption and the W-dispatch actor lane seam.

mod actor_support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use skiff_router::actor::{
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, ActorLaneSpawnControl,
    ActorMethodSpawnExecutionSink, ActorSpawnParentResolver, FunctionSpawnParentResolver,
    ParentQuery, RelaySpawnParentLookup, SpawnAuthorityProbe, SpawnErrorCode, SpawnParentLookup,
    SpawnParentSnapshot, SpawnSubmitAcceptance, SpawnSubmitError, SpawnSubmitRouter,
};
use skiff_router::dispatch::{ActorMethodSpawnControl, ActorMethodSpawnDispatch};
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_request_frame, encode_spawn_submit_error_frame,
    encode_spawn_submit_request_frame, encode_spawn_submit_response_frame,
    SpawnActorMethodTargetFrameMetadata, SpawnCallerKind, SpawnSubmitRequestFrameHeaderV2,
    SpawnTargetKind,
};

use actor_support::{
    abi, activation_identity_wire, actor_implementation_identity, actor_ref_wire,
    declaration_owner, hex_bytes, method_identity, route_authority, spawn_wire_dir,
};

const REQUIRED_SCENARIOS: [&str; 10] = [
    "resolve-function-parent-exact",
    "resolve-actor-invocation-parent-exact",
    "same-request-id-both-namespaces-no-collision",
    "missing-caller-kind-legacy-cut-rejected",
    "parent-terminal-before-submit-rejected",
    "parent-replaced-before-submit-rejected",
    "parent-connection-mismatch-rejected",
    "authority-mismatch-rejected",
    "accepted-spawn-outlives-parent-terminal",
    "target-kind-mismatch-rejected",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentJson {
    id: String,
    runtime_id: String,
    connection: String,
    assembly_generation: u64,
    test_case_capability: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentsJson {
    request: Vec<ParentJson>,
    #[serde(rename = "actorInvocation")]
    actor_invocation: Vec<ParentJson>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitEvent {
    op: String,
    #[serde(default)]
    legacy: bool,
    #[serde(default)]
    caller_kind: Option<String>,
    #[serde(default)]
    caller_request_id: Option<String>,
    #[serde(default)]
    target_kind: Option<String>,
    #[serde(default)]
    actor_method: bool,
    #[serde(default)]
    connection: Option<String>,
    #[serde(default)]
    assembly_generation: Option<u64>,
    #[serde(default)]
    test_case_capability: Option<String>,
    #[serde(default)]
    new_connection: Option<String>,
    #[serde(default)]
    new_runtime_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnScenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    parents: ParentsJson,
    events: Vec<SubmitEvent>,
    expect: SpawnExpect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnExpect {
    accepted: Vec<String>,
    rejected: Vec<String>,
    errors: BTreeMap<String, String>,
    #[serde(default)]
    accepted_spawns: u64,
}

#[derive(Debug, Default, Clone)]
struct FakeParentStore(Arc<Mutex<HashMap<String, SpawnParentSnapshot>>>);

impl FakeParentStore {
    fn insert(&self, parent: &ParentJson) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                parent.id.clone(),
                SpawnParentSnapshot {
                    runtime_id: parent.runtime_id.clone(),
                    connection: parent.connection.clone(),
                    assembly_generation: parent.assembly_generation,
                    test_case_capability: parent.test_case_capability.clone(),
                    active: true,
                    replaced: false,
                },
            );
    }

    fn with_parent(id: &str, snapshot: SpawnParentSnapshot) -> Self {
        let store = Self::default();
        store
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.to_string(), snapshot);
        store
    }

    fn mutate(&self, id: &str, mutate: impl FnOnce(&mut SpawnParentSnapshot)) {
        let mut parents = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = parents.get_mut(id).expect("parent exists");
        mutate(snapshot);
    }

    fn snapshot(&self, id: &str) -> Option<SpawnParentSnapshot> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
    }
}

impl SpawnParentLookup for FakeParentStore {
    fn find_parent(&self, caller_request_id: &str) -> Option<SpawnParentSnapshot> {
        self.snapshot(caller_request_id)
    }
}

fn actor_method_metadata() -> SpawnActorMethodTargetFrameMetadata {
    SpawnActorMethodTargetFrameMetadata {
        actor_ref: actor_ref_wire(7),
        declaration_owner: declaration_owner(),
        actor_abi_identity: abi(),
        actor_implementation_identity: actor_implementation_identity(),
        method_identity: method_identity(),
    }
}

fn spawn_header(
    caller_kind: SpawnCallerKind,
    caller_request_id: &str,
    target_kind: SpawnTargetKind,
    with_actor_method: bool,
    spawn_id: Option<&str>,
) -> SpawnSubmitRequestFrameHeaderV2 {
    SpawnSubmitRequestFrameHeaderV2 {
        schema_version: "skiff-runtime-frame-v3".to_string(),
        envelope_type: "spawn.submit.request".to_string(),
        rpc_id: format!("rpc:{caller_request_id}"),
        runtime_id: "runtime-a".to_string(),
        caller_kind,
        caller_request_id: caller_request_id.to_string(),
        target_kind,
        service_id: "example.com/docs".to_string(),
        service_version: "1.0.0".to_string(),
        service_protocol_identity: "example.com/docs:1.0.0".to_string(),
        target: "example.com/fn".to_string(),
        spawn_id: spawn_id.map(str::to_string),
        build_id: None,
        activation_identity: activation_identity_wire("runtime-a"),
        trace_id: None,
        caller_target: None,
        max_queue_wait_ms: None,
        actor_method: with_actor_method.then(actor_method_metadata),
    }
}

fn probe(event: &SubmitEvent) -> SpawnAuthorityProbe {
    SpawnAuthorityProbe {
        connection: event.connection.clone().unwrap_or_default(),
        runtime_id: Some("runtime-a".to_string()),
        assembly_generation: event.assembly_generation.unwrap_or(42),
        test_case_capability: event.test_case_capability.clone(),
    }
}

fn run_spawn_scenario(raw: &str) {
    let scenario: SpawnScenario = serde_json::from_str(raw).expect("spawn scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let request_store = FakeParentStore::default();
    let actor_store = FakeParentStore::default();
    for parent in scenario.parents.request {
        request_store.insert(&parent);
    }
    for parent in scenario.parents.actor_invocation {
        actor_store.insert(&parent);
    }
    let router = SpawnSubmitRouter::new(
        Arc::new(FunctionSpawnParentResolver::new(Arc::new(
            request_store.clone(),
        ))),
        Arc::new(ActorSpawnParentResolver::new(Arc::new(actor_store.clone()))),
        64,
    )
    .expect("router");
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut errors = BTreeMap::new();
    for event in &scenario.events {
        match event.op.as_str() {
            "submit" => {
                let key = event_key(event);
                let result = if event.legacy || event.caller_kind.is_none() {
                    Err(router.reject_legacy(event.caller_request_id.as_deref().unwrap_or("?")))
                } else {
                    let caller_kind = match event.caller_kind.as_deref().expect("callerKind") {
                        "request" => SpawnCallerKind::Request,
                        "actorInvocation" => SpawnCallerKind::ActorInvocation,
                        other => panic!("invalid callerKind {other} in corpus"),
                    };
                    let target_kind = match event.target_kind.as_deref().expect("targetKind") {
                        "function" => SpawnTargetKind::Function,
                        "actorMethod" => SpawnTargetKind::ActorMethod,
                        other => panic!("invalid targetKind {other} in corpus"),
                    };
                    let header = spawn_header(
                        caller_kind,
                        event.caller_request_id.as_deref().expect("callerRequestId"),
                        target_kind,
                        event.actor_method,
                        None,
                    );
                    router.submit(&header, &probe(event))
                };
                match result {
                    Ok(_acceptance) => accepted.push(key),
                    Err(error) => {
                        rejected.push(key.clone());
                        errors.insert(key, error.to_string());
                    }
                }
            }
            "parentTerminal" => {
                mark_parent(&request_store, &actor_store, event, |snapshot| {
                    snapshot.active = false
                });
            }
            "replace" => {
                mark_parent(&request_store, &actor_store, event, |snapshot| {
                    snapshot.replaced = true;
                    if let Some(connection) = &event.new_connection {
                        snapshot.connection = connection.clone();
                    }
                    if let Some(runtime_id) = &event.new_runtime_id {
                        snapshot.runtime_id = runtime_id.clone();
                    }
                });
            }
            other => panic!("unknown spawn scenario op {other}"),
        }
    }
    assert_eq!(accepted, scenario.expect.accepted, "accepted");
    assert_eq!(rejected, scenario.expect.rejected, "rejected");
    assert_eq!(errors, scenario.expect.errors, "errors");
    assert_eq!(
        router.health().accepted,
        scenario.expect.accepted_spawns,
        "acceptedSpawns"
    );
}

fn event_key(event: &SubmitEvent) -> String {
    if event.legacy || event.caller_kind.is_none() {
        format!(
            "legacy:{}",
            event.caller_request_id.as_deref().unwrap_or("?")
        )
    } else {
        format!(
            "{}:{}",
            event.caller_kind.as_deref().unwrap_or("?"),
            event.caller_request_id.as_deref().unwrap_or("?")
        )
    }
}

fn mark_parent(
    request_store: &FakeParentStore,
    actor_store: &FakeParentStore,
    event: &SubmitEvent,
    mutate: impl Fn(&mut SpawnParentSnapshot),
) {
    let caller_kind = event.caller_kind.as_deref().expect("callerKind");
    let id = event.caller_request_id.as_deref().expect("callerRequestId");
    let store = match caller_kind {
        "request" => request_store,
        "actorInvocation" => actor_store,
        other => panic!("invalid callerKind {other}"),
    };
    store.mutate(id, mutate);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_scenarios_drive_the_real_stateless_router() {
        let dir = spawn_wire_dir();
        for (prefix, name) in [
            ("01", "resolve-function-parent-exact"),
            ("02", "resolve-actor-invocation-parent-exact"),
            ("03", "same-request-id-both-namespaces-no-collision"),
            ("04", "missing-caller-kind-legacy-cut-rejected"),
            ("05", "parent-terminal-before-submit-rejected"),
            ("06", "parent-replaced-before-submit-rejected"),
            ("07", "parent-connection-mismatch-rejected"),
            ("08", "authority-mismatch-rejected"),
            ("09", "accepted-spawn-outlives-parent-terminal"),
            ("10", "target-kind-mismatch-rejected"),
        ] {
            let raw = std::fs::read_to_string(
                dir.join("scenarios").join(format!("{prefix}-{name}.json")),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_spawn_scenario(&raw);
        }
    }

    #[test]
    fn canonical_codec_frames_drive_the_router() {
        let catalog: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(spawn_wire_dir().join("frames.json")).expect("frames"),
        )
        .expect("frames decode");
        let request_store = FakeParentStore::with_parent(
            "parent-1",
            SpawnParentSnapshot {
                runtime_id: "runtime-a".to_string(),
                connection: "conn-a".to_string(),
                assembly_generation: 42,
                test_case_capability: None,
                active: true,
                replaced: false,
            },
        );
        let router = SpawnSubmitRouter::new(
            Arc::new(FunctionSpawnParentResolver::new(Arc::new(request_store))),
            Arc::new(ActorSpawnParentResolver::new(Arc::new(
                FakeParentStore::default(),
            ))),
            64,
        )
        .expect("router");

        let function_hex = catalog["frames"]["spawn.submit.request.function"]["frameHex"]
            .as_str()
            .expect("function frame");
        let (header, payload) = decode_spawn_submit_request_frame(&hex_bytes(function_hex))
            .expect("function frame decode");
        assert_eq!(
            encode_spawn_submit_request_frame(&header, &payload).expect("re-encode"),
            hex_bytes(function_hex),
            "function frame must be byte-exact"
        );
        let acceptance = router
            .submit(
                &header,
                &SpawnAuthorityProbe {
                    connection: "conn-a".to_string(),
                    runtime_id: Some("runtime-a".to_string()),
                    assembly_generation: 42,
                    test_case_capability: None,
                },
            )
            .expect("canonical function submit accepted");
        assert_eq!(acceptance.caller_kind, SpawnCallerKind::Request);
        assert_eq!(acceptance.target_kind, SpawnTargetKind::Function);

        let legacy_hex = catalog["frames"]["spawn.submit.request.legacy-no-caller-kind"]
            ["frameHex"]
            .as_str()
            .expect("legacy frame");
        let error = decode_spawn_submit_request_frame(&hex_bytes(legacy_hex))
            .expect_err("legacy old shape must be rejected");
        assert!(error.to_string().contains("callerKind"));

        let response_hex = catalog["frames"]["spawn.submit.response"]["frameHex"]
            .as_str()
            .expect("response frame");
        let response = skiff_runtime_transport::protocol::decode_spawn_submit_response_frame(
            &hex_bytes(response_hex),
        )
        .expect("response decode");
        assert_eq!(
            encode_spawn_submit_response_frame(&response).expect("response re-encode"),
            hex_bytes(response_hex)
        );

        let error_hex = catalog["frames"]["spawn.submit.error.parentNotFound"]["frameHex"]
            .as_str()
            .expect("error frame");
        let spawn_error = skiff_runtime_transport::protocol::decode_spawn_submit_error_frame(
            &hex_bytes(error_hex),
        )
        .expect("error decode");
        assert_eq!(spawn_error.error.code, "ParentNotFound");
        assert_eq!(
            encode_spawn_submit_error_frame(&spawn_error).expect("error re-encode"),
            hex_bytes(error_hex)
        );

        router.release_accepted();
        assert_eq!(router.health().capacity_in_use, 0);
    }

    #[test]
    fn saturation_rejects_without_leaking_capacity() {
        let request_store = FakeParentStore::with_parent(
            "parent-1",
            SpawnParentSnapshot {
                runtime_id: "runtime-a".to_string(),
                connection: "conn-a".to_string(),
                assembly_generation: 42,
                test_case_capability: None,
                active: true,
                replaced: false,
            },
        );
        let router = SpawnSubmitRouter::new(
            Arc::new(FunctionSpawnParentResolver::new(Arc::new(request_store))),
            Arc::new(ActorSpawnParentResolver::new(Arc::new(
                FakeParentStore::default(),
            ))),
            1,
        )
        .expect("router");
        let header = spawn_header(
            SpawnCallerKind::Request,
            "parent-1",
            SpawnTargetKind::Function,
            false,
            None,
        );
        router
            .submit(
                &header,
                &SpawnAuthorityProbe {
                    connection: "conn-a".to_string(),
                    runtime_id: Some("runtime-a".to_string()),
                    assembly_generation: 42,
                    test_case_capability: None,
                },
            )
            .expect("first submit");
        let error = router
            .submit(
                &header,
                &SpawnAuthorityProbe {
                    connection: "conn-a".to_string(),
                    runtime_id: Some("runtime-a".to_string()),
                    assembly_generation: 42,
                    test_case_capability: None,
                },
            )
            .expect_err("second submit must be saturated");
        assert_eq!(error.code(), SpawnErrorCode::Saturated);
        assert_eq!(router.health().capacity_in_use, 1);
        router.release_accepted();
        assert_eq!(router.health().capacity_in_use, 0);
    }

    #[test]
    fn actor_lane_seam_resolves_exact_parents_and_hands_accepted_spawns_over() {
        #[derive(Debug, Default)]
        struct Sink {
            accepted: Mutex<Vec<SpawnSubmitAcceptance>>,
        }
        impl ActorMethodSpawnExecutionSink for Sink {
            fn on_accept(&self, acceptance: &SpawnSubmitAcceptance) {
                self.accepted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(acceptance.clone());
            }
        }

        let relay = Arc::new(ActorInvocationRelay::new(
            ActorInvocationRelayOptions::default(),
        ));
        relay
            .invoke(&ActorInvokeInput {
                invocation_id: "inv-parent-1".to_string(),
                caller_connection: "conn-a".to_string(),
                caller_runtime_id: "runtime-a".to_string(),
                owner_fence: actor_support::fence("runtime-b", 7, 40_000),
                owner_connection: "conn-b".to_string(),
                route_authority: route_authority(),
                correlation: "cancel:1".to_string(),
                deadline: None,
                test_case_capability: None,
                now: 0,
            })
            .expect("parent invoke");
        let router = Arc::new(
            SpawnSubmitRouter::new(
                Arc::new(FunctionSpawnParentResolver::new(Arc::new(
                    FakeParentStore::default(),
                ))),
                Arc::new(ActorSpawnParentResolver::new(Arc::new(
                    RelaySpawnParentLookup::new(Arc::clone(&relay)),
                ))),
                64,
            )
            .expect("router"),
        );
        let sink = Arc::new(Sink::default());
        let lane =
            ActorLaneSpawnControl::new(Arc::clone(&relay), Arc::clone(&router), sink.clone());
        assert!(lane.is_active_invocation_parent("inv-parent-1"));
        assert!(!lane.is_active_invocation_parent("missing"));

        let dispatch = ActorMethodSpawnDispatch {
            spawn_request_id: "rpc:spawn-1".to_string(),
            caller_request_id: "inv-parent-1".to_string(),
            target: "example.com/fn".to_string(),
        };
        lane.submit_spawn(dispatch);
        let accepted = sink
            .accepted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].caller_kind, SpawnCallerKind::ActorInvocation);
        assert_eq!(accepted[0].parent_request_id, "inv-parent-1");
        assert_eq!(router.health().actor_invocation_accepted, 1);

        // Parent terminal does not affect the already accepted spawn; the
        // execution sink already owns it and the router has no mapping.
        relay
            .on_owner_settle(
                "inv-parent-1",
                &actor_support::fence("runtime-b", 7, 40_000),
                "conn-b",
                skiff_router::actor::OwnerSettleKind::Return,
            )
            .expect("parent settles");
        assert!(!lane.is_active_invocation_parent("inv-parent-1"));
        let dispatch = ActorMethodSpawnDispatch {
            spawn_request_id: "rpc:spawn-2".to_string(),
            caller_request_id: "inv-parent-1".to_string(),
            target: "example.com/fn".to_string(),
        };
        lane.submit_spawn(dispatch);
        assert_eq!(
            router.health().by_error.get("ParentTerminal"),
            Some(&1),
            "terminal parent submit must fail closed"
        );
        assert_eq!(
            sink.accepted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );
        router.release_accepted();
        assert_eq!(router.health().capacity_in_use, 0);
    }

    #[test]
    fn resolver_checks_are_exact_and_fail_closed() {
        let store = FakeParentStore::with_parent(
            "parent-1",
            SpawnParentSnapshot {
                runtime_id: "runtime-a".to_string(),
                connection: "conn-a".to_string(),
                assembly_generation: 42,
                test_case_capability: Some("test:cap-1".to_string()),
                active: true,
                replaced: false,
            },
        );
        let resolver = FunctionSpawnParentResolver::new(Arc::new(store));
        let query = ParentQuery {
            caller_request_id: "parent-1".to_string(),
            connection: "conn-a".to_string(),
            runtime_id: Some("runtime-a".to_string()),
            assembly_generation: 42,
            test_case_capability: Some("test:cap-1".to_string()),
        };
        let resolution = resolver.resolve(&query).expect("exact resolution");
        assert_eq!(resolution.origin_runtime_connection, "conn-a");

        let mismatch = ParentQuery {
            test_case_capability: None,
            ..query.clone()
        };
        assert_eq!(
            resolver
                .resolve(&mismatch)
                .expect_err("test capability drift"),
            SpawnSubmitError::new(SpawnErrorCode::TestCapabilityMismatch)
        );
        let authority = ParentQuery {
            assembly_generation: 41,
            ..query
        };
        assert_eq!(
            resolver.resolve(&authority).expect_err("authority drift"),
            SpawnSubmitError::new(SpawnErrorCode::AuthorityMismatch)
        );
    }

    #[test]
    fn scenario_names_are_frozen() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(spawn_wire_dir().join("scenarios")).expect("scenarios dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            names.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "required spawn scenario {required} is missing"
            );
        }
    }

    #[test]
    fn error_code_vocabulary_is_frozen() {
        let codes = SpawnErrorCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            vec![
                "ParentNotFound",
                "ParentTerminal",
                "ParentReplaced",
                "ParentConnectionMismatch",
                "CallerKindRejected",
                "TargetKindMismatch",
                "AuthorityMismatch",
                "TestCapabilityMismatch",
                "Saturated",
                "UnknownTarget",
            ]
        );
    }
}
