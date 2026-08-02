//! Spawn-family byte-exact wire corpus + resolver/router reference model for
//! C-model-spawn / C-spawn
//! (`doc/implementation/router-rust-migration-c-model-spawn-contract.md`,
//! `doc/implementation/router-rust-migration-c-spawn-contract.md`).
//!
//! The target wire freezes the explicit closed enum
//! `callerKind = request | actorInvocation` (plan §5.3) with a typed parent
//! namespace and no string-prefix fallback. The old shape (no `callerKind`)
//! is `legacyCut` and must be rejected with no compatible reader. The frame
//! hexes are frozen from the target mirror (C-model-spawn); since W-model-spawn
//! the production canonical codec carries `callerKind` and takes over the same
//! corpus byte-exactly. `H-spawn-parent-cut` is exactly the later change that
//! switches the production consumers and deletes the old shape.

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_error_frame, decode_spawn_submit_request_frame,
    decode_spawn_submit_response_frame, encode_spawn_submit_error_frame,
    encode_spawn_submit_request_frame, encode_spawn_submit_response_frame,
    SpawnSubmitRequestFrameHeaderV2,
};

const REQUIRED_FRAMES: [&str; 5] = [
    "spawn.submit.request.function",
    "spawn.submit.request.actorMethod",
    "spawn.submit.request.legacy-no-caller-kind",
    "spawn.submit.response",
    "spawn.submit.error.parentNotFound",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "payloadBase64")]
    payload_base64: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    #[serde(rename = "legacyCut")]
    legacy_cut: bool,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn catalog() -> Catalog {
    serde_json::from_str(include_str!("../testdata/spawn-wire/frames.json"))
        .expect("spawn wire corpus must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("frameHex hex"))
        .collect()
}

fn payload_of(entry: &FrameEntry) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(&entry.payload_base64)
        .expect("payloadBase64 must be canonical base64")
}

// ---------------------------------------------------------------------------
// Resolver / router reference model (C-spawn)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ParentRecord {
    runtime_id: String,
    connection: String,
    assembly_generation: u64,
    test_case_capability: Option<String>,
    active: bool,
    replaced: bool,
}

#[derive(Debug, Default)]
struct ParentStores {
    request: HashMap<String, ParentRecord>,
    actor_invocation: HashMap<String, ParentRecord>,
}

impl ParentStores {
    fn get(&self, caller_kind: &str, id: &str) -> Option<&ParentRecord> {
        match caller_kind {
            "request" => self.request.get(id),
            "actorInvocation" => self.actor_invocation.get(id),
            _ => None,
        }
    }

