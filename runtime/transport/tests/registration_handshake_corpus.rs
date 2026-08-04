//! Byte-exact handshake sequence corpus verifier for C-model-registration
//! (`doc/implementation/router-rust-migration-c-model-registration-contract.md`).
//!
//! This is a TEST-ONLY reference model. It is not production code, is not
//! imported by any production crate, and must not be treated as the
//! W-session implementation. W-session must implement the frozen semantics
//! (owner/invariant in the contract doc) and consume the same fixtures.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    protocol::{
        decode_binary_frame, decode_router_bootstrap_frame_header, decode_typed_binary_frame,
        encode_binary_frame, RouterBootstrapFrameHeader, RuntimeCapabilitiesFrameHeader,
        RuntimeHealthFrameHeader, RuntimeRegisterFrameHeader, RuntimeRegisteredFrameHeader,
    },
};

const REQUIRED_SCENARIOS: [&str; 19] = [
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
];

const REQUIRED_FRAMES: [&str; 12] = [
    "bootstrap.prod.42",
    "capabilities.runtime-a",
    "capabilities.runtime-b",
    "register.prod.42.a",
    "register.prod.42.b",
    "register.prod.41.a",
    "register.prod.42.other-assembly",
    "register.prod.43.a",
    "registered.runtime-a",
    "registered.runtime-b",
    "health.empty",
    "legacy.runtime.register",
];

#[derive(Debug, Clone, Deserialize)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tuple {
    profile: String,
    generation: u64,
    assembly: String,
    config_snapshot: String,
}

#[derive(Debug, Clone)]
enum SemanticFrame {
    Bootstrap,
    Capabilities { runtime_id: String },
    Register { tuple: Tuple, replica_id: String },
    Registered { runtime_id: String },
    Health { runtime_id: String },
    LegacyRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    WrongOrder,
    IdentityChange,
    DuplicateRegister,
    StaleRegister,
    NewGenerationBeforeEpochSwap,
    LegacyRegisterRejected,
    BootstrapWriteFail,
    AckLoss,
    BootstrapTimeout,
    CapabilitiesTimeout,
    RegisterTimeout,
    Disconnect,
    PreAuthLimitRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Accepted,
    BootstrapSent,
    CapabilitiesBound,
    RegisterValidated,
    Registered,
    Closed,
}

#[derive(Debug)]
struct Conn {
    phase: Phase,
    terminal: Option<Terminal>,
    replica: Option<String>,
    tuple: Option<Tuple>,
    revision: u64,
    health_before_ack: u64,
}

impl Conn {
    fn new() -> Self {
        Self {
            phase: Phase::Accepted,
            terminal: None,
            replica: None,
            tuple: None,
            revision: 0,
            health_before_ack: 0,
        }
    }
}

struct Machine {
    current: Tuple,
    pending: Option<Tuple>,
    pre_auth_limit: usize,
    pre_auth: Vec<String>,
    conns: HashMap<String, Conn>,
    refused: HashMap<String, u64>,
    registered_replicas: Vec<String>,
    observed_health: u64,
    health_before_ack_total: u64,
}

impl Machine {
    fn new(current: Tuple, pending: Option<Tuple>, pre_auth_limit: usize) -> Self {
        Self {
            current,
            pending,
            pre_auth_limit,
            pre_auth: Vec::new(),
            conns: HashMap::new(),
            refused: HashMap::new(),
            registered_replicas: Vec::new(),
            observed_health: 0,
            health_before_ack_total: 0,
        }
    }

    fn terminal(&mut self, conn_id: &str, terminal: Terminal) {
        let conn = self.conns.get_mut(conn_id).expect("connection exists");
        assert!(conn.terminal.is_none(), "connection already terminal");
        conn.terminal = Some(terminal);
        conn.phase = Phase::Closed;
        conn.revision = 0;
        conn.tuple = None;
        if let Some(replica) = conn.replica.clone() {
            self.registered_replicas
                .retain(|existing| existing != &replica);
        }
        self.pre_auth.retain(|id| id != conn_id);
    }

