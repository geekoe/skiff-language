//! Replays the frozen contracts-session handshake corpus
//! (`runtime/transport/testdata/registration-handshake/`) through the
//! PRODUCTION W-session state machine, directory and registration sink.
//!
//! This is the consumer gate required by C-model-registration §5.7: the same
//! fixtures must pass through the real codec/state machine, not only the
//! test-only reference model in `skiff-runtime-transport`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{AssemblyActivationControl, RuntimeAssemblyRef};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::demux::{RegistrationFrameSink, RegistrationSinkOutput};
use skiff_router::session::directory::RuntimeRegistrationDirectory;
use skiff_router::session::handshake::{
    CapabilitiesEvent, EpochContext, HandshakeState, HealthEvent, RegisterControl, TimeoutKind,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::pre_auth::PreAuthPool;
use skiff_router::session::{ConsumerKind, HandshakePhase, TerminalKind};
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame_header, decode_typed_binary_frame,
    RouterBootstrapFrameHeader, RuntimeCapabilitiesFrameHeader, RuntimeHealthFrameHeader,
    RuntimeRegisterFrameHeader,
};

const REQUIRED_SCENARIOS: [&str; 20] = [
    "accept-sequence",
    "wrong-order-health-before-capabilities",
    "wrong-order-register-before-capabilities",
    "legacy-register-rejected",
    "identity-change-register-replica",
    "identity-change-capabilities-replica",
    "duplicate-register-pre-ack",
    "stale-register-old-generation",
    "tuple-mismatch-assembly",
    "new-generation-before-epoch-swap",
    "ack-loss",
    "health-before-ack-no-observation",
    "pre-auth-limit",
    "bootstrap-timeout",
    "capabilities-timeout",
    "register-timeout",
    "disconnect-mid-handshake",
    "re-register-exact-idempotent",
    "re-register-stale-after-ack",
    "capabilities-refresh-same-replica",
];

#[derive(Debug, Clone)]
struct FrameEntry {
    frame_hex: String,
    decode_as: String,
}

#[derive(Debug, Clone)]
enum SemanticFrame {
    Bootstrap,
    Capabilities {
        runtime_id: String,
    },
    Register {
        tuple: RegisteredAssemblyTuple,
        replica_id: String,
    },
    Registered {
        runtime_id: String,
    },
    Health {
        runtime_id: String,
    },
    LegacyRegister,
}

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn corpus_path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("registration-handshake")
        .join(relative)
}

fn catalog() -> HashMap<String, FrameEntry> {
    let text = std::fs::read_to_string(corpus_path("frames.json")).expect("frames.json must exist");
    let root: Value = serde_json::from_str(&text).expect("frames.json must parse");
    root["frames"]
        .as_object()
        .expect("frames must be an object")
        .iter()
        .map(|(name, entry)| {
            (
                name.clone(),
                FrameEntry {
                    frame_hex: entry["frameHex"].as_str().expect("frameHex").to_string(),
                    decode_as: entry["decodeAs"].as_str().expect("decodeAs").to_string(),
                },
            )
        })
        .collect()
}