    fn get_mut(&mut self, caller_kind: &str, id: &str) -> Option<&mut ParentRecord> {
        match caller_kind {
            "request" => self.request.get_mut(id),
            "actorInvocation" => self.actor_invocation.get_mut(id),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct SpawnSubmitRouter {
    accepted_spawns: usize,
}

impl SpawnSubmitRouter {
    fn submit(
        &mut self,
        stores: &ParentStores,
        event: &SubmitEvent,
    ) -> Result<String, &'static str> {
        let key = if event.legacy {
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
        };
        if event.legacy || event.caller_kind.is_none() {
            return Err("CallerKindRejected");
        }
        let caller_kind = event.caller_kind.as_deref().expect("callerKind");
        if caller_kind != "request" && caller_kind != "actorInvocation" {
            return Err("CallerKindRejected");
        }
        let parent = stores
            .get(
                caller_kind,
                event.caller_request_id.as_deref().expect("callerRequestId"),
            )
            .ok_or("ParentNotFound")?;
        if !parent.active {
            return Err("ParentTerminal");
        }
        if parent.replaced {
            return Err("ParentReplaced");
        }
        if parent.connection != event.connection.as_deref().unwrap_or("") {
            return Err("ParentConnectionMismatch");
        }
        if parent.test_case_capability.is_some()
            && event.test_case_capability != parent.test_case_capability
        {
            return Err("TestCapabilityMismatch");
        }
        if let Some(generation) = event.assembly_generation {
            if generation != parent.assembly_generation {
                return Err("AuthorityMismatch");
            }
        }
        let target_kind = event.target_kind.as_deref().ok_or("TargetKindMismatch")?;
        if target_kind == "actorMethod" && !event.actor_method {
            return Err("TargetKindMismatch");
        }
        if target_kind == "function" && event.actor_method {
            return Err("TargetKindMismatch");
        }
        self.accepted_spawns += 1;
        Ok(key)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    accepted_spawns: usize,
}

fn run_spawn_scenario(expected_name: &str, raw: &str) {
    let scenario: SpawnScenario = serde_json::from_str(raw).expect("spawn scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.scenario, expected_name);
    let mut stores = ParentStores::default();
    for parent in scenario.parents.request {
        stores.request.insert(
            parent.id.clone(),
            ParentRecord {
                runtime_id: parent.runtime_id,
                connection: parent.connection,
                assembly_generation: parent.assembly_generation,
                test_case_capability: parent.test_case_capability,
                active: true,
                replaced: false,
            },
        );
    }
    for parent in scenario.parents.actor_invocation {
        stores.actor_invocation.insert(
            parent.id.clone(),
            ParentRecord {
                runtime_id: parent.runtime_id,
                connection: parent.connection,
                assembly_generation: parent.assembly_generation,
                test_case_capability: parent.test_case_capability,
                active: true,
                replaced: false,
            },
        );
    }
    let mut router = SpawnSubmitRouter::default();
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut errors = BTreeMap::new();
    for event in &scenario.events {
        match event.op.as_str() {
            "submit" => {
                let key = if event.legacy {
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
                };
                match router.submit(&stores, event) {
                    Ok(_) => accepted.push(key),
                    Err(error) => {
                        rejected.push(key.clone());
                        errors.insert(key, error.to_string());
                    }
                }
            }
            "parentTerminal" => {
                let record = stores
                    .get_mut(
                        event
                            .caller_kind
                            .as_deref()
                            .expect("parentTerminal callerKind"),
                        event
                            .caller_request_id
                            .as_deref()
                            .expect("parentTerminal callerRequestId"),
                    )
                    .expect("parentTerminal parent exists");
                record.active = false;
            }
            "replace" => {
                let record = stores
                    .get_mut(
                        event.caller_kind.as_deref().expect("replace callerKind"),
                        event
                            .caller_request_id
                            .as_deref()
                            .expect("replace callerRequestId"),
                    )
                    .expect("replace parent exists");
                record.replaced = true;
                if let Some(connection) = &event.new_connection {
                    record.connection = connection.clone();
                }
                if let Some(runtime_id) = &event.new_runtime_id {
                    record.runtime_id = runtime_id.clone();
                }
            }
            other => panic!("unknown spawn scenario op {other}"),
        }
    }
    assert_eq!(accepted, scenario.expect.accepted, "accepted");
    assert_eq!(rejected, scenario.expect.rejected, "rejected");
    assert_eq!(errors, scenario.expect.errors, "errors");
    assert_eq!(
        router.accepted_spawns, scenario.expect.accepted_spawns,
        "acceptedSpawns"
    );
}

const SPAWN_SCENARIOS: [(&str, &str); 10] = [
    (
        "resolve-function-parent-exact",
        include_str!("../testdata/spawn-wire/scenarios/01-resolve-function-parent-exact.json"),
    ),
    (
        "resolve-actor-invocation-parent-exact",
        include_str!(
            "../testdata/spawn-wire/scenarios/02-resolve-actor-invocation-parent-exact.json"
        ),
    ),
    (
        "same-request-id-both-namespaces-no-collision",
        include_str!(
            "../testdata/spawn-wire/scenarios/03-same-request-id-both-namespaces-no-collision.json"
        ),
    ),
    (
        "missing-caller-kind-legacy-cut-rejected",
        include_str!(
            "../testdata/spawn-wire/scenarios/04-missing-caller-kind-legacy-cut-rejected.json"
        ),
    ),
    (
        "parent-terminal-before-submit-rejected",
        include_str!(
            "../testdata/spawn-wire/scenarios/05-parent-terminal-before-submit-rejected.json"
        ),
    ),
    (
        "parent-replaced-before-submit-rejected",
        include_str!(
            "../testdata/spawn-wire/scenarios/06-parent-replaced-before-submit-rejected.json"
        ),
    ),
    (
        "parent-connection-mismatch-rejected",
        include_str!(
            "../testdata/spawn-wire/scenarios/07-parent-connection-mismatch-rejected.json"
        ),
    ),
    (
        "authority-mismatch-rejected",
        include_str!("../testdata/spawn-wire/scenarios/08-authority-mismatch-rejected.json"),
    ),
    (
        "accepted-spawn-outlives-parent-terminal",
        include_str!(
            "../testdata/spawn-wire/scenarios/09-accepted-spawn-outlives-parent-terminal.json"
        ),
    ),
    (
        "target-kind-mismatch-rejected",
        include_str!("../testdata/spawn-wire/scenarios/10-target-kind-mismatch-rejected.json"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_required_frames_are_frozen() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "spawn-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "required spawn frame {required} is missing"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        let legacy = &catalog.frames["spawn.submit.request.legacy-no-caller-kind"];
        assert!(
            legacy.legacy_cut,
            "legacy old-shape frame must be legacyCut"
        );
    }

    #[test]
    fn spawn_family_rule_is_mixed_direction_with_required_payload_and_frame_table() {
        use skiff_runtime_transport::protocol::spawn_submit_frame_direction;
        use skiff_runtime_transport::protocol::{
            FrameDirection, PayloadPresenceRule, RuntimeFrameFamily, SPAWN_SUBMIT_ERROR_FRAME_TYPE,
            SPAWN_SUBMIT_REQUEST_FRAME_TYPE, SPAWN_SUBMIT_RESPONSE_FRAME_TYPE,
        };
        assert_eq!(
            RuntimeFrameFamily::Spawn.direction(),
            FrameDirection::Either,
            "spawn family is mixed-direction; consumers narrow per frame"
        );
        assert_eq!(
            RuntimeFrameFamily::Spawn.payload_presence(),
            PayloadPresenceRule::Required
        );
        assert_eq!(RuntimeFrameFamily::Spawn.wire_type_prefix(), "spawn.");
        assert_eq!(
            spawn_submit_frame_direction(SPAWN_SUBMIT_REQUEST_FRAME_TYPE),
            Some(FrameDirection::RuntimeToRouter)
        );
        assert_eq!(
            spawn_submit_frame_direction(SPAWN_SUBMIT_RESPONSE_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
        assert_eq!(
            spawn_submit_frame_direction(SPAWN_SUBMIT_ERROR_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
    }

    #[test]
    fn frame_metadata_is_frozen() {
        let catalog = catalog();
        for (name, entry) in &catalog.frames {
            assert_eq!(
                entry.direction,
                expected_direction(name),
                "{name}: direction"
            );
            let (frame_type, decode_as, presence) = match name.as_str() {
                "spawn.submit.request.function"
                | "spawn.submit.request.actorMethod"
                | "spawn.submit.request.legacy-no-caller-kind" => {
                    ("spawn.submit.request", "SpawnSubmitRequest", "required")
                }
                "spawn.submit.response" => {
                    ("spawn.submit.response", "SpawnSubmitResponse", "empty")
                }
                "spawn.submit.error.parentNotFound" => {
                    ("spawn.submit.error", "SpawnSubmitError", "empty")
                }
                _ => panic!("unexpected spawn frame {name}"),
            };
            assert_eq!(entry.frame_type, frame_type, "{name}: frameType");
            assert_eq!(entry.decode_as, decode_as, "{name}: decodeAs");
            assert_eq!(entry.payload_presence, presence, "{name}: payloadPresence");
        }
    }

    fn expected_direction(name: &str) -> &'static str {
        match name {
            "spawn.submit.request.function"
            | "spawn.submit.request.actorMethod"
            | "spawn.submit.request.legacy-no-caller-kind" => "RuntimeToRouter",
            "spawn.submit.response" | "spawn.submit.error.parentNotFound" => "RouterToRuntime",
            _ => panic!("unexpected spawn frame {name}"),
        }
    }

    #[test]
    fn target_shaped_requests_are_byte_exact_and_payload_consistent() {
        let catalog = catalog();
        for name in [
            "spawn.submit.request.function",
            "spawn.submit.request.actorMethod",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_spawn_submit_request_frame(&bytes)
                .expect("target shape must decode through the canonical codec");
            let reencoded = encode_spawn_submit_request_frame(&header, &payload)
                .expect("target shape re-encode");
            assert_eq!(bytes, reencoded, "{name} must be byte-exact");
            let fixture_header: SpawnSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert_eq!(payload, payload_of(entry), "{name} payload mismatch");
            assert!(!entry.legacy_cut, "{name} is not legacy cut");
        }
    }

    #[test]
    fn legacy_old_shape_has_no_compatible_reader() {
        let catalog = catalog();
        let entry = &catalog.frames["spawn.submit.request.legacy-no-caller-kind"];
        assert!(entry.legacy_cut);
        let bytes = hex_bytes(&entry.frame_hex);
        let error = decode_spawn_submit_request_frame(&bytes).expect_err(
            "legacy old-shape frame (no callerKind) must be rejected by the canonical codec",
        );
        assert!(
            error.to_string().contains("callerKind"),
            "legacy rejection must name callerKind, got {error}"
        );
        // No fallback: the closed enum has exactly two values and the canonical
        // codec requires the field. A reader that guessed a default would accept
        // this frame; the frozen contract forbids that reader.
        let header_json: Value = serde_json::from_str(
            r#"{
                "schemaVersion":"skiff-runtime-frame-v3",
                "type":"spawn.submit.request",
                "rpcId":"rpc:probe-1",
                "runtimeId":"runtime-a",
                "callerKind":"function",
                "callerRequestId":"parent-1",
                "targetKind":"function",
                "serviceId":"example.com/docs",
                "serviceVersion":"1.0.0",
                "serviceProtocolIdentity":"example.com/docs:1.0.0",
                "target":"example.com/fn",
                "activationIdentity":{
                    "assemblyIdentity":"skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "generation":42,
                    "runtimeReplicaId":"runtime-a",
                    "deploymentRevision":"rev-1"
                }
            }"#,
        )
        .expect("probe json");
        let probe_result = serde_json::from_value::<SpawnSubmitRequestFrameHeaderV2>(header_json);
        assert!(
            probe_result.is_err(),
            "callerKind=function is not a valid parent kind; closed enum must reject it"
        );
    }

    #[test]
    fn response_and_error_frames_round_trip_through_canonical_dtos() {
        let catalog = catalog();
        let entry = &catalog.frames["spawn.submit.response"];
        let header = decode_spawn_submit_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("spawn.submit.response must decode");
        assert_eq!(
            bytes_hex(&encode_spawn_submit_response_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "spawn.submit.response must be byte-exact"
        );

        let entry = &catalog.frames["spawn.submit.error.parentNotFound"];
        let header = decode_spawn_submit_error_frame(&hex_bytes(&entry.frame_hex))
            .expect("spawn.submit.error must decode");
        assert_eq!(
            bytes_hex(&encode_spawn_submit_error_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "spawn.submit.error must be byte-exact"
        );
    }

    #[test]
    fn spawn_scenarios_drive_the_resolver_router_reference_model() {
        for (name, raw) in SPAWN_SCENARIOS {
            run_spawn_scenario(name, raw);
        }
    }

    #[test]
    fn required_spawn_scenario_names_are_frozen() {
        let required = [
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
        for (name, _) in SPAWN_SCENARIOS {
            assert!(
                required.contains(&name),
                "spawn scenario {name} is not in the frozen required list"
            );
        }
        for name in required {
            assert!(
                SPAWN_SCENARIOS
                    .iter()
                    .any(|(scenario, _)| *scenario == name),
                "required spawn scenario {name} is missing"
            );
        }
    }

    fn bytes_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