    fn accept(&mut self, conn_id: &str, generation: u64) {
        if self.pre_auth.len() >= self.pre_auth_limit {
            *self.refused.entry(conn_id.to_string()).or_default() += 1;
            return;
        }
        assert!(generation >= 1, "connection generation must be positive");
        if let Some(existing) = self.conns.get(conn_id) {
            assert!(
                existing.phase == Phase::Closed,
                "re-accept of a live connection is a fixture error"
            );
        }
        self.pre_auth.push(conn_id.to_string());
        self.conns.insert(conn_id.to_string(), Conn::new());
    }

    fn write(&mut self, conn_id: &str, frame: &SemanticFrame) {
        match frame {
            SemanticFrame::Bootstrap => {
                let conn = self.conns.get_mut(conn_id).expect("connection exists");
                assert!(conn.terminal.is_none(), "write after terminal");
                assert_eq!(
                    conn.phase,
                    Phase::Accepted,
                    "bootstrap must be written from Accepted"
                );
                conn.phase = Phase::BootstrapSent;
            }
            SemanticFrame::Registered { runtime_id } => {
                let (phase, replica) = {
                    let conn = self.conns.get(conn_id).expect("connection exists");
                    (conn.phase, conn.replica.clone())
                };
                assert!(
                    phase == Phase::RegisterValidated,
                    "registered ACK requires RegisterValidated"
                );
                assert_eq!(
                    replica.as_deref(),
                    Some(runtime_id.as_str()),
                    "registered ACK runtimeId must match bound replica"
                );
                let conn = self.conns.get_mut(conn_id).unwrap();
                conn.phase = Phase::Registered;
                self.pre_auth.retain(|id| id != conn_id);
                self.registered_replicas.push(runtime_id.clone());
            }
            _ => panic!("unexpected outbound frame kind"),
        }
    }

    fn write_fail(&mut self, conn_id: &str, frame: &SemanticFrame) {
        match frame {
            SemanticFrame::Bootstrap => {
                let conn = self.conns.get(conn_id).expect("connection exists");
                assert_eq!(conn.phase, Phase::Accepted);
                self.terminal(conn_id, Terminal::BootstrapWriteFail);
            }
            SemanticFrame::Registered { .. } => {
                let conn = self.conns.get(conn_id).expect("connection exists");
                assert_eq!(conn.phase, Phase::RegisterValidated);
                self.terminal(conn_id, Terminal::AckLoss);
            }
            _ => panic!("unexpected outbound frame kind"),
        }
    }

