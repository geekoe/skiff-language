//! Runtime-owned recoverable views for execution diagnostics.
//!
//! These structs are linked/runtime data structures, not the persisted
//! recoverable artifact schema. The durable compiler/runtime contract lives in
//! `skiff_artifact_model::recoverable` and must be bridged explicitly.

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan};

pub const RECOVERABLE_ENVELOPE_SCHEMA_VERSION: &str = "skiff.recoverable.envelope.v2";

const RECOVERABLE_MAGIC: &[u8; 4] = b"SKRE";
const RECOVERABLE_BINARY_VERSION: u8 = 1;
const RECOVERABLE_MIN_DATE_EPOCH_MILLIS: i64 = -62_167_219_200_000;
const RECOVERABLE_MAX_DATE_EPOCH_MILLIS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableValidationLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_encoded_bytes: usize,
}

impl Default for RecoverableValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_depth: 512,
            max_encoded_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("recoverable state invalid at {path}: {message}")]
pub struct RecoverableStateInvalid {
    path: String,
    message: String,
}

impl RecoverableStateInvalid {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageCoordinate {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableEnvelope {
    pub schema_version: String,
    pub root: RecoverableNode,
}

impl RecoverableEnvelope {
    pub fn new(root: RecoverableNode) -> Self {
        Self {
            schema_version: RECOVERABLE_ENVELOPE_SCHEMA_VERSION.to_string(),
            root,
        }
    }

    pub fn validate(&self, limits: &RecoverableValidationLimits) -> RecoverableValidationResult {
        if self.schema_version != RECOVERABLE_ENVELOPE_SCHEMA_VERSION {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!(
                    "unsupported recoverable schema version {}",
                    self.schema_version
                ),
            ));
        }
        let mut validator = RecoverableValidator {
            limits,
            node_count: 0,
        };
        validator.validate_node(&self.root, "$")
    }

    pub fn to_canonical_bytes(
        &self,
        limits: &RecoverableValidationLimits,
    ) -> RecoverableValidationResult<Vec<u8>> {
        self.validate(limits)?;
        let mut encoder = RecoverableCanonicalEncoder {
            output: Vec::with_capacity(256),
            limits,
        };
        encoder.output.extend_from_slice(RECOVERABLE_MAGIC);
        encoder.output.push(RECOVERABLE_BINARY_VERSION);
        encoder.write_string(&self.schema_version, "$.schemaVersion")?;
        encoder.write_node(&self.root, "$.root", 0)?;
        encoder.finish()
    }