fn semantic_frame(entry: &FrameEntry) -> SemanticFrame {
    let bytes = hex_decode(&entry.frame_hex);
    match entry.decode_as.as_str() {
        "RouterBootstrap" => {
            let header: Value = decode_binary_frame(&bytes)
                .expect("bootstrap decodes")
                .header;
            let _: RouterBootstrapFrameHeader =
                decode_router_bootstrap_frame_header(header).expect("bootstrap typed decode");
            SemanticFrame::Bootstrap
        }
        "Capabilities" => {
            let (header, payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("capabilities decodes");
            assert!(payload.is_empty(), "capabilities payload must be empty");
            SemanticFrame::Capabilities {
                runtime_id: header.runtime_id,
            }
        }
        "AssemblyRegister" => {
            let control = decode_assembly_activation_frame(
                AssemblyActivationFrameDirection::RuntimeToRouter,
                &bytes,
            )
            .expect("register decodes");
            let AssemblyActivationControl::Register {
                profile,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            } = control
            else {
                panic!("expected register control");
            };
            SemanticFrame::Register {
                tuple: RegisteredAssemblyTuple {
                    profile,
                    generation,
                    assembly,
                    config_snapshot,
                },
                replica_id,
            }
        }
        "Registered" => {
            let (header, payload): (
                skiff_runtime_transport::protocol::RuntimeRegisteredFrameHeader,
                Vec<u8>,
            ) = decode_typed_binary_frame(&bytes).expect("registered decodes");
            assert!(payload.is_empty(), "registered payload must be empty");
            SemanticFrame::Registered {
                runtime_id: header.runtime_id,
            }
        }
        "Health" => {
            let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("health decodes");
            assert!(payload.is_empty(), "health payload must be empty");
            SemanticFrame::Health {
                runtime_id: header.runtime_id,
            }
        }
        "LegacyRegister" => {
            let (_header, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("legacy register decodes");
            assert!(payload.is_empty(), "legacy register payload must be empty");
            SemanticFrame::LegacyRegister
        }
        other => panic!("unknown decodeAs {other}"),
    }
}

#[derive(Debug)]
struct Conn {
    machine: HandshakeState,
    session: Option<RuntimeSessionEpoch>,
    revision: u64,
}

#[derive(Debug)]
struct Harness {
    current: RegisteredAssemblyTuple,
    pending: Option<RegisteredAssemblyTuple>,
    pre_auth: PreAuthPool,
    directory: RuntimeRegistrationDirectory,
    conns: HashMap<String, Conn>,
    refused: HashMap<String, u64>,
    registered_replicas: Vec<String>,
    observed_health: u64,
    health_before_ack: u64,
}

impl Harness {
    fn new(
        current: RegisteredAssemblyTuple,
        pending: Option<RegisteredAssemblyTuple>,
        pre_auth_limit: usize,
    ) -> Self {
        Self {
            current,
            pending,
            pre_auth: PreAuthPool::new(pre_auth_limit),
            directory: RuntimeRegistrationDirectory::new(&ConsumerManifest::installed([
                ConsumerKind::HealthLedger,
            ])),
            conns: HashMap::new(),
            refused: HashMap::new(),
            registered_replicas: Vec::new(),
            observed_health: 0,
            health_before_ack: 0,
        }
    }

    fn accept(&mut self, connection_id: &str, generation: u64) {
        assert!(generation >= 1, "connection generation must be positive");
        if !self.pre_auth.try_acquire(connection_id) {
            *self.refused.entry(connection_id.to_string()).or_insert(0) += 1;
            return;
        }
        self.conns.insert(
            connection_id.to_string(),
            Conn {
                machine: HandshakeState::new(),
                session: None,
                revision: 0,
            },
        );
    }

    fn close(&mut self, connection_id: &str) {
        let Some(conn) = self.conns.get_mut(connection_id) else {
            return;
        };
        if let Some(replica) = conn.machine.replica() {
            self.registered_replicas
                .retain(|existing| existing != replica);
        }
        let session = conn.session.clone();
        if let Some(session) = session {
            if self.directory.begin_close(&session).is_some() {
                let permits = self
                    .directory
                    .record(&session)
                    .map(|record| record.consumer_permits.clone())
                    .unwrap_or_default();
                for permit in permits {
                    let _ = self.directory.ack_close(&session, permit);
                }
            }
        }
        conn.revision = 0;
        self.pre_auth.release(connection_id);
    }

    fn write(&mut self, connection_id: &str, frame: &SemanticFrame) {
        let conn = self
            .conns
            .get_mut(connection_id)
            .expect("connection exists");
        match frame {
            SemanticFrame::Bootstrap => {
                conn.machine
                    .on_bootstrap_written()
                    .expect("bootstrap written from Accepted");
            }
            SemanticFrame::Registered { runtime_id } => {
                conn.machine
                    .on_ack_written()
                    .expect("registered ACK requires RegisterValidated");
                let session = conn.session.as_ref().expect("session bound").clone();
                assert!(
                    self.directory.mark_registered(&session),
                    "pending record must exist and not be cancelled"
                );
                self.pre_auth.release(connection_id);
                self.registered_replicas.push(runtime_id.clone());
            }
            other => panic!("unexpected outbound frame {other:?}"),
        }
    }

    fn write_fail(&mut self, connection_id: &str, frame: &SemanticFrame) {
        match frame {
            SemanticFrame::Bootstrap => {
                self.conns
                    .get_mut(connection_id)
                    .expect("connection exists")
                    .machine
                    .on_bootstrap_write_failed();
            }
            SemanticFrame::Registered { .. } => {
                self.conns
                    .get_mut(connection_id)
                    .expect("connection exists")
                    .machine
                    .on_ack_write_failed();
            }
            other => panic!("unexpected outbound write-fail frame {other:?}"),
        }
        self.close(connection_id);
    }

    fn read(&mut self, connection_id: &str, frame: &SemanticFrame) {
        let conn = self
            .conns
            .get_mut(connection_id)
            .expect("connection exists");
        assert!(conn.machine.terminal().is_none(), "read after terminal");
        let context = EpochContext {
            current: Some(self.current.clone()),
            pending: self.pending.clone(),
        };
        match frame {
            SemanticFrame::Capabilities { runtime_id } => {
                match conn.machine.on_capabilities(runtime_id) {
                    CapabilitiesEvent::Bound => {
                        conn.session = Some(RuntimeSessionEpoch {
                            replica_id: runtime_id.clone(),
                            connection_generation: 1,
                        });
                    }
                    CapabilitiesEvent::Refreshed => {}
                    CapabilitiesEvent::Terminal(_) => {}
                }
            }
            SemanticFrame::Register { tuple, replica_id } => {
                let register = RegisterControl {
                    profile: tuple.profile.clone(),
                    generation: tuple.generation,
                    assembly: tuple.assembly.clone(),
                    config_snapshot: tuple.config_snapshot.clone(),
                    replica_id: replica_id.clone(),
                };
                if let Some(session) = conn.session.clone() {
                    let output = RegistrationFrameSink.handle_register(
                        &mut conn.machine,
                        &mut self.directory,
                        &session,
                        &register,
                        &context,
                        &[ConsumerKind::HealthLedger],
                    );
                    match output {
                        RegistrationSinkOutput::PendingPublished { revision, .. }
                        | RegistrationSinkOutput::TransitionPublished { revision } => {
                            conn.revision = revision
                        }
                        RegistrationSinkOutput::Idempotent => {}
                        RegistrationSinkOutput::Terminal(_) => {}
                    }
                } else {
                    // Pre-bind register: phase machine reports WrongOrder.
                    conn.machine.on_register(&register, &context);
                }
            }
            SemanticFrame::Health { runtime_id } => match conn.machine.on_health(runtime_id) {
                HealthEvent::Observed => {
                    self.observed_health += 1;
                }
                HealthEvent::DroppedBeforeAck => {
                    self.health_before_ack += 1;
                }
                HealthEvent::Terminal(_) => {}
            },
            SemanticFrame::LegacyRegister => {
                conn.machine.on_legacy_register();
            }
            SemanticFrame::Bootstrap | SemanticFrame::Registered { .. } => {
                panic!("router-to-runtime frames are outbound only")
            }
        }
        if self
            .conns
            .get(connection_id)
            .expect("connection exists")
            .machine
            .terminal()
            .is_some()
        {
            self.close(connection_id);
        }
    }

    fn timeout(&mut self, connection_id: &str, kind: &str) {
        let terminal = match (kind, self.conns.get(connection_id).unwrap().machine.phase()) {
            ("bootstrap", HandshakePhase::Accepted) => TimeoutKind::Bootstrap,
            ("capabilities", HandshakePhase::BootstrapSent) => TimeoutKind::Capabilities,
            ("register", HandshakePhase::CapabilitiesBound) => TimeoutKind::Register,
            _ => panic!("timeout {kind} in invalid phase"),
        };
        self.conns
            .get_mut(connection_id)
            .expect("connection exists")
            .machine
            .on_timeout(terminal);
        self.close(connection_id);
    }

    fn disconnect(&mut self, connection_id: &str) {
        let conn = self
            .conns
            .get_mut(connection_id)
            .expect("connection exists");
        assert!(
            conn.machine.terminal().is_none(),
            "disconnect after terminal"
        );
        conn.machine.on_disconnect();
        self.close(connection_id);
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Scenario {
    scenario: String,
    #[serde(rename = "mainConnection")]
    main_connection: String,
    epoch: EpochValue,
    #[serde(rename = "preAuthLimit")]
    pre_auth_limit: usize,
    events: Vec<EventValue>,
    expect: ExpectValue,
}

#[derive(Debug, Clone, Deserialize)]
struct RefValue {
    #[serde(rename = "assemblyIdentity")]
    assembly_identity: Option<String>,
    #[serde(rename = "snapshotId")]
    snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EpochValue {
    profile: String,
    generation: u64,
    assembly: RefValue,
    #[serde(rename = "configSnapshot")]
    config_snapshot: RefValue,
    pending: Option<PendingValue>,
}

#[derive(Debug, Clone, Deserialize)]
struct PendingValue {
    profile: String,
    generation: u64,
    assembly: RefValue,
    #[serde(rename = "configSnapshot")]
    config_snapshot: RefValue,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum EventValue {
    Accept {
        connection: String,
        #[serde(rename = "connectionGeneration")]
        connection_generation: u64,
    },
    Write {
        connection: String,
        frame: String,
    },
    #[serde(rename = "writeFail")]
    WriteFail {
        connection: String,
        frame: String,
    },
    Read {
        connection: String,
        frame: String,
    },
    Timeout {
        connection: String,
        #[serde(rename = "timeoutKind")]
        timeout_kind: String,
    },
    Disconnect {
        connection: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct ExpectValue {
    outcomes: HashMap<String, String>,
    #[serde(rename = "refusedCount")]
    refused_count: u64,
    #[serde(rename = "preAuthCount")]
    pre_auth_count: usize,
    #[serde(rename = "registeredSessions")]
    registered_sessions: Vec<String>,
    #[serde(rename = "observedHealth")]
    observed_health: u64,
    #[serde(rename = "healthBeforeAck")]
    health_before_ack: u64,
    #[serde(rename = "routableRegistered")]
    routable_registered: bool,
    #[serde(rename = "publishedPending")]
    published_pending: bool,
    revision: u64,
    #[serde(rename = "failStop")]
    fail_stop: bool,
}

fn scenarios() -> Vec<Scenario> {
    let dir = corpus_path("scenarios");
    let mut paths = std::fs::read_dir(&dir)
        .expect("scenarios dir")
        .map(|entry| entry.expect("scenario entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("scenario readable");
            serde_json::from_str(&text).expect("scenario parses")
        })
        .collect()
}

fn tuple(
    assembly: &RefValue,
    snapshot: &RefValue,
    profile: &str,
    generation: u64,
) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: profile.to_string(),
        generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                assembly
                    .assembly_identity
                    .as_ref()
                    .expect("assembly identity")
                    .clone(),
            ),
        },
        config_snapshot: skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                snapshot.snapshot_id.as_ref().expect("snapshot id").clone(),
            )
            .expect("valid snapshot id"),
        },
    }
}

fn epoch_tuple(epoch: &EpochValue) -> RegisteredAssemblyTuple {
    tuple(
        &epoch.assembly,
        &epoch.config_snapshot,
        &epoch.profile,
        epoch.generation,
    )
}

fn pending_tuple(epoch: &EpochValue) -> Option<RegisteredAssemblyTuple> {
    epoch.pending.as_ref().map(|pending| {
        tuple(
            &pending.assembly,
            &pending.config_snapshot,
            &pending.profile,
            pending.generation,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_production_handshake_machine_matches_corpus_scenarios() {
        let frames = catalog();
        let scenario_files = scenarios();
        let names = scenario_files
            .iter()
            .map(|scenario| scenario.scenario.as_str())
            .collect::<std::collections::HashSet<_>>();
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.contains(required),
                "required scenario {required} missing"
            );
        }

        for scenario in &scenario_files {
            let semantic_frames = frames
                .iter()
                .map(|(name, entry)| (name.clone(), semantic_frame(entry)))
                .collect::<HashMap<_, _>>();
            let mut harness = Harness::new(
                epoch_tuple(&scenario.epoch),
                pending_tuple(&scenario.epoch),
                scenario.pre_auth_limit,
            );
            for event in &scenario.events {
                match event {
                    EventValue::Accept {
                        connection,
                        connection_generation,
                    } => harness.accept(connection, *connection_generation),
                    EventValue::Write { connection, frame } => {
                        let frame = semantic_frames
                            .get(frame)
                            .unwrap_or_else(|| panic!("unknown frame {frame}"));
                        harness.write(connection, frame);
                    }
                    EventValue::WriteFail { connection, frame } => {
                        let frame = semantic_frames
                            .get(frame)
                            .unwrap_or_else(|| panic!("unknown frame {frame}"));
                        harness.write_fail(connection, frame);
                    }
                    EventValue::Read { connection, frame } => {
                        let frame = semantic_frames
                            .get(frame)
                            .unwrap_or_else(|| panic!("unknown frame {frame}"));
                        harness.read(connection, frame);
                    }
                    EventValue::Timeout {
                        connection,
                        timeout_kind,
                    } => harness.timeout(connection, timeout_kind),
                    EventValue::Disconnect { connection } => harness.disconnect(connection),
                }
            }

            let mut actual_outcomes = harness
                .conns
                .iter()
                .map(|(id, conn)| (id.clone(), conn.machine.outcome_name()))
                .collect::<BTreeMap<_, _>>();
            for id in harness.refused.keys() {
                if !harness.conns.contains_key(id) {
                    actual_outcomes.insert(
                        id.clone(),
                        format!("{:?}", TerminalKind::PreAuthLimitRejected),
                    );
                }
            }
            let expected_outcomes = scenario
                .expect
                .outcomes
                .iter()
                .map(|(id, outcome)| (id.clone(), outcome.clone()))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(
                actual_outcomes, expected_outcomes,
                "scenario {}: outcomes",
                scenario.scenario
            );
            assert_eq!(
                harness.refused.values().sum::<u64>(),
                scenario.expect.refused_count,
                "scenario {}: refusedCount",
                scenario.scenario
            );
            assert_eq!(
                harness.pre_auth.occupied(),
                scenario.expect.pre_auth_count,
                "scenario {}: preAuthCount",
                scenario.scenario
            );
            assert_eq!(
                harness.registered_replicas, scenario.expect.registered_sessions,
                "scenario {}: registeredSessions",
                scenario.scenario
            );
            assert_eq!(
                harness.observed_health, scenario.expect.observed_health,
                "scenario {}: observedHealth",
                scenario.scenario
            );
            assert_eq!(
                harness.health_before_ack, scenario.expect.health_before_ack,
                "scenario {}: healthBeforeAck",
                scenario.scenario
            );
            let main = harness
                .conns
                .get(&scenario.main_connection)
                .expect("main connection exists");
            assert_eq!(
                main.machine.phase() == HandshakePhase::Registered,
                scenario.expect.routable_registered,
                "scenario {}: routableRegistered",
                scenario.scenario
            );
            assert_eq!(
                main.machine.phase() == HandshakePhase::RegisterValidated,
                scenario.expect.published_pending,
                "scenario {}: publishedPending",
                scenario.scenario
            );
            assert_eq!(
                main.revision, scenario.expect.revision,
                "scenario {}: revision",
                scenario.scenario
            );
            assert!(
                !scenario.expect.fail_stop,
                "scenario {}: failStop must be false",
                scenario.scenario
            );
            assert!(
                !harness.directory.fail_stopped(),
                "scenario {}: directory must not fail-stop",
                scenario.scenario
            );
        }
    }
}