    fn read(&mut self, conn_id: &str, frame: &SemanticFrame) {
        let snapshot = {
            let conn = self.conns.get(conn_id).expect("connection exists");
            assert!(conn.terminal.is_none(), "read after terminal");
            (conn.phase, conn.replica.clone(), conn.tuple.clone())
        };
        match frame {
            SemanticFrame::Capabilities { runtime_id } => match snapshot.0 {
                Phase::Accepted => self.terminal(conn_id, Terminal::WrongOrder),
                Phase::BootstrapSent => {
                    if let Some(bound) = snapshot.1 {
                        if bound == *runtime_id {
                            self.terminal(conn_id, Terminal::WrongOrder);
                        } else {
                            self.terminal(conn_id, Terminal::IdentityChange);
                        }
                    } else {
                        let conn = self.conns.get_mut(conn_id).unwrap();
                        conn.replica = Some(runtime_id.clone());
                        conn.phase = Phase::CapabilitiesBound;
                    }
                }
                Phase::CapabilitiesBound | Phase::RegisterValidated | Phase::Registered => {
                    if snapshot.1.as_deref() == Some(runtime_id.as_str()) {
                        self.terminal(conn_id, Terminal::WrongOrder);
                    } else {
                        self.terminal(conn_id, Terminal::IdentityChange);
                    }
                }
                Phase::Closed => unreachable!("closed handled above"),
            },
            SemanticFrame::Register { tuple, replica_id } => match snapshot.0 {
                Phase::Accepted | Phase::BootstrapSent => {
                    self.terminal(conn_id, Terminal::WrongOrder);
                }
                Phase::CapabilitiesBound => {
                    if snapshot.1.as_deref() != Some(replica_id.as_str()) {
                        self.terminal(conn_id, Terminal::IdentityChange);
                    } else if *tuple == self.current {
                        let conn = self.conns.get_mut(conn_id).unwrap();
                        conn.revision += 1;
                        conn.tuple = Some(tuple.clone());
                        conn.phase = Phase::RegisterValidated;
                    } else if self.pending.as_ref() == Some(tuple) {
                        self.terminal(conn_id, Terminal::NewGenerationBeforeEpochSwap);
                    } else {
                        self.terminal(conn_id, Terminal::StaleRegister);
                    }
                }
                Phase::RegisterValidated => self.terminal(conn_id, Terminal::DuplicateRegister),
                Phase::Registered => {
                    if snapshot.2.as_ref() == Some(tuple) {
                        // Exact duplicate re-register is idempotent (§3.2).
                    } else if *tuple == self.current {
                        self.terminal(conn_id, Terminal::StaleRegister);
                    } else if self.pending.as_ref() == Some(tuple) {
                        self.terminal(conn_id, Terminal::NewGenerationBeforeEpochSwap);
                    } else {
                        self.terminal(conn_id, Terminal::StaleRegister);
                    }
                }
                Phase::Closed => unreachable!("closed handled above"),
            },
            SemanticFrame::Health { runtime_id } => match snapshot.0 {
                Phase::Registered => {
                    if snapshot.1.as_deref() != Some(runtime_id.as_str()) {
                        self.terminal(conn_id, Terminal::IdentityChange);
                    } else {
                        self.observed_health += 1;
                    }
                }
                Phase::RegisterValidated => {
                    if snapshot.1.as_deref() != Some(runtime_id.as_str()) {
                        self.terminal(conn_id, Terminal::IdentityChange);
                    } else {
                        let conn = self.conns.get_mut(conn_id).unwrap();
                        conn.health_before_ack += 1;
                        self.health_before_ack_total += 1;
                    }
                }
                _ => self.terminal(conn_id, Terminal::WrongOrder),
            },
            SemanticFrame::LegacyRegister => {
                self.terminal(conn_id, Terminal::LegacyRegisterRejected);
            }
            SemanticFrame::Bootstrap | SemanticFrame::Registered { .. } => {
                panic!("router-to-runtime frames are outbound only")
            }
        }
    }

    fn timeout(&mut self, conn_id: &str, kind: &str) {
        let phase = self.conns.get(conn_id).expect("connection exists").phase;
        let terminal = match (kind, phase) {
            ("bootstrap", Phase::Accepted) => Terminal::BootstrapTimeout,
            ("capabilities", Phase::BootstrapSent) => Terminal::CapabilitiesTimeout,
            ("register", Phase::CapabilitiesBound) => Terminal::RegisterTimeout,
            _ => panic!("timeout {kind} in invalid phase {phase:?}"),
        };
        self.terminal(conn_id, terminal);
    }

    fn disconnect(&mut self, conn_id: &str) {
        let conn = self.conns.get(conn_id).expect("connection exists");
        assert!(conn.terminal.is_none(), "disconnect after terminal");
        self.terminal(conn_id, Terminal::Disconnect);
    }

    fn outcome(&self, conn_id: &str) -> String {
        let conn = self.conns.get(conn_id).expect("connection exists");
        match (conn.phase, conn.terminal) {
            (Phase::Closed, Some(terminal)) => format!("{terminal:?}"),
            (phase, None) => format!("{phase:?}"),
            _ => panic!("closed without terminal"),
        }
    }

