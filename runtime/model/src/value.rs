use std::{cmp::Ordering, collections::BTreeMap, fmt};

use crate::{
    addr::{ExecutableAddr, TypeAddr},
    service_error::{CatchIdentity, RequestException},
};

pub type RuntimeString = String;
pub type RuntimeObjectFields = BTreeMap<RuntimeString, RuntimeValue>;
pub type RuntimeMap = BTreeMap<RuntimeValueKey, RuntimeValue>;
pub type InterfaceMethodTableId = RuntimeString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeBytes {
    bytes: bytes::Bytes,
}

impl RuntimeBytes {
    pub fn new(bytes: impl Into<bytes::Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl From<Vec<u8>> for RuntimeBytes {
    fn from(value: Vec<u8>) -> Self {
        Self::new(bytes::Bytes::from(value))
    }
}

impl From<bytes::Bytes> for RuntimeBytes {
    fn from(value: bytes::Bytes) -> Self {
        Self::new(value)
    }
}

impl From<&[u8]> for RuntimeBytes {
    fn from(value: &[u8]) -> Self {
        Self::new(bytes::Bytes::copy_from_slice(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeapHandle {
    index: u32,
    generation: u32,
}

impl HeapHandle {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Display for HeapHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.index, self.generation)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeValue {
    Null,
    Bool(bool),
    Number(f64),
    Date(i64),
    String(RuntimeString),
    ActorRef(ActorRef),
    Heap(HeapHandle),
}

/// Canonical runtime value carrier for nominal catch identity.
///
/// Slots, container elements and call arguments can carry this value by move
/// or clone without reconstructing identity from a static type plan or runtime
/// shape. Structural values deliberately use `unidentified`; nominal records,
/// representations and selected union branches use `identified`.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeValueCarrier {
    value: RuntimeValue,
    catch_identity: Option<CatchIdentity>,
}

impl RuntimeValueCarrier {
    pub fn unidentified(value: RuntimeValue) -> Self {
        Self {
            value,
            catch_identity: None,
        }
    }

    pub fn identified(value: RuntimeValue, catch_identity: CatchIdentity) -> Self {
        Self {
            value,
            catch_identity: Some(catch_identity),
        }
    }

    pub fn from_parts(value: RuntimeValue, catch_identity: Option<CatchIdentity>) -> Self {
        Self {
            value,
            catch_identity,
        }
    }

    pub fn value(&self) -> &RuntimeValue {
        &self.value
    }

    pub fn into_value(self) -> RuntimeValue {
        self.value
    }

    pub fn into_parts(self) -> (RuntimeValue, Option<CatchIdentity>) {
        (self.value, self.catch_identity)
    }

    pub fn map_value(self, map: impl FnOnce(RuntimeValue) -> RuntimeValue) -> Self {
        let (value, catch_identity) = self.into_parts();
        Self {
            value: map(value),
            catch_identity,
        }
    }

    pub fn catch_identity(&self) -> Option<&CatchIdentity> {
        self.catch_identity.as_ref()
    }
}

impl From<RuntimeValue> for RuntimeValueCarrier {
    fn from(value: RuntimeValue) -> Self {
        Self::unidentified(value)
    }
}

impl std::ops::Deref for RuntimeValueCarrier {
    type Target = RuntimeValue;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

impl AsRef<RuntimeValue> for RuntimeValueCarrier {
    fn as_ref(&self) -> &RuntimeValue {
        self.value()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceValue {
    interface: RuntimeString,
    carrier: InterfaceCarrier,
}

impl InterfaceValue {
    pub fn new(interface: RuntimeString, carrier: InterfaceCarrier) -> Self {
        Self { interface, carrier }
    }

    pub fn interface(&self) -> &str {
        &self.interface
    }

    pub fn carrier(&self) -> &InterfaceCarrier {
        &self.carrier
    }

    pub fn diagnostic_label(&self) -> String {
        format!(
            "any interface {} ({})",
            self.interface,
            self.carrier.kind_label()
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterfaceCarrier {
    Local {
        concrete_type: RuntimeString,
        method_table: InterfaceMethodTable,
        payload: RuntimeValue,
    },
    /// Opaque request-scoped route back to the activation that owns a
    /// boundary-capable interface or native adapter.
    ///
    /// This carrier deliberately contains no method table, native object, or
    /// process address.  Dispatch has to resolve it through the owning
    /// runtime's activation capability table.
    CallbackCapability(CallbackCapabilityCarrier),
}

impl InterfaceCarrier {
    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::CallbackCapability(_) => "callback capability",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackCapabilityCarrier {
    owner_runtime_replica_id: RuntimeString,
    owner_activation_id: RuntimeString,
    request_generation: u64,
    interface_or_adapter_contract: RuntimeString,
    opaque_capability_id: RuntimeString,
}

impl CallbackCapabilityCarrier {
    pub fn new(
        owner_runtime_replica_id: impl Into<RuntimeString>,
        owner_activation_id: impl Into<RuntimeString>,
        request_generation: u64,
        interface_or_adapter_contract: impl Into<RuntimeString>,
        opaque_capability_id: impl Into<RuntimeString>,
    ) -> Self {
        Self {
            owner_runtime_replica_id: owner_runtime_replica_id.into(),
            owner_activation_id: owner_activation_id.into(),
            request_generation,
            interface_or_adapter_contract: interface_or_adapter_contract.into(),
            opaque_capability_id: opaque_capability_id.into(),
        }
    }

    pub fn owner_runtime_replica_id(&self) -> &str {
        &self.owner_runtime_replica_id
    }

    pub fn owner_activation_id(&self) -> &str {
        &self.owner_activation_id
    }

    pub const fn request_generation(&self) -> u64 {
        self.request_generation
    }

    pub fn interface_or_adapter_contract(&self) -> &str {
        &self.interface_or_adapter_contract
    }

    pub fn opaque_capability_id(&self) -> &str {
        &self.opaque_capability_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceMethodTable {
    id: InterfaceMethodTableId,
    interface_abi_id: RuntimeString,
    slots: Vec<InterfaceMethodSlot>,
}

impl InterfaceMethodTable {
    pub fn new(
        id: InterfaceMethodTableId,
        interface_abi_id: RuntimeString,
        slots: Vec<InterfaceMethodSlot>,
    ) -> Self {
        Self {
            id,
            interface_abi_id,
            slots,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn interface_abi_id(&self) -> &str {
        &self.interface_abi_id
    }

    pub fn slots(&self) -> &[InterfaceMethodSlot] {
        &self.slots
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceMethodSlot {
    slot: u32,
    /// Exact source/interface operation name retained from admitted linked metadata.
    /// Callback contract projection must match this name explicitly; slot order is not a mapping.
    method_name: Option<RuntimeString>,
    /// Exact linked execution signature. `None` is reserved for legacy/runtime
    /// fixtures that did not originate from an admitted method table; canonical
    /// callback projection rejects such slots.
    signature: Option<InterfaceMethodSignature>,
    method_abi_id: RuntimeString,
    target: InterfaceMethodTarget,
}

impl InterfaceMethodSlot {
    pub fn new(slot: u32, method_abi_id: RuntimeString, target: InterfaceMethodTarget) -> Self {
        Self {
            slot,
            method_name: None,
            signature: None,
            method_abi_id,
            target,
        }
    }

    pub fn from_admitted_metadata(
        slot: u32,
        method_name: RuntimeString,
        method_abi_id: RuntimeString,
        signature: InterfaceMethodSignature,
        target: InterfaceMethodTarget,
    ) -> Self {
        Self {
            slot,
            method_name: Some(method_name),
            signature: Some(signature),
            method_abi_id,
            target,
        }
    }

    pub fn method_name(&self) -> Option<&str> {
        self.method_name.as_deref()
    }

    pub fn signature(&self) -> Option<&InterfaceMethodSignature> {
        self.signature.as_ref()
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn target(&self) -> &InterfaceMethodTarget {
        &self.target
    }
}

/// Immutable, typed execution signature retained from a linked interface
/// method-table slot. The first parameter is the receiver for
/// `ExplicitSelfFirst` targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceMethodSignature {
    parameters: Vec<InterfaceMethodType>,
    return_type: InterfaceMethodType,
}

impl InterfaceMethodSignature {
    pub fn new(parameters: Vec<InterfaceMethodType>, return_type: InterfaceMethodType) -> Self {
        Self {
            parameters,
            return_type,
        }
    }

    pub fn parameters(&self) -> &[InterfaceMethodType] {
        &self.parameters
    }

    pub fn return_type(&self) -> &InterfaceMethodType {
        &self.return_type
    }
}

/// Linked execution-type facts used only for typed callback projection. These
/// Variants intentionally carry no service-owned schema identity: Package schema
/// nominal identity is resolved before values enter the runtime model.
/// identity and local execution identity remain separate domains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterfaceMethodType {
    Builtin {
        name: RuntimeString,
        arguments: Vec<InterfaceMethodType>,
    },
    Nominal(TypeAddr),
    AppliedNominal {
        base: TypeAddr,
        arguments: Vec<InterfaceMethodType>,
    },
    Record(BTreeMap<RuntimeString, InterfaceMethodType>),
    Union(Vec<InterfaceMethodType>),
    Nullable(Box<InterfaceMethodType>),
    Literal(InterfaceMethodLiteral),
    AnyInterface {
        interface_abi_id: RuntimeString,
        canonical_type_arguments: Vec<InterfaceMethodType>,
    },
    Function {
        parameters: Vec<InterfaceMethodType>,
        return_type: Box<InterfaceMethodType>,
    },
    TypeParameter(RuntimeString),
    Unresolved(InterfaceMethodUnresolvedType),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceMethodUnresolvedType {
    LocalType,
    PublicationType,
    ServiceSymbol,
    PackageSymbol,
    DbObjectSymbol,
}

impl InterfaceMethodType {
    pub fn builtin(name: impl Into<RuntimeString>) -> Self {
        Self::Builtin {
            name: name.into(),
            arguments: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterfaceMethodLiteral {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(RuntimeString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterfaceMethodTarget {
    LocalExecutable {
        executable: ExecutableAddr,
        receiver_call_abi: InterfaceReceiverCallAbi,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceReceiverCallAbi {
    ExplicitSelfFirst,
}

impl RuntimeValue {
    pub const fn as_heap_handle(&self) -> Option<HeapHandle> {
        match self {
            Self::Heap(handle) => Some(*handle),
            _ => None,
        }
    }
}

impl From<bool> for RuntimeValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for RuntimeValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<String> for RuntimeValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RuntimeValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<HeapHandle> for RuntimeValue {
    fn from(handle: HeapHandle) -> Self {
        Self::Heap(handle)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActorRef {
    service_id: String,
    actor_type_identity: String,
    actor_id_type_identity: String,
    actor_id_encoding_version: String,
    canonical_actor_id_key_bytes: Vec<u8>,
    actor_id_hash: String,
    epoch: Option<u64>,
}

impl ActorRef {
    pub fn new(
        service_id: impl Into<String>,
        actor_type_identity: impl Into<String>,
        actor_id_type_identity: impl Into<String>,
        actor_id_encoding_version: impl Into<String>,
        canonical_actor_id_key_bytes: impl Into<Vec<u8>>,
        actor_id_hash: impl Into<String>,
        epoch: Option<u64>,
    ) -> Self {
        Self {
            service_id: service_id.into(),
            actor_type_identity: actor_type_identity.into(),
            actor_id_type_identity: actor_id_type_identity.into(),
            actor_id_encoding_version: actor_id_encoding_version.into(),
            canonical_actor_id_key_bytes: canonical_actor_id_key_bytes.into(),
            actor_id_hash: actor_id_hash.into(),
            epoch,
        }
    }

    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub fn actor_type_identity(&self) -> &str {
        &self.actor_type_identity
    }

    pub fn actor_id_type_identity(&self) -> &str {
        &self.actor_id_type_identity
    }

    pub fn actor_id_encoding_version(&self) -> &str {
        &self.actor_id_encoding_version
    }

    pub fn canonical_actor_id_key_bytes(&self) -> &[u8] {
        &self.canonical_actor_id_key_bytes
    }

    pub fn actor_id_hash(&self) -> &str {
        &self.actor_id_hash
    }

    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeapNode {
    Bytes(RuntimeBytes),
    Array(Vec<RuntimeValue>),
    Object(RuntimeObject),
    Map(RuntimeMap),
    Interface(InterfaceValue),
    Exception(RequestException),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeObject {
    fields: RuntimeObjectFields,
}

impl RuntimeObject {
    pub fn unshaped(fields: RuntimeObjectFields) -> Self {
        Self { fields }
    }

    pub fn clone_with_fields(&self, fields: RuntimeObjectFields) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &RuntimeObjectFields {
        &self.fields
    }

    pub fn fields_mut(&mut self) -> &mut RuntimeObjectFields {
        &mut self.fields
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeValueKey {
    String(RuntimeString),
}

impl RuntimeValueKey {
    pub fn string(value: impl Into<RuntimeString>) -> Self {
        Self::String(value.into())
    }

    pub fn string_payload(&self) -> &str {
        match self {
            Self::String(value) => value,
        }
    }

    fn ordering_parts(&self) -> (u8, &str) {
        match self {
            Self::String(value) => (0, value),
        }
    }
}

impl Ord for RuntimeValueKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering_parts().cmp(&other.ordering_parts())
    }
}

impl PartialOrd for RuntimeValueKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests;