    pub fn from_canonical_bytes(
        bytes: &[u8],
        limits: &RecoverableValidationLimits,
    ) -> RecoverableValidationResult<Self> {
        if bytes.len() > limits.max_encoded_bytes {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!(
                    "recoverable envelope exceeds max encoded bytes {}",
                    limits.max_encoded_bytes
                ),
            ));
        }
        let mut decoder = RecoverableCanonicalDecoder {
            input: bytes,
            offset: 0,
            limits,
            node_count: 0,
        };
        decoder.expect_magic()?;
        let schema_version = decoder.read_string("$.schemaVersion")?;
        if schema_version != RECOVERABLE_ENVELOPE_SCHEMA_VERSION {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!("unsupported recoverable schema version {schema_version}"),
            ));
        }
        let root = decoder.read_node("$.root", 0)?;
        if decoder.offset != bytes.len() {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!(
                    "recoverable envelope has {} trailing byte(s)",
                    bytes.len() - decoder.offset
                ),
            ));
        }
        let envelope = Self {
            schema_version,
            root,
        };
        envelope.validate(limits)?;
        Ok(envelope)
    }

    pub fn collect_artifact_refs(&self) -> Vec<RecoverableArtifactRef> {
        let mut collector = RecoverableArtifactCollector::default();
        collector.collect_node(&self.root, "$.root");
        collector.into_refs()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableNode {
    pub value_kind: RecoverableValueKind,
    pub variant_identity: RecoverableVariantIdentity,
    pub code_identity: RecoverableCodeIdentity,
    pub state: RecoverableState,
}

impl RecoverableNode {
    pub fn plain(value_kind: RecoverableValueKind, state: RecoverableState) -> Self {
        Self {
            value_kind,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::None,
            state,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoverableValueKind {
    Null,
    Bool,
    Number,
    String,
    Bytes,
    Date,
    Array,
    Map,
    Record,
    NominalObject,
    InterfaceValue,
    NativeHandle,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecoverableVariantIdentity {
    None,
    UnionBranch {
        union_identity: String,
        branch_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalConcreteOwner {
    Service,
    Package { package_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConcreteRestoreKey {
    pub owner: LocalConcreteOwner,
    pub concrete_type_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecoverableCodeIdentity {
    None,
    LocalConcrete {
        owner: LocalConcreteOwner,
        concrete_type_identity: String,
    },
    NativeAdapter {
        adapter_identity: String,
        adapter_schema_version: String,
        owner: NativeAdapterOwner,
        native_type_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NativeAdapterOwner {
    Builtin,
    Artifact {
        artifact_identity: String,
        build_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<PackageCoordinate>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableNumber {
    finite_f64_le_bits: u64,
}

impl RecoverableNumber {
    pub fn try_from_f64(value: f64) -> RecoverableValidationResult<Self> {
        if !value.is_finite() {
            return Err(RecoverableStateInvalid::new(
                "$",
                "recoverable number must be finite",
            ));
        }
        Ok(Self {
            finite_f64_le_bits: value.to_bits(),
        })
    }

    pub fn to_f64(self) -> f64 {
        f64::from_bits(self.finite_f64_le_bits)
    }

    pub fn bits(self) -> u64 {
        self.finite_f64_le_bits
    }

    pub fn from_bits(bits: u64, path: &str) -> RecoverableValidationResult<Self> {
        let number = Self {
            finite_f64_le_bits: bits,
        };
        if !number.to_f64().is_finite() {
            return Err(RecoverableStateInvalid::new(
                path,
                "recoverable number must be finite",
            ));
        }
        Ok(number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableDate {
    pub epoch_millis: i64,
}

impl RecoverableDate {
    pub fn new(epoch_millis: i64) -> RecoverableValidationResult<Self> {
        validate_recoverable_date(epoch_millis, "$")?;
        Ok(Self { epoch_millis })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum RecoverableState {
    Null,
    Bool(bool),
    Number(RecoverableNumber),
    String(String),
    Bytes(Vec<u8>),
    Date(RecoverableDate),
    Array(Vec<RecoverableNode>),
    Map(Vec<(RecoverableMapKey, RecoverableNode)>),
    Record(Vec<RecoverableField>),
    NominalObject(NominalObjectState),
    InterfaceValue(InterfaceValueState),
    NativeHandle(NativeHandleState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableField {
    pub field_identity: String,
    pub value: RecoverableNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InterfaceValueState {
    Local { self_node: Box<RecoverableNode> },
    Remote {
        carrier: RecoverableRemoteInterfaceCarrier,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRemoteInterfaceCarrier {
    pub dependency_ref: String,
    pub public_instance_key: String,
    pub operations: RecoverableRemoteOperationTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRemoteOperationTable {
    pub id: String,
    pub interface_abi_id: String,
    pub slots: Vec<RecoverableRemoteOperationSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRemoteOperationSlot {
    pub slot: u32,
    pub method_abi_id: String,
    pub operation_abi_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NominalObjectState {
    DefaultFields { fields: Vec<RecoverableField> },
    Custom { durable_state: Box<RecoverableNode> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHandleState {
    pub durable_state: Box<RecoverableNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecoverableMapKey {
    String(String),
    NominalRepresentation {
        representation_identity: String,
        value: Box<RecoverableMapKey>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableArtifactRef {
    pub artifact_identity: String,
    pub build_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageCoordinate>,
    pub node_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableArtifactRetentionRoot {
    pub service_id: String,
    pub artifact_identity: String,
    pub build_id: String,
    pub boundary_kind: RuntimeRecoverableBoundaryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_epoch_millis: Option<i64>,
}

pub type RecoverableValidationResult<T = ()> = Result<T, RecoverableStateInvalid>;

#[derive(Default)]
struct RecoverableArtifactCollector {
    refs_by_key:
        BTreeMap<(String, String, Option<PackageCoordinate>, String), RecoverableArtifactRef>,
}

impl RecoverableArtifactCollector {
    fn collect_node(&mut self, node: &RecoverableNode, path: &str) {
        match &node.code_identity {
            RecoverableCodeIdentity::LocalConcrete { .. } => {}
            RecoverableCodeIdentity::NativeAdapter {
                owner:
                    NativeAdapterOwner::Artifact {
                        artifact_identity,
                        build_id,
                        package,
                    },
                ..
            } => self.insert_ref(artifact_identity, build_id, package, path),
            RecoverableCodeIdentity::None
            | RecoverableCodeIdentity::NativeAdapter {
                owner: NativeAdapterOwner::Builtin,
                ..
            } => {}
        }
        match &node.state {
            RecoverableState::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.collect_node(item, &format!("{path}[{index}]"));
                }
            }
            RecoverableState::Map(entries) => {
                for (index, (_key, value)) in entries.iter().enumerate() {
                    self.collect_node(value, &format!("{path}.map[{index}]"));
                }
            }
            RecoverableState::Record(fields) => {
                for field in fields {
                    self.collect_node(
                        &field.value,
                        &format!("{path}.field({})", field.field_identity),
                    );
                }
            }
            RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) => {
                for field in fields {
                    self.collect_node(
                        &field.value,
                        &format!("{path}.field({})", field.field_identity),
                    );
                }
            }
            RecoverableState::NominalObject(NominalObjectState::Custom {
                durable_state, ..
            }) => {
                self.collect_node(durable_state, &format!("{path}.durableState"));
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Local { self_node }) => {
                self.collect_node(self_node, &format!("{path}.selfNode"));
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Remote { .. }) => {}
            RecoverableState::NativeHandle(state) => {
                self.collect_node(&state.durable_state, &format!("{path}.durableState"));
            }
            RecoverableState::Null
            | RecoverableState::Bool(_)
            | RecoverableState::Number(_)
            | RecoverableState::String(_)
            | RecoverableState::Bytes(_)
            | RecoverableState::Date(_) => {}
        }
    }

    fn insert_ref(
        &mut self,
        artifact_identity: &str,
        build_id: &str,
        package: &Option<PackageCoordinate>,
        path: &str,
    ) {
        let key = (
            artifact_identity.to_string(),
            build_id.to_string(),
            package.clone(),
            path.to_string(),
        );
        self.refs_by_key
            .entry(key)
            .or_insert_with(|| RecoverableArtifactRef {
                artifact_identity: artifact_identity.to_string(),
                build_id: build_id.to_string(),
                package: package.clone(),
                node_path: path.to_string(),
            });
    }

    fn into_refs(self) -> Vec<RecoverableArtifactRef> {
        self.refs_by_key.into_values().collect()
    }
}

struct RecoverableValidator<'a> {
    limits: &'a RecoverableValidationLimits,
    node_count: usize,
}

impl RecoverableValidator<'_> {
    fn validate_node(&mut self, node: &RecoverableNode, path: &str) -> RecoverableValidationResult {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(RecoverableStateInvalid::new(
                path,
                format!("recoverable node count exceeds {}", self.limits.max_nodes),
            ));
        }
        self.validate_state_matches_kind(node, path)?;
        self.validate_state(&node.state, path, 0)
    }

    fn validate_child(
        &mut self,
        node: &RecoverableNode,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        if depth > self.limits.max_depth {
            return Err(RecoverableStateInvalid::new(
                path,
                format!("recoverable depth exceeds {}", self.limits.max_depth),
            ));
        }
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(RecoverableStateInvalid::new(
                path,
                format!("recoverable node count exceeds {}", self.limits.max_nodes),
            ));
        }
        self.validate_state_matches_kind(node, path)?;
        self.validate_state(&node.state, path, depth)
    }

    fn validate_state_matches_kind(
        &self,
        node: &RecoverableNode,
        path: &str,
    ) -> RecoverableValidationResult {
        let state_kind = match &node.state {
            RecoverableState::Null => RecoverableValueKind::Null,
            RecoverableState::Bool(_) => RecoverableValueKind::Bool,
            RecoverableState::Number(_) => RecoverableValueKind::Number,
            RecoverableState::String(_) => RecoverableValueKind::String,
            RecoverableState::Bytes(_) => RecoverableValueKind::Bytes,
            RecoverableState::Date(_) => RecoverableValueKind::Date,
            RecoverableState::Array(_) => RecoverableValueKind::Array,
            RecoverableState::Map(_) => RecoverableValueKind::Map,
            RecoverableState::Record(_) => RecoverableValueKind::Record,
            RecoverableState::NominalObject(_) => RecoverableValueKind::NominalObject,
            RecoverableState::InterfaceValue(_) => RecoverableValueKind::InterfaceValue,
            RecoverableState::NativeHandle(_) => RecoverableValueKind::NativeHandle,
        };
        if node.value_kind != state_kind {
            return Err(RecoverableStateInvalid::new(
                path,
                format!(
                    "recoverable value kind {:?} does not match state {:?}",
                    node.value_kind, state_kind
                ),
            ));
        }
        if matches!(node.state, RecoverableState::InterfaceValue(_))
            && !matches!(node.code_identity, RecoverableCodeIdentity::None)
        {
            return Err(RecoverableStateInvalid::new(
                path,
                "InterfaceValue wrapper must not carry code identity",
            ));
        }
        if matches!(node.state, RecoverableState::NativeHandle(_))
            && !matches!(
                node.code_identity,
                RecoverableCodeIdentity::NativeAdapter { .. }
            )
        {
            return Err(RecoverableStateInvalid::new(
                path,
                "NativeHandleState requires NativeAdapter code identity",
            ));
        }
        Ok(())
    }

    fn validate_state(
        &mut self,
        state: &RecoverableState,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        match state {
            RecoverableState::Number(number) => {
                RecoverableNumber::from_bits(number.bits(), path)?;
            }
            RecoverableState::Date(date) => validate_recoverable_date(date.epoch_millis, path)?,
            RecoverableState::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.validate_child(item, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
            RecoverableState::Map(entries) => {
                let mut keys = HashSet::new();
                for (index, (key, value)) in entries.iter().enumerate() {
                    let key_bytes = canonical_map_key_bytes(
                        key,
                        self.limits,
                        &format!("{path}.mapKey[{index}]"),
                        depth + 1,
                    )?;
                    if !keys.insert(key_bytes) {
                        return Err(RecoverableStateInvalid::new(
                            path,
                            "duplicate recoverable map key",
                        ));
                    }
                    self.validate_child(value, &format!("{path}.map[{index}]"), depth + 1)?;
                }
            }
            RecoverableState::Record(fields) => {
                self.validate_fields(fields, path, depth)?;
            }
            RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) => {
                self.validate_fields(fields, path, depth)?;
            }
            RecoverableState::NominalObject(NominalObjectState::Custom {
                durable_state, ..
            }) => {
                self.validate_child(durable_state, &format!("{path}.durableState"), depth + 1)?;
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Local { self_node }) => {
                self.validate_child(self_node, &format!("{path}.selfNode"), depth + 1)?;
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Remote { carrier }) => {
                validate_remote_interface_carrier(carrier, path)?;
            }
            RecoverableState::NativeHandle(state) => {
                self.validate_child(
                    &state.durable_state,
                    &format!("{path}.durableState"),
                    depth + 1,
                )?;
            }
            RecoverableState::Null
            | RecoverableState::Bool(_)
            | RecoverableState::String(_)
            | RecoverableState::Bytes(_) => {}
        }
        Ok(())
    }

    fn validate_fields(
        &mut self,
        fields: &[RecoverableField],
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        let mut identities = HashSet::new();
        for field in fields {
            if !identities.insert(field.field_identity.as_str()) {
                return Err(RecoverableStateInvalid::new(
                    path,
                    format!(
                        "duplicate recoverable field identity {}",
                        field.field_identity
                    ),
                ));
            }
            self.validate_child(
                &field.value,
                &format!("{path}.field({})", field.field_identity),
                depth + 1,
            )?;
        }
        Ok(())
    }
}

struct RecoverableCanonicalEncoder<'a> {
    output: Vec<u8>,
    limits: &'a RecoverableValidationLimits,
}

impl RecoverableCanonicalEncoder<'_> {
    fn finish(self) -> RecoverableValidationResult<Vec<u8>> {
        if self.output.len() > self.limits.max_encoded_bytes {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!(
                    "recoverable envelope exceeds max encoded bytes {}",
                    self.limits.max_encoded_bytes
                ),
            ));
        }
        Ok(self.output)
    }

    fn write_node(
        &mut self,
        node: &RecoverableNode,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        self.write_u8(value_kind_tag(node.value_kind));
        self.write_variant_identity(&node.variant_identity, path)?;
        self.write_code_identity(&node.code_identity, path)?;
        self.write_state(&node.state, path, depth)
    }

    fn write_state(
        &mut self,
        state: &RecoverableState,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        match state {
            RecoverableState::Null => {}
            RecoverableState::Bool(value) => self.write_u8(u8::from(*value)),
            RecoverableState::Number(number) => {
                self.output.extend_from_slice(&number.bits().to_le_bytes())
            }
            RecoverableState::String(value) => self.write_string(value, path)?,
            RecoverableState::Bytes(value) => self.write_bytes(value, path)?,
            RecoverableState::Date(date) => self
                .output
                .extend_from_slice(&date.epoch_millis.to_le_bytes()),
            RecoverableState::Array(items) => {
                self.write_len(items.len(), path)?;
                for (index, item) in items.iter().enumerate() {
                    self.write_node(item, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
            RecoverableState::Map(entries) => {
                let mut canonical_entries: Vec<_> = entries
                    .iter()
                    .enumerate()
                    .map(|(source_index, (key, value))| {
                        canonical_map_key_bytes(
                            key,
                            self.limits,
                            &format!("{path}.mapKey[{source_index}]"),
                            depth + 1,
                        )
                        .map(|key_bytes| (key_bytes, key, value))
                    })
                    .collect::<RecoverableValidationResult<Vec<_>>>()?;
                canonical_entries.sort_by(|left, right| left.0.cmp(&right.0));
                self.write_len(canonical_entries.len(), path)?;
                for (index, (_key_bytes, key, value)) in canonical_entries.into_iter().enumerate() {
                    self.write_map_key(key, &format!("{path}.mapKey[{index}]"), depth + 1)?;
                    self.write_node(value, &format!("{path}.map[{index}]"), depth + 1)?;
                }
            }
            RecoverableState::Record(fields) => self.write_fields(fields, path, depth)?,
            RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) => {
                self.write_u8(0);
                self.write_fields(fields, path, depth)?;
            }
            RecoverableState::NominalObject(NominalObjectState::Custom { durable_state }) => {
                self.write_u8(1);
                self.write_node(durable_state, &format!("{path}.durableState"), depth + 1)?;
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Local { self_node }) => {
                self.write_u8(0);
                self.write_node(self_node, &format!("{path}.selfNode"), depth + 1)?;
            }
            RecoverableState::InterfaceValue(InterfaceValueState::Remote { carrier }) => {
                self.write_u8(1);
                self.write_remote_interface_carrier(carrier, path)?;
            }
            RecoverableState::NativeHandle(state) => {
                self.write_node(
                    &state.durable_state,
                    &format!("{path}.durableState"),
                    depth + 1,
                )?;
            }
        }
        Ok(())
    }

    fn write_fields(
        &mut self,
        fields: &[RecoverableField],
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        let mut sorted: Vec<_> = fields.iter().collect();
        sorted.sort_by(|left, right| left.field_identity.cmp(&right.field_identity));
        self.write_len(sorted.len(), path)?;
        for field in sorted {
            self.write_string(&field.field_identity, path)?;
            self.write_node(
                &field.value,
                &format!("{path}.field({})", field.field_identity),
                depth + 1,
            )?;
        }
        Ok(())
    }

    fn write_variant_identity(
        &mut self,
        identity: &RecoverableVariantIdentity,
        path: &str,
    ) -> RecoverableValidationResult {
        match identity {
            RecoverableVariantIdentity::None => self.write_u8(0),
            RecoverableVariantIdentity::UnionBranch {
                union_identity,
                branch_identity,
            } => {
                self.write_u8(1);
                self.write_string(union_identity, path)?;
                self.write_string(branch_identity, path)?;
            }
        }
        Ok(())
    }

    fn write_code_identity(
        &mut self,
        identity: &RecoverableCodeIdentity,
        path: &str,
    ) -> RecoverableValidationResult {
        match identity {
            RecoverableCodeIdentity::None => self.write_u8(0),
            RecoverableCodeIdentity::LocalConcrete {
                owner,
                concrete_type_identity,
            } => {
                self.write_u8(1);
                self.write_local_concrete_owner(owner, path)?;
                self.write_string(concrete_type_identity, path)?;
            }
            RecoverableCodeIdentity::NativeAdapter {
                adapter_identity,
                adapter_schema_version,
                owner,
                native_type_identity,
            } => {
                self.write_u8(2);
                self.write_string(adapter_identity, path)?;
                self.write_string(adapter_schema_version, path)?;
                self.write_native_owner(owner, path)?;
                self.write_string(native_type_identity, path)?;
            }
        }
        Ok(())
    }

    fn write_native_owner(
        &mut self,
        owner: &NativeAdapterOwner,
        path: &str,
    ) -> RecoverableValidationResult {
        match owner {
            NativeAdapterOwner::Builtin => self.write_u8(0),
            NativeAdapterOwner::Artifact {
                artifact_identity,
                build_id,
                package,
            } => {
                self.write_u8(1);
                self.write_string(artifact_identity, path)?;
                self.write_string(build_id, path)?;
                self.write_package(package, path)?;
            }
        }
        Ok(())
    }

    fn write_local_concrete_owner(
        &mut self,
        owner: &LocalConcreteOwner,
        path: &str,
    ) -> RecoverableValidationResult {
        match owner {
            LocalConcreteOwner::Service => self.write_u8(0),
            LocalConcreteOwner::Package { package_id } => {
                self.write_u8(1);
                self.write_string(package_id, path)?;
            }
        }
        Ok(())
    }

    fn write_package(
        &mut self,
        package: &Option<PackageCoordinate>,
        path: &str,
    ) -> RecoverableValidationResult {
        match package {
            Some(package) => {
                self.write_u8(1);
                self.write_string(&package.name, path)?;
                self.write_string(&package.version, path)?;
            }
            None => self.write_u8(0),
        }
        Ok(())
    }

    fn write_map_key(
        &mut self,
        key: &RecoverableMapKey,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult {
        validate_recoverable_depth(self.limits, path, depth)?;
        match key {
            RecoverableMapKey::String(value) => {
                self.write_u8(0);
                self.write_string(value, path)?;
            }
            RecoverableMapKey::NominalRepresentation {
                representation_identity,
                value,
            } => {
                self.write_u8(1);
                self.write_string(representation_identity, path)?;
                self.write_map_key(value, path, depth + 1)?;
            }
        }
        Ok(())
    }

    fn write_remote_interface_carrier(
        &mut self,
        carrier: &RecoverableRemoteInterfaceCarrier,
        path: &str,
    ) -> RecoverableValidationResult {
        self.write_string(&carrier.dependency_ref, &format!("{path}.dependencyRef"))?;
        self.write_string(
            &carrier.public_instance_key,
            &format!("{path}.publicInstanceKey"),
        )?;
        self.write_string(&carrier.operations.id, &format!("{path}.operations.id"))?;
        self.write_string(
            &carrier.operations.interface_abi_id,
            &format!("{path}.operations.interfaceAbiId"),
        )?;
        self.write_len(
            carrier.operations.slots.len(),
            &format!("{path}.operations.slots"),
        )?;
        for (index, slot) in carrier.operations.slots.iter().enumerate() {
            self.output.extend_from_slice(&slot.slot.to_le_bytes());
            self.write_string(
                &slot.method_abi_id,
                &format!("{path}.operations.slots[{index}].methodAbiId"),
            )?;
            self.write_string(
                &slot.operation_abi_id,
                &format!("{path}.operations.slots[{index}].operationAbiId"),
            )?;
        }
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8], path: &str) -> RecoverableValidationResult {
        self.write_len(bytes.len(), path)?;
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    fn write_string(&mut self, value: &str, path: &str) -> RecoverableValidationResult {
        self.write_bytes(value.as_bytes(), path)
    }

    fn write_len(&mut self, len: usize, path: &str) -> RecoverableValidationResult {
        let len = u32::try_from(len).map_err(|_| {
            RecoverableStateInvalid::new(path, "recoverable length exceeds u32::MAX")
        })?;
        self.output.extend_from_slice(&len.to_le_bytes());
        Ok(())
    }

    fn write_u8(&mut self, value: u8) {
        self.output.push(value);
    }
}

struct RecoverableCanonicalDecoder<'a> {
    input: &'a [u8],
    offset: usize,
    limits: &'a RecoverableValidationLimits,
    node_count: usize,
}

impl RecoverableCanonicalDecoder<'_> {
    fn expect_magic(&mut self) -> RecoverableValidationResult {
        let magic = self.read_exact(4, "$")?;
        if magic != RECOVERABLE_MAGIC {
            return Err(RecoverableStateInvalid::new(
                "$",
                "recoverable envelope bytes missing SKRE magic",
            ));
        }
        let version = self.read_u8("$")?;
        if version != RECOVERABLE_BINARY_VERSION {
            return Err(RecoverableStateInvalid::new(
                "$",
                format!("unsupported recoverable binary version {version}"),
            ));
        }
        Ok(())
    }

    fn read_node(
        &mut self,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult<RecoverableNode> {
        self.enter_node(path, depth)?;
        let value_kind = recoverable_value_kind_from_tag(self.read_u8(path)?, path)?;
        let variant_identity = self.read_variant_identity(path)?;
        let code_identity = self.read_code_identity(path)?;
        let state = self.read_state(value_kind, path, depth)?;
        Ok(RecoverableNode {
            value_kind,
            variant_identity,
            code_identity,
            state,
        })
    }

    fn enter_node(&mut self, path: &str, depth: usize) -> RecoverableValidationResult {
        if depth > self.limits.max_depth {
            return Err(RecoverableStateInvalid::new(
                path,
                format!("recoverable depth exceeds {}", self.limits.max_depth),
            ));
        }
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(RecoverableStateInvalid::new(
                path,
                format!("recoverable node count exceeds {}", self.limits.max_nodes),
            ));
        }
        Ok(())
    }

    fn ensure_child_node_budget(&self, len: usize, path: &str) -> RecoverableValidationResult {
        let remaining = self.limits.max_nodes.saturating_sub(self.node_count);
        if len > remaining {
            return Err(RecoverableStateInvalid::new(
                path,
                format!(
                    "recoverable container declares {len} child node(s), exceeding remaining node budget {remaining}"
                ),
            ));
        }
        let remaining_bytes = self.input.len().saturating_sub(self.offset);
        if len > remaining_bytes {
            return Err(RecoverableStateInvalid::new(
                path,
                format!(
                    "recoverable container declares {len} child node(s), exceeding remaining encoded bytes {remaining_bytes}"
                ),
            ));
        }
        Ok(())
    }

    fn ensure_depth(&self, path: &str, depth: usize) -> RecoverableValidationResult {
        validate_recoverable_depth(self.limits, path, depth)
    }

    fn read_state(
        &mut self,
        value_kind: RecoverableValueKind,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult<RecoverableState> {
        match value_kind {
            RecoverableValueKind::Null => Ok(RecoverableState::Null),
            RecoverableValueKind::Bool => match self.read_u8(path)? {
                0 => Ok(RecoverableState::Bool(false)),
                1 => Ok(RecoverableState::Bool(true)),
                value => Err(RecoverableStateInvalid::new(
                    path,
                    format!("recoverable bool payload must be 0 or 1, got {value}"),
                )),
            },
            RecoverableValueKind::Number => Ok(RecoverableState::Number(
                RecoverableNumber::from_bits(self.read_u64(path)?, path)?,
            )),
            RecoverableValueKind::String => Ok(RecoverableState::String(self.read_string(path)?)),
            RecoverableValueKind::Bytes => Ok(RecoverableState::Bytes(self.read_bytes(path)?)),
            RecoverableValueKind::Date => {
                let epoch_millis = self.read_i64(path)?;
                validate_recoverable_date(epoch_millis, path)?;
                Ok(RecoverableState::Date(RecoverableDate { epoch_millis }))
            }
            RecoverableValueKind::Array => {
                let len = self.read_len(path)?;
                self.ensure_child_node_budget(len, path)?;
                let mut items = Vec::with_capacity(len);
                for index in 0..len {
                    items.push(self.read_node(&format!("{path}[{index}]"), depth + 1)?);
                }
                Ok(RecoverableState::Array(items))
            }
            RecoverableValueKind::Map => {
                let len = self.read_len(path)?;
                self.ensure_child_node_budget(len, path)?;
                let mut entries = Vec::with_capacity(len);
                let mut previous_key: Option<Vec<u8>> = None;
                for index in 0..len {
                    let key = self.read_map_key(&format!("{path}.mapKey[{index}]"), depth + 1)?;
                    let key_bytes = canonical_map_key_bytes(
                        &key,
                        self.limits,
                        &format!("{path}.mapKey[{index}]"),
                        depth + 1,
                    )?;
                    if previous_key
                        .as_ref()
                        .is_some_and(|previous| previous >= &key_bytes)
                    {
                        return Err(RecoverableStateInvalid::new(
                            path,
                            "recoverable map entries are not in canonical key order",
                        ));
                    }
                    previous_key = Some(key_bytes);
                    let value = self.read_node(&format!("{path}.map[{index}]"), depth + 1)?;
                    entries.push((key, value));
                }
                Ok(RecoverableState::Map(entries))
            }
            RecoverableValueKind::Record => {
                Ok(RecoverableState::Record(self.read_fields(path, depth)?))
            }
            RecoverableValueKind::NominalObject => match self.read_u8(path)? {
                0 => Ok(RecoverableState::NominalObject(
                    NominalObjectState::DefaultFields {
                        fields: self.read_fields(path, depth)?,
                    },
                )),
                1 => {
                    let durable_state =
                        Box::new(self.read_node(&format!("{path}.durableState"), depth + 1)?);
                    Ok(RecoverableState::NominalObject(
                        NominalObjectState::Custom { durable_state },
                    ))
                }
                tag => Err(RecoverableStateInvalid::new(
                    path,
                    format!("unknown nominal object state tag {tag}"),
                )),
            },
            RecoverableValueKind::InterfaceValue => match self.read_u8(path)? {
                0 => {
                    let self_node =
                        Box::new(self.read_node(&format!("{path}.selfNode"), depth + 1)?);
                    Ok(RecoverableState::InterfaceValue(InterfaceValueState::Local {
                        self_node,
                    }))
                }
                1 => Ok(RecoverableState::InterfaceValue(InterfaceValueState::Remote {
                    carrier: self.read_remote_interface_carrier(path)?,
                })),
                tag => Err(RecoverableStateInvalid::new(
                    path,
                    format!("unknown interface value state tag {tag}"),
                )),
            },
            RecoverableValueKind::NativeHandle => {
                let durable_state =
                    Box::new(self.read_node(&format!("{path}.durableState"), depth + 1)?);
                Ok(RecoverableState::NativeHandle(NativeHandleState {
                    durable_state,
                }))
            }
        }
    }

    fn read_fields(
        &mut self,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult<Vec<RecoverableField>> {
        let len = self.read_len(path)?;
        self.ensure_child_node_budget(len, path)?;
        let mut fields = Vec::with_capacity(len);
        let mut previous: Option<String> = None;
        for _ in 0..len {
            let field_identity = self.read_string(path)?;
            if previous
                .as_ref()
                .is_some_and(|previous| previous >= &field_identity)
            {
                return Err(RecoverableStateInvalid::new(
                    path,
                    "recoverable record fields are not in canonical identity order",
                ));
            }
            previous = Some(field_identity.clone());
            let value = self.read_node(&format!("{path}.field({field_identity})"), depth + 1)?;
            fields.push(RecoverableField {
                field_identity,
                value,
            });
        }
        Ok(fields)
    }

    fn read_variant_identity(
        &mut self,
        path: &str,
    ) -> RecoverableValidationResult<RecoverableVariantIdentity> {
        match self.read_u8(path)? {
            0 => Ok(RecoverableVariantIdentity::None),
            1 => Ok(RecoverableVariantIdentity::UnionBranch {
                union_identity: self.read_string(path)?,
                branch_identity: self.read_string(path)?,
            }),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown recoverable variant identity tag {tag}"),
            )),
        }
    }

    fn read_code_identity(
        &mut self,
        path: &str,
    ) -> RecoverableValidationResult<RecoverableCodeIdentity> {
        match self.read_u8(path)? {
            0 => Ok(RecoverableCodeIdentity::None),
            1 => Ok(RecoverableCodeIdentity::LocalConcrete {
                owner: self.read_local_concrete_owner(path)?,
                concrete_type_identity: self.read_string(path)?,
            }),
            2 => Ok(RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: self.read_string(path)?,
                adapter_schema_version: self.read_string(path)?,
                owner: self.read_native_owner(path)?,
                native_type_identity: self.read_string(path)?,
            }),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown recoverable code identity tag {tag}"),
            )),
        }
    }

    fn read_native_owner(&mut self, path: &str) -> RecoverableValidationResult<NativeAdapterOwner> {
        match self.read_u8(path)? {
            0 => Ok(NativeAdapterOwner::Builtin),
            1 => Ok(NativeAdapterOwner::Artifact {
                artifact_identity: self.read_string(path)?,
                build_id: self.read_string(path)?,
                package: self.read_package(path)?,
            }),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown native adapter owner tag {tag}"),
            )),
        }
    }

    fn read_local_concrete_owner(
        &mut self,
        path: &str,
    ) -> RecoverableValidationResult<LocalConcreteOwner> {
        match self.read_u8(path)? {
            0 => Ok(LocalConcreteOwner::Service),
            1 => Ok(LocalConcreteOwner::Package {
                package_id: self.read_string(path)?,
            }),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown local concrete owner tag {tag}"),
            )),
        }
    }

    fn read_package(
        &mut self,
        path: &str,
    ) -> RecoverableValidationResult<Option<PackageCoordinate>> {
        match self.read_u8(path)? {
            0 => Ok(None),
            1 => Ok(Some(PackageCoordinate {
                name: self.read_string(path)?,
                version: self.read_string(path)?,
            })),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown package coordinate tag {tag}"),
            )),
        }
    }

    fn read_map_key(
        &mut self,
        path: &str,
        depth: usize,
    ) -> RecoverableValidationResult<RecoverableMapKey> {
        self.ensure_depth(path, depth)?;
        match self.read_u8(path)? {
            0 => Ok(RecoverableMapKey::String(self.read_string(path)?)),
            1 => Ok(RecoverableMapKey::NominalRepresentation {
                representation_identity: self.read_string(path)?,
                value: Box::new(self.read_map_key(path, depth + 1)?),
            }),
            tag => Err(RecoverableStateInvalid::new(
                path,
                format!("unknown recoverable map key tag {tag}"),
            )),
        }
    }

    fn read_remote_interface_carrier(
        &mut self,
        path: &str,
    ) -> RecoverableValidationResult<RecoverableRemoteInterfaceCarrier> {
        let dependency_ref = self.read_string(&format!("{path}.dependencyRef"))?;
        let public_instance_key = self.read_string(&format!("{path}.publicInstanceKey"))?;
        let id = self.read_string(&format!("{path}.operations.id"))?;
        let interface_abi_id = self.read_string(&format!("{path}.operations.interfaceAbiId"))?;
        let len = self.read_len(&format!("{path}.operations.slots"))?;
        let mut slots = Vec::with_capacity(len);
        for index in 0..len {
            let slot_path = format!("{path}.operations.slots[{index}]");
            let slot = {
                let bytes = self.read_exact(4, &format!("{slot_path}.slot"))?;
                u32::from_le_bytes(bytes.try_into().expect("slice length checked"))
            };
            let method_abi_id = self.read_string(&format!("{slot_path}.methodAbiId"))?;
            let operation_abi_id = self.read_string(&format!("{slot_path}.operationAbiId"))?;
            slots.push(RecoverableRemoteOperationSlot {
                slot,
                method_abi_id,
                operation_abi_id,
            });
        }
        Ok(RecoverableRemoteInterfaceCarrier {
            dependency_ref,
            public_instance_key,
            operations: RecoverableRemoteOperationTable {
                id,
                interface_abi_id,
                slots,
            },
        })
    }

    fn read_string(&mut self, path: &str) -> RecoverableValidationResult<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes).map_err(|error| {
            RecoverableStateInvalid::new(path, format!("recoverable string is not UTF-8: {error}"))
        })
    }

    fn read_bytes(&mut self, path: &str) -> RecoverableValidationResult<Vec<u8>> {
        let len = self.read_len(path)?;
        Ok(self.read_exact(len, path)?.to_vec())
    }

    fn read_len(&mut self, path: &str) -> RecoverableValidationResult<usize> {
        let bytes = self.read_exact(4, path)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("slice length checked")) as usize)
    }

    fn read_i64(&mut self, path: &str) -> RecoverableValidationResult<i64> {
        let bytes = self.read_exact(8, path)?;
        Ok(i64::from_le_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_u64(&mut self, path: &str) -> RecoverableValidationResult<u64> {
        let bytes = self.read_exact(8, path)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_u8(&mut self, path: &str) -> RecoverableValidationResult<u8> {
        Ok(*self
            .read_exact(1, path)?
            .first()
            .expect("slice length checked"))
    }

    fn read_exact(&mut self, len: usize, path: &str) -> RecoverableValidationResult<&[u8]> {
        let end = self.offset.checked_add(len).ok_or_else(|| {
            RecoverableStateInvalid::new(path, "recoverable byte offset overflow")
        })?;
        if end > self.input.len() {
            return Err(RecoverableStateInvalid::new(
                path,
                "recoverable envelope ended unexpectedly",
            ));
        }
        let bytes = &self.input[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

fn value_kind_tag(kind: RecoverableValueKind) -> u8 {
    match kind {
        RecoverableValueKind::Null => 0,
        RecoverableValueKind::Bool => 1,
        RecoverableValueKind::Number => 2,
        RecoverableValueKind::String => 3,
        RecoverableValueKind::Bytes => 4,
        RecoverableValueKind::Date => 5,
        RecoverableValueKind::Array => 6,
        RecoverableValueKind::Map => 7,
        RecoverableValueKind::Record => 8,
        RecoverableValueKind::NominalObject => 9,
        RecoverableValueKind::InterfaceValue => 10,
        RecoverableValueKind::NativeHandle => 11,
    }
}

fn recoverable_value_kind_from_tag(
    tag: u8,
    path: &str,
) -> RecoverableValidationResult<RecoverableValueKind> {
    match tag {
        0 => Ok(RecoverableValueKind::Null),
        1 => Ok(RecoverableValueKind::Bool),
        2 => Ok(RecoverableValueKind::Number),
        3 => Ok(RecoverableValueKind::String),
        4 => Ok(RecoverableValueKind::Bytes),
        5 => Ok(RecoverableValueKind::Date),
        6 => Ok(RecoverableValueKind::Array),
        7 => Ok(RecoverableValueKind::Map),
        8 => Ok(RecoverableValueKind::Record),
        9 => Ok(RecoverableValueKind::NominalObject),
        10 => Ok(RecoverableValueKind::InterfaceValue),
        11 => Ok(RecoverableValueKind::NativeHandle),
        _ => Err(RecoverableStateInvalid::new(
            path,
            format!("unknown recoverable value kind tag {tag}"),
        )),
    }
}

fn canonical_map_key_bytes(
    key: &RecoverableMapKey,
    limits: &RecoverableValidationLimits,
    path: &str,
    depth: usize,
) -> RecoverableValidationResult<Vec<u8>> {
    let mut encoder = RecoverableCanonicalEncoder {
        output: Vec::with_capacity(32),
        limits,
    };
    encoder.write_map_key(key, path, depth)?;
    Ok(encoder.output)
}

fn validate_recoverable_depth(
    limits: &RecoverableValidationLimits,
    path: &str,
    depth: usize,
) -> RecoverableValidationResult {
    if depth > limits.max_depth {
        return Err(RecoverableStateInvalid::new(
            path,
            format!("recoverable depth exceeds {}", limits.max_depth),
        ));
    }
    Ok(())
}

fn validate_recoverable_date(epoch_millis: i64, path: &str) -> RecoverableValidationResult {
    if (RECOVERABLE_MIN_DATE_EPOCH_MILLIS..=RECOVERABLE_MAX_DATE_EPOCH_MILLIS)
        .contains(&epoch_millis)
    {
        Ok(())
    } else {
        Err(RecoverableStateInvalid::new(
            path,
            "recoverable Date is outside RFC3339 year range 0000..9999",
        ))
    }
}

fn validate_remote_interface_carrier(
    carrier: &RecoverableRemoteInterfaceCarrier,
    path: &str,
) -> RecoverableValidationResult {
    validate_non_empty_string(
        &carrier.dependency_ref,
        &format!("{path}.dependencyRef"),
        "remote InterfaceValue dependencyRef must be non-empty",
    )?;
    validate_non_empty_string(
        &carrier.public_instance_key,
        &format!("{path}.publicInstanceKey"),
        "remote InterfaceValue publicInstanceKey must be non-empty",
    )?;
    validate_non_empty_string(
        &carrier.operations.id,
        &format!("{path}.operations.id"),
        "remote InterfaceValue operation table id must be non-empty",
    )?;
    validate_non_empty_string(
        &carrier.operations.interface_abi_id,
        &format!("{path}.operations.interfaceAbiId"),
        "remote InterfaceValue operation table interfaceAbiId must be non-empty",
    )?;
    for (index, slot) in carrier.operations.slots.iter().enumerate() {
        let slot_path = format!("{path}.operations.slots[{index}]");
        if usize::try_from(slot.slot).ok() != Some(index) {
            return Err(RecoverableStateInvalid::new(
                &slot_path,
                "remote InterfaceValue operation slot index must match its canonical position",
            ));
        }
        validate_non_empty_string(
            &slot.method_abi_id,
            &format!("{slot_path}.methodAbiId"),
            "remote InterfaceValue operation slot methodAbiId must be non-empty",
        )?;
        validate_non_empty_string(
            &slot.operation_abi_id,
            &format!("{slot_path}.operationAbiId"),
            "remote InterfaceValue operation slot operationAbiId must be non-empty",
        )?;
    }
    Ok(())
}

fn validate_non_empty_string(
    value: &str,
    path: &str,
    message: &'static str,
) -> RecoverableValidationResult {
    if value.is_empty() {
        Err(RecoverableStateInvalid::new(path, message))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeRecoverableBoundaryKind {
    DbValue,
    SpawnPayload,
    QueueWorkItemPayload,
    RuntimeWirePayload,
    RuntimeBinaryPayload,
    ServicePayload,
    PublicApiPayload,
    MaterializationPayload,
    RecoverableEnvelopeSlot,
}

impl RuntimeRecoverableBoundaryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DbValue => "dbValue",
            Self::SpawnPayload => "spawnPayload",
            Self::QueueWorkItemPayload => "queueWorkItemPayload",
            Self::RuntimeWirePayload => "runtimeWirePayload",
            Self::RuntimeBinaryPayload => "runtimeBinaryPayload",
            Self::ServicePayload => "servicePayload",
            Self::PublicApiPayload => "publicApiPayload",
            Self::MaterializationPayload => "materializationPayload",
            Self::RecoverableEnvelopeSlot => "recoverableEnvelopeSlot",
        }
    }
}

impl fmt::Display for RuntimeRecoverableBoundaryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeRecoverableTrustBoundary {
    OwnerInternal,
    CrossService,
    ExternalUntrusted,
}

impl RuntimeRecoverableTrustBoundary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnerInternal => "ownerInternal",
            Self::CrossService => "crossService",
            Self::ExternalUntrusted => "externalUntrusted",
        }
    }
}

impl fmt::Display for RuntimeRecoverableTrustBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeRecoverableStorageLane {
    SchemaProjectable,
    RecoverableEnvelope,
}

impl RuntimeRecoverableStorageLane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaProjectable => "schemaProjectable",
            Self::RecoverableEnvelope => "recoverableEnvelope",
        }
    }
}

impl fmt::Display for RuntimeRecoverableStorageLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableServiceRef {
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
}

impl RuntimeRecoverableServiceRef {
    pub fn new(service_id: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            version: None,
            build_id: None,
        }
    }
}

/// Runtime boundary context used by fail-closed recoverable diagnostics.
///
/// This mirrors runtime execution state only; it is not the persisted artifact
/// boundary plan.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableBoundaryContext {
    pub kind: RuntimeRecoverableBoundaryKind,
    pub trust_boundary: RuntimeRecoverableTrustBoundary,
    pub storage_lane: RuntimeRecoverableStorageLane,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_service: Option<RuntimeRecoverableServiceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_service: Option<RuntimeRecoverableServiceRef>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explicit_recoverable_slot: bool,
}

impl RuntimeRecoverableBoundaryContext {
    pub fn new(
        kind: RuntimeRecoverableBoundaryKind,
        trust_boundary: RuntimeRecoverableTrustBoundary,
        storage_lane: RuntimeRecoverableStorageLane,
    ) -> Self {
        Self {
            kind,
            trust_boundary,
            storage_lane,
            origin_service: None,
            target_service: None,
            explicit_recoverable_slot: false,
        }
    }

    pub fn with_origin_service(mut self, service: RuntimeRecoverableServiceRef) -> Self {
        self.origin_service = Some(service);
        self
    }

    pub fn with_target_service(mut self, service: RuntimeRecoverableServiceRef) -> Self {
        self.target_service = Some(service);
        self
    }

    pub fn with_explicit_recoverable_slot(mut self) -> Self {
        self.explicit_recoverable_slot = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableNamedTypeRef {
    pub name: String,
}

impl RuntimeRecoverableNamedTypeRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableServiceTypeRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<RuntimeRecoverableServiceRef>,
    pub module_path: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverablePackageTypeRef {
    pub package_ref: String,
    pub symbol_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableArtifactTypeRef {
    pub artifact_identity: String,
    pub type_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableInterfaceTypeRef {
    pub interface_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableExpectedAnyInterfacePlan {
    pub interface_identity: String,
    pub method_projection_identity: String,
}

impl RuntimeRecoverableExpectedAnyInterfacePlan {
    pub fn new(
        interface_identity: impl Into<String>,
        method_projection_identity: impl Into<String>,
    ) -> Self {
        Self {
            interface_identity: interface_identity.into(),
            method_projection_identity: method_projection_identity.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum RuntimeRecoverableTypeIdentityRef {
    RuntimeNamedType(RuntimeRecoverableNamedTypeRef),
    ServiceSymbol(RuntimeRecoverableServiceTypeRef),
    PackageSymbol(RuntimeRecoverablePackageTypeRef),
    ArtifactType(RuntimeRecoverableArtifactTypeRef),
    Interface(RuntimeRecoverableInterfaceTypeRef),
}

/// Runtime expected type view used for diagnostics before envelope support.
///
/// Build this from runtime shapes only through the explicit shape-only helper;
/// artifact-authored recoverable contracts must use a separate bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableExpectedTypePlan {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<RuntimeRecoverableTypeIdentityRef>,
    pub node: RuntimeRecoverableExpectedTypeNode,
}

impl RuntimeRecoverableExpectedTypePlan {
    /// Build a shape-only runtime diagnostic view from `RuntimeTypePlan`.
    ///
    /// This helper does not create or imply a durable recoverable artifact
    /// contract. It exists only so fail-closed diagnostics can name the runtime
    /// shape until an explicit artifact-to-runtime recoverable bridge exists.
    pub fn from_runtime_type_plan_shape_only_for_diagnostics(plan: &RuntimeTypePlan) -> Self {
        Self {
            label: plan.label().to_string(),
            identity: plan.named_type_name().map(runtime_named_type_identity_ref),
            node: runtime_recoverable_expected_node_from_runtime_plan_for_diagnostics(plan),
        }
    }

    pub fn unresolved(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            label: label.clone(),
            identity: None,
            node: RuntimeRecoverableExpectedTypeNode::Unresolved {
                diagnostic_label: label,
            },
        }
    }

    pub fn any_interface(
        label: impl Into<String>,
        interface_identity: impl Into<String>,
        method_projection_identity: impl Into<String>,
    ) -> Self {
        let interface_identity = interface_identity.into();
        Self {
            label: label.into(),
            identity: Some(RuntimeRecoverableTypeIdentityRef::Interface(
                RuntimeRecoverableInterfaceTypeRef {
                    interface_identity: interface_identity.clone(),
                },
            )),
            node: RuntimeRecoverableExpectedTypeNode::AnyInterface {
                expected: RuntimeRecoverableExpectedAnyInterfacePlan::new(
                    interface_identity,
                    method_projection_identity,
                ),
            },
        }
    }

    pub fn diagnostic_label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeRecoverableExpectedTypeNode {
    Alias {
        target: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Nullable {
        inner: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Union {
        items: Vec<RuntimeRecoverableExpectedTypePlan>,
    },
    LiteralString {
        value: String,
    },
    Representation {
        identity: RuntimeRecoverableTypeIdentityRef,
        payload: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Json,
    JsonObject,
    Bytes,
    Date,
    String,
    Bool,
    Number,
    Integer,
    Null,
    Stream {
        item: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Array {
        item: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Map {
        key: Box<RuntimeRecoverableExpectedTypePlan>,
        value: Box<RuntimeRecoverableExpectedTypePlan>,
    },
    Record {
        fields: Vec<RuntimeRecoverableExpectedRecordFieldPlan>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boundary_record_kind: Option<String>,
    },
    AnyInterface {
        expected: RuntimeRecoverableExpectedAnyInterfacePlan,
    },
    Unresolved {
        diagnostic_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoverableExpectedRecordFieldPlan {
    pub name: String,
    pub ty: RuntimeRecoverableExpectedTypePlan,
    pub required: bool,
}

fn runtime_recoverable_expected_node_from_runtime_plan_for_diagnostics(
    plan: &RuntimeTypePlan,
) -> RuntimeRecoverableExpectedTypeNode {
    match plan.node() {
        RuntimeTypeNode::Alias(target) => RuntimeRecoverableExpectedTypeNode::Alias {
            target: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    target,
                ),
            ),
        },
        RuntimeTypeNode::Nullable(inner) => RuntimeRecoverableExpectedTypeNode::Nullable {
            inner: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    inner,
                ),
            ),
        },
        RuntimeTypeNode::Union(items) => RuntimeRecoverableExpectedTypeNode::Union {
            items: items
                .iter()
                .map(
                    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics,
                )
                .collect(),
        },
        RuntimeTypeNode::LiteralString(value) => RuntimeRecoverableExpectedTypeNode::LiteralString {
            value: value.clone(),
        },
        RuntimeTypeNode::Representation { type_name, payload } => {
            RuntimeRecoverableExpectedTypeNode::Representation {
                identity: runtime_named_type_identity_ref(type_name),
                payload: Box::new(
                    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                        payload,
                    ),
                ),
            }
        }
        RuntimeTypeNode::Json => RuntimeRecoverableExpectedTypeNode::Json,
        RuntimeTypeNode::JsonObject => RuntimeRecoverableExpectedTypeNode::JsonObject,
        RuntimeTypeNode::Bytes => RuntimeRecoverableExpectedTypeNode::Bytes,
        RuntimeTypeNode::Date => RuntimeRecoverableExpectedTypeNode::Date,
        RuntimeTypeNode::String => RuntimeRecoverableExpectedTypeNode::String,
        RuntimeTypeNode::Bool => RuntimeRecoverableExpectedTypeNode::Bool,
        RuntimeTypeNode::Number => RuntimeRecoverableExpectedTypeNode::Number,
        RuntimeTypeNode::Integer => RuntimeRecoverableExpectedTypeNode::Integer,
        RuntimeTypeNode::Null => RuntimeRecoverableExpectedTypeNode::Null,
        RuntimeTypeNode::Stream(item) => RuntimeRecoverableExpectedTypeNode::Stream {
            item: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    item,
                ),
            ),
        },
        RuntimeTypeNode::Array(item) => RuntimeRecoverableExpectedTypeNode::Array {
            item: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    item,
                ),
            ),
        },
        RuntimeTypeNode::Map { key, value } => RuntimeRecoverableExpectedTypeNode::Map {
            key: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    key,
                ),
            ),
            value: Box::new(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    value,
                ),
            ),
        },
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind,
        } => RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(recoverable_expected_record_field_from_runtime_plan)
                .collect(),
            boundary_record_kind: boundary_record_kind.clone(),
        },
        RuntimeTypeNode::Unknown => RuntimeRecoverableExpectedTypeNode::Unresolved {
            diagnostic_label: plan.label().to_string(),
        },
    }
}

fn recoverable_expected_record_field_from_runtime_plan(
    field: &RuntimeRecordFieldPlan,
) -> RuntimeRecoverableExpectedRecordFieldPlan {
    RuntimeRecoverableExpectedRecordFieldPlan {
        name: field.name.clone(),
        ty: RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
            &field.ty,
        ),
        required: field.required,
    }
}

fn runtime_named_type_identity_ref(name: &str) -> RuntimeRecoverableTypeIdentityRef {
    RuntimeRecoverableTypeIdentityRef::RuntimeNamedType(RuntimeRecoverableNamedTypeRef::new(name))
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn string_node(value: &str) -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::String,
            RecoverableState::String(value.to_string()),
        )
    }

    fn local_concrete_node(concrete_type_identity: &str) -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::NominalObject,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::LocalConcrete {
                owner: LocalConcreteOwner::Service,
                concrete_type_identity: concrete_type_identity.to_string(),
            },
            state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
                fields: vec![RecoverableField {
                    field_identity: "value".to_string(),
                    value: string_node("state"),
                }],
            }),
        }
    }

    fn nested_nominal_map_key(representation_count: usize) -> RecoverableMapKey {
        let mut key = RecoverableMapKey::String("leaf".to_string());
        for index in (0..representation_count).rev() {
            key = RecoverableMapKey::NominalRepresentation {
                representation_identity: format!("repr-{index}"),
                value: Box::new(key),
            };
        }
        key
    }

    #[test]
    fn boundary_context_skips_empty_optional_fields() {
        let context = RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        );

        let json = serde_json::to_value(context).expect("context should serialize");

        assert_eq!(
            json,
            json!({
                "kind": "runtimeBinaryPayload",
                "trustBoundary": "ownerInternal",
                "storageLane": "recoverableEnvelope"
            })
        );
    }

    #[test]
    fn expected_plan_preserves_unresolved_runtime_plan_explicitly() {
        let plan = RuntimeTypePlan {
            label: "anyInterface".to_string(),
            named_type_name: Some("pkg.Reader".to_string()),
            identity: Default::default(),
            node: RuntimeTypeNode::Unknown,
        };

        let recoverable =
            RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                &plan,
            );

        assert_eq!(
            recoverable.identity,
            Some(RuntimeRecoverableTypeIdentityRef::RuntimeNamedType(
                RuntimeRecoverableNamedTypeRef::new("pkg.Reader")
            ))
        );
        assert_eq!(
            recoverable.node,
            RuntimeRecoverableExpectedTypeNode::Unresolved {
                diagnostic_label: "anyInterface".to_string()
            }
        );
    }

    #[test]
    fn envelope_json_debug_roundtrip_preserves_nodes() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![
                string_node("Ada"),
                RecoverableNode::plain(
                    RecoverableValueKind::Number,
                    RecoverableState::Number(
                        RecoverableNumber::try_from_f64(42.5).expect("finite number"),
                    ),
                ),
            ]),
        ));

        let json = serde_json::to_string_pretty(&envelope).expect("json encode");
        let decoded: RecoverableEnvelope = serde_json::from_str(&json).expect("json decode");

        assert_eq!(decoded, envelope);
        decoded
            .validate(&RecoverableValidationLimits::default())
            .expect("debug JSON roundtrip should validate");
    }

    #[test]
    fn canonical_binary_roundtrip_sorts_record_fields_and_map_entries() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Record,
            RecoverableState::Record(vec![
                RecoverableField {
                    field_identity: "z".to_string(),
                    value: string_node("last"),
                },
                RecoverableField {
                    field_identity: "a".to_string(),
                    value: RecoverableNode::plain(
                        RecoverableValueKind::Map,
                        RecoverableState::Map(vec![
                            (RecoverableMapKey::String("b".to_string()), string_node("2")),
                            (RecoverableMapKey::String("a".to_string()), string_node("1")),
                        ]),
                    ),
                },
            ]),
        ));
        let limits = RecoverableValidationLimits::default();

        let first = envelope
            .to_canonical_bytes(&limits)
            .expect("canonical encode should succeed");
        let decoded =
            RecoverableEnvelope::from_canonical_bytes(&first, &limits).expect("canonical decode");
        let second = decoded
            .to_canonical_bytes(&limits)
            .expect("canonical re-encode should succeed");

        assert_eq!(first, second);
        let RecoverableState::Record(fields) = decoded.root.state else {
            panic!("expected record");
        };
        assert_eq!(fields[0].field_identity, "a");
        assert_eq!(fields[1].field_identity, "z");
    }

    #[test]
    fn canonical_binary_rejects_non_canonical_bool_payload() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Bool,
            RecoverableState::Bool(true),
        ));
        let limits = RecoverableValidationLimits::default();
        let mut bytes = envelope
            .to_canonical_bytes(&limits)
            .expect("canonical bool encode should succeed");
        let bool_payload = bytes
            .last_mut()
            .expect("encoded bool should have payload byte");
        assert_eq!(*bool_payload, 1);
        *bool_payload = 2;

        let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
            .expect_err("non-canonical bool payload must fail closed");

        assert_eq!(error.path(), "$.root");
        assert!(error
            .message()
            .contains("recoverable bool payload must be 0 or 1"));
    }

    #[test]
    fn map_key_depth_limit_is_enforced_before_canonical_encode() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Map,
            RecoverableState::Map(vec![(nested_nominal_map_key(2), string_node("value"))]),
        ));
        let limits = RecoverableValidationLimits {
            max_nodes: 16,
            max_depth: 1,
            max_encoded_bytes: 4096,
        };

        let error = envelope
            .to_canonical_bytes(&limits)
            .expect_err("deep nominal map key must fail encode validation");

        assert_eq!(error.path(), "$.mapKey[0]");
        assert!(error.message().contains("recoverable depth exceeds 1"));
    }

    #[test]
    fn nested_nominal_map_key_roundtrips_when_depth_allows_it() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Map,
            RecoverableState::Map(vec![(nested_nominal_map_key(2), string_node("value"))]),
        ));
        let limits = RecoverableValidationLimits {
            max_nodes: 16,
            max_depth: 3,
            max_encoded_bytes: 4096,
        };

        let bytes = envelope
            .to_canonical_bytes(&limits)
            .expect("allowed nominal map key depth should encode");
        let decoded = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
            .expect("allowed nominal map key depth should decode");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn canonical_decode_enforces_depth_before_constructing_tree() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![RecoverableNode::plain(
                RecoverableValueKind::Array,
                RecoverableState::Array(vec![string_node("too-deep")]),
            )]),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("canonical nested array encode should succeed");
        let limits = RecoverableValidationLimits {
            max_nodes: 16,
            max_depth: 1,
            max_encoded_bytes: bytes.len(),
        };

        let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
            .expect_err("decode must enforce max depth while reading");

        assert_eq!(error.path(), "$.root[0][0]");
        assert!(error.message().contains("recoverable depth exceeds 1"));
    }

    #[test]
    fn canonical_decode_enforces_node_budget_before_container_allocation() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![string_node("one"), string_node("two")]),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("canonical array encode should succeed");
        let limits = RecoverableValidationLimits {
            max_nodes: 2,
            max_depth: 512,
            max_encoded_bytes: bytes.len(),
        };

        let error = RecoverableEnvelope::from_canonical_bytes(&bytes, &limits)
            .expect_err("decode must enforce max node budget before Vec allocation");

        assert_eq!(error.path(), "$.root");
        assert!(error.message().contains("exceeding remaining node budget"));
    }

    #[test]
    fn duplicate_field_identity_is_invalid() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Record,
            RecoverableState::Record(vec![
                RecoverableField {
                    field_identity: "same".to_string(),
                    value: string_node("1"),
                },
                RecoverableField {
                    field_identity: "same".to_string(),
                    value: string_node("2"),
                },
            ]),
        ));

        let error = envelope
            .validate(&RecoverableValidationLimits::default())
            .expect_err("duplicate fields must fail");

        assert!(error
            .message()
            .contains("duplicate recoverable field identity"));
    }

    #[test]
    fn duplicate_canonical_map_key_is_invalid() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Map,
            RecoverableState::Map(vec![
                (
                    RecoverableMapKey::String("same".to_string()),
                    string_node("1"),
                ),
                (
                    RecoverableMapKey::String("same".to_string()),
                    string_node("2"),
                ),
            ]),
        ));

        let error = envelope
            .validate(&RecoverableValidationLimits::default())
            .expect_err("duplicate map keys must fail");

        assert!(error.message().contains("duplicate recoverable map key"));
    }

    #[test]
    fn nested_local_concrete_refs_are_not_collected_as_artifact_refs() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![
                local_concrete_node("pkg.User"),
                local_concrete_node("pkg.Org"),
            ]),
        ));

        let refs = envelope.collect_artifact_refs();

        assert!(refs.is_empty());
    }

    #[test]
    fn interface_wrapper_has_no_code_identity_and_self_node_carries_local_concrete() {
        let envelope = RecoverableEnvelope::new(RecoverableNode {
            value_kind: RecoverableValueKind::InterfaceValue,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::None,
            state: RecoverableState::InterfaceValue(InterfaceValueState {
                self_node: Box::new(local_concrete_node("pkg.FileReader")),
            }),
        });

        envelope
            .validate(&RecoverableValidationLimits::default())
            .expect("interface wrapper with self_node code should validate");
        let refs = envelope.collect_artifact_refs();
        assert!(refs.is_empty());
    }

    #[test]
    fn interface_wrapper_rejects_own_code_identity() {
        let envelope = RecoverableEnvelope::new(RecoverableNode {
            value_kind: RecoverableValueKind::InterfaceValue,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::LocalConcrete {
                owner: LocalConcreteOwner::Service,
                concrete_type_identity: "pkg.FileReader".to_string(),
            },
            state: RecoverableState::InterfaceValue(InterfaceValueState {
                self_node: Box::new(local_concrete_node("pkg.FileReader")),
            }),
        });

        let error = envelope
            .validate(&RecoverableValidationLimits::default())
            .expect_err("wrapper code identity must fail");

        assert!(error
            .message()
            .contains("InterfaceValue wrapper must not carry code identity"));
    }

    #[test]
    fn native_handle_adapter_identity_only_lives_in_code_identity() {
        let envelope = RecoverableEnvelope::new(RecoverableNode {
            value_kind: RecoverableValueKind::NativeHandle,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: "std.FileHandleAdapter".to_string(),
                adapter_schema_version: "1".to_string(),
                owner: NativeAdapterOwner::Artifact {
                    artifact_identity: "svc/files".to_string(),
                    build_id: "build-native".to_string(),
                    package: None,
                },
                native_type_identity: "std.FileHandle".to_string(),
            },
            state: RecoverableState::NativeHandle(NativeHandleState {
                durable_state: Box::new(string_node("handle-state")),
            }),
        });

        envelope
            .validate(&RecoverableValidationLimits::default())
            .expect("native handle should validate");
        let refs = envelope.collect_artifact_refs();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].artifact_identity, "svc/files");
        assert_eq!(refs[0].node_path, "$.root");
    }

    #[test]
    fn invalid_schema_and_limits_fail_closed() {
        let mut envelope = RecoverableEnvelope::new(string_node("Ada"));
        envelope.schema_version = "old".to_string();
        assert!(envelope
            .validate(&RecoverableValidationLimits::default())
            .expect_err("unknown schema should fail")
            .message()
            .contains("unsupported recoverable schema version"));

        let mut v1_bytes = RecoverableEnvelope::new(string_node("Ada"))
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("v2 envelope should encode");
        let schema_offset = v1_bytes
            .windows(RECOVERABLE_ENVELOPE_SCHEMA_VERSION.len())
            .position(|window| window == RECOVERABLE_ENVELOPE_SCHEMA_VERSION.as_bytes())
            .expect("schema string should be encoded");
        *v1_bytes
            .get_mut(schema_offset + RECOVERABLE_ENVELOPE_SCHEMA_VERSION.len() - 1)
            .expect("schema version suffix should exist") = b'1';
        let error = RecoverableEnvelope::from_canonical_bytes(
            &v1_bytes,
            &RecoverableValidationLimits::default(),
        )
        .expect_err("v1 canonical bytes must fail closed");
        assert_eq!(error.path(), "$");
        assert!(error
            .message()
            .contains("unsupported recoverable schema version"));

        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![string_node("nested")]),
        ));
        let limits = RecoverableValidationLimits {
            max_nodes: 1,
            max_depth: 512,
            max_encoded_bytes: 1024,
        };
        assert!(envelope
            .validate(&limits)
            .expect_err("node count limit should fail")
            .message()
            .contains("node count exceeds"));
    }
}