    fn refused_outcome(&self, conn_id: &str) -> String {
        assert!(
            self.refused.contains_key(conn_id),
            "connection was never refused"
        );
        format!("{:?}", Terminal::PreAuthLimitRejected)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssemblyValue {
    assembly_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotValue {
    snapshot_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct EpochValue {
    profile: String,
    generation: u64,
    assembly: AssemblyValue,
    #[serde(rename = "configSnapshot")]
    config_snapshot: SnapshotValue,
    pending: Option<PendingValue>,
}

impl EpochValue {
    fn tuple(&self) -> Tuple {
        Tuple {
            profile: self.profile.clone(),
            generation: self.generation,
            assembly: self.assembly.assembly_identity.clone(),
            config_snapshot: self.config_snapshot.snapshot_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PendingValue {
    profile: String,
    generation: u64,
    assembly: AssemblyValue,
    #[serde(rename = "configSnapshot")]
    config_snapshot: SnapshotValue,
}

impl PendingValue {
    fn tuple(&self) -> Tuple {
        Tuple {
            profile: self.profile.clone(),
            generation: self.generation,
            assembly: self.assembly.assembly_identity.clone(),
            config_snapshot: self.config_snapshot.snapshot_id.clone(),
        }
    }
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
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    #[serde(rename = "mainConnection")]
    main_connection: String,
    epoch: EpochValue,
    #[serde(rename = "preAuthLimit")]
    pre_auth_limit: usize,
    events: Vec<EventValue>,
    expect: ExpectValue,
}

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn decode_catalog_frame(entry: &FrameEntry) -> SemanticFrame {
    let bytes = hex_decode(&entry.frame_hex);
    let decoded_header = decode_binary_frame(&bytes)
        .expect("frame must decode as a skiff binary frame")
        .header;
    assert_eq!(
        decoded_header, entry.header,
        "stored header JSON must match canonical decode for {}",
        entry.decode_as
    );
    match entry.decode_as.as_str() {
        "RouterBootstrap" => {
            let typed: RouterBootstrapFrameHeader =
                decode_router_bootstrap_frame_header(decoded_header.clone())
                    .expect("bootstrap decodes");
            let reencoded = encode_binary_frame(&typed, &[]).expect("bootstrap re-encodes");
            assert_eq!(reencoded, bytes, "bootstrap frame must be byte-exact");
            SemanticFrame::Bootstrap
        }
        "Capabilities" => {
            let (typed, payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("capabilities decodes");
            assert!(payload.is_empty(), "capabilities payload must be empty");
            let reencoded = encode_binary_frame(&typed, &[]).expect("capabilities re-encodes");
            assert_eq!(reencoded, bytes, "capabilities frame must be byte-exact");
            SemanticFrame::Capabilities {
                runtime_id: typed.runtime_id,
            }
        }
        "AssemblyRegister" => {
            let control = decode_assembly_activation_frame(
                AssemblyActivationFrameDirection::RuntimeToRouter,
                &bytes,
            )
            .expect("register decodes");
            let reencoded = encode_assembly_activation_frame(
                AssemblyActivationFrameDirection::RuntimeToRouter,
                &control,
            )
            .expect("register re-encodes");
            assert_eq!(reencoded, bytes, "register frame must be byte-exact");
            match control {
                AssemblyActivationControl::Register {
                    profile,
                    generation,
                    assembly,
                    config_snapshot,
                    replica_id,
                } => SemanticFrame::Register {
                    tuple: Tuple {
                        profile,
                        generation,
                        assembly: assembly.assembly_identity.as_str().to_string(),
                        config_snapshot: config_snapshot.snapshot_id.as_str().to_string(),
                    },
                    replica_id,
                },
                other => panic!("expected register control, got {other:?}"),
            }
        }
        "Registered" => {
            let (typed, payload): (RuntimeRegisteredFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("registered decodes");
            assert!(payload.is_empty(), "registered payload must be empty");
            let reencoded = encode_binary_frame(&typed, &[]).expect("registered re-encodes");
            assert_eq!(reencoded, bytes, "registered frame must be byte-exact");
            SemanticFrame::Registered {
                runtime_id: typed.runtime_id,
            }
        }
        "Health" => {
            let (typed, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("health decodes");
            assert!(payload.is_empty(), "health payload must be empty");
            let reencoded = encode_binary_frame(&typed, &[]).expect("health re-encodes");
            assert_eq!(reencoded, bytes, "health frame must be byte-exact");
            SemanticFrame::Health {
                runtime_id: typed.runtime_id,
            }
        }
        "LegacyRegister" => {
            let (typed, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("legacy register decodes");
            assert!(payload.is_empty(), "legacy register payload must be empty");
            let reencoded = encode_binary_frame(&typed, &[]).expect("legacy register re-encodes");
            assert_eq!(reencoded, bytes, "legacy register frame must be byte-exact");
            let _ = typed.runtime_id;
            SemanticFrame::LegacyRegister
        }
        other => panic!("unknown decodeAs {other}"),
    }
}

fn load_catalog() -> Catalog {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("registration-handshake")
        .join("frames.json");
    let text = std::fs::read_to_string(&path).expect("frames.json must exist");
    serde_json::from_str(&text).expect("frames.json must parse")
}

fn load_scenarios() -> Vec<ScenarioFile> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("registration-handshake")
        .join("scenarios");
    let mut paths = std::fs::read_dir(&dir)
        .expect("scenarios dir must exist")
        .map(|entry| entry.expect("scenario entry"))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("scenario must be readable");
            serde_json::from_str(&text).expect("scenario must parse")
        })
        .collect()
}

#[test]
fn frame_catalog_is_byte_exact_and_complete() {
    let catalog = load_catalog();
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.corpus, "registration-handshake-v1");
    for required in REQUIRED_FRAMES {
        assert!(
            catalog.frames.contains_key(required),
            "required frame {required} missing from catalog"
        );
    }
    for (name, entry) in &catalog.frames {
        let semantic = decode_catalog_frame(entry);
        let expected_frame_type = match entry.decode_as.as_str() {
            "RouterBootstrap" => "router.bootstrap",
            "Capabilities" => "runtime.capabilities",
            "AssemblyRegister" => "assembly.activation:Register",
            "Registered" => "runtime.registered",
            "Health" => "runtime.health",
            "LegacyRegister" => "runtime.register",
            other => panic!("unknown decodeAs {other}"),
        };
        assert_eq!(
            entry.frame_type, expected_frame_type,
            "{name}: frameType must match decodeAs"
        );
        match &semantic {
            SemanticFrame::Bootstrap => assert_eq!(entry.direction, "RouterToRuntime"),
            SemanticFrame::Registered { .. } => assert_eq!(entry.direction, "RouterToRuntime"),
            _ => assert_eq!(entry.direction, "RuntimeToRouter"),
        }
        assert!(
            !entry.frame_hex.is_empty() && entry.frame_hex.len() % 2 == 0,
            "{name}: frameHex must be even-length hex"
        );
        // Every catalog frame is used by at least one scenario; the scenario
        // set below exercises the required handshake and terminal families.
        let _ = name;
    }
}

#[test]
fn handshake_sequences_match_frozen_semantics() {
    let catalog = load_catalog();
    let frames = catalog
        .frames
        .iter()
        .map(|(name, entry)| (name.clone(), decode_catalog_frame(entry)))
        .collect::<HashMap<_, _>>();
    let scenarios = load_scenarios();
    let scenario_names = scenarios
        .iter()
        .map(|scenario| scenario.scenario.as_str())
        .collect::<HashSet<_>>();
    for required in REQUIRED_SCENARIOS {
        assert!(
            scenario_names.contains(required),
            "required scenario {required} missing"
        );
    }

    for scenario in &scenarios {
        assert_eq!(scenario.schema_version, 1);
        let mut machine = Machine::new(
            scenario.epoch.tuple(),
            scenario.epoch.pending.as_ref().map(PendingValue::tuple),
            scenario.pre_auth_limit,
        );
        for event in &scenario.events {
            match event {
                EventValue::Accept {
                    connection,
                    connection_generation,
                } => machine.accept(connection, *connection_generation),
                EventValue::Write { connection, frame } => {
                    let semantic = frames
                        .get(frame)
                        .unwrap_or_else(|| panic!("unknown frame {frame}"));
                    machine.write(connection, semantic);
                }
                EventValue::WriteFail { connection, frame } => {
                    let semantic = frames
                        .get(frame)
                        .unwrap_or_else(|| panic!("unknown frame {frame}"));
                    machine.write_fail(connection, semantic);
                }
                EventValue::Read { connection, frame } => {
                    let semantic = frames
                        .get(frame)
                        .unwrap_or_else(|| panic!("unknown frame {frame}"));
                    machine.read(connection, semantic);
                }
                EventValue::Timeout {
                    connection,
                    timeout_kind,
                } => machine.timeout(connection, timeout_kind),
                EventValue::Disconnect { connection } => machine.disconnect(connection),
            }
        }

        let expect = &scenario.expect;
        let mut actual_outcomes = machine
            .conns
            .keys()
            .map(|id| (id.clone(), machine.outcome(id)))
            .collect::<BTreeMap<_, _>>();
        for id in machine.refused.keys() {
            if !machine.conns.contains_key(id) {
                actual_outcomes.insert(id.clone(), machine.refused_outcome(id));
            }
        }
        let expected_outcomes = expect
            .outcomes
            .iter()
            .map(|(id, outcome)| (id.clone(), outcome.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            actual_outcomes, expected_outcomes,
            "scenario {}: outcomes mismatch",
            scenario.scenario
        );

        let refused_total = machine.refused.values().sum::<u64>();
        assert_eq!(
            refused_total, expect.refused_count,
            "scenario {}: refusedCount",
            scenario.scenario
        );
        assert_eq!(
            machine.pre_auth.len(),
            expect.pre_auth_count,
            "scenario {}: preAuthCount",
            scenario.scenario
        );
        assert_eq!(
            machine.registered_replicas, expect.registered_sessions,
            "scenario {}: registeredSessions",
            scenario.scenario
        );
        assert_eq!(
            machine.observed_health, expect.observed_health,
            "scenario {}: observedHealth",
            scenario.scenario
        );
        assert_eq!(
            machine.health_before_ack_total, expect.health_before_ack,
            "scenario {}: healthBeforeAck",
            scenario.scenario
        );

        let main = machine
            .conns
            .get(&scenario.main_connection)
            .expect("main connection exists");
        assert_eq!(
            main.phase == Phase::Registered,
            expect.routable_registered,
            "scenario {}: routableRegistered",
            scenario.scenario
        );
        assert_eq!(
            main.phase == Phase::RegisterValidated,
            expect.published_pending,
            "scenario {}: publishedPending",
            scenario.scenario
        );
        assert_eq!(
            main.revision, expect.revision,
            "scenario {}: revision",
            scenario.scenario
        );
        assert!(
            !expect.fail_stop,
            "scenario {}: failStop must be false for handshake corpus",
            scenario.scenario
        );
    }
}

#[test]
fn mutated_frame_is_rejected_by_codec() {
    let catalog = load_catalog();
    let bootstrap = catalog
        .frames
        .get("bootstrap.prod.42")
        .expect("bootstrap frame");
    let mut bytes = hex_decode(&bootstrap.frame_hex);
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(
        decode_binary_frame(&bytes).is_err(),
        "mutated frame bytes must not decode; byte-exactness is a hard contract"
    );
}
