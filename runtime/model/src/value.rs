use std::{cmp::Ordering, collections::BTreeMap, fmt};

use crate::addr::ExecutableAddr;

pub type RuntimeString = String;
pub type RuntimeObjectFields = BTreeMap<RuntimeString, RuntimeValue>;
pub type RuntimeMap = BTreeMap<RuntimeValueKey, RuntimeValue>;
pub type InterfaceMethodTableId = RuntimeString;
pub type RemoteOperationTableId = RuntimeString;

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
    Remote {
        dependency_ref: RuntimeString,
        public_instance_key: RuntimeString,
        operations: RemoteOperationTable,
    },
}

impl InterfaceCarrier {
    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Remote { .. } => "remote",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOperationTable {
    id: RemoteOperationTableId,
    interface_abi_id: RuntimeString,
    slots: Vec<RemoteOperationSlot>,
}

impl RemoteOperationTable {
    pub fn new(
        id: RemoteOperationTableId,
        interface_abi_id: RuntimeString,
        slots: Vec<RemoteOperationSlot>,
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

    pub fn slots(&self) -> &[RemoteOperationSlot] {
        &self.slots
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOperationSlot {
    slot: u32,
    method_abi_id: RuntimeString,
    operation_abi_id: RuntimeString,
}

impl RemoteOperationSlot {
    pub fn new(slot: u32, method_abi_id: RuntimeString, operation_abi_id: RuntimeString) -> Self {
        Self {
            slot,
            method_abi_id,
            operation_abi_id,
        }
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn operation_abi_id(&self) -> &str {
        &self.operation_abi_id
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
    method_abi_id: RuntimeString,
    target: InterfaceMethodTarget,
}

impl InterfaceMethodSlot {
    pub fn new(slot: u32, method_abi_id: RuntimeString, target: InterfaceMethodTarget) -> Self {
        Self {
            slot,
            method_abi_id,
            target,
        }
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
mod tests {
    use super::*;

    #[test]
    fn interface_value_local_carrier_keeps_method_table_and_payload() {
        let interface = "reader-interface<string>".to_string();
        let table = InterfaceMethodTable::new(
            "table:reader:string".to_string(),
            "reader-interface".to_string(),
            vec![InterfaceMethodSlot::new(
                0,
                "method:reader:read".to_string(),
                InterfaceMethodTarget::LocalExecutable {
                    executable: ExecutableAddr::service(0, 7),
                    receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                },
            )],
        );
        let value = InterfaceValue::new(
            interface.clone(),
            InterfaceCarrier::Local {
                concrete_type: "root.ReaderImpl".to_string(),
                method_table: table,
                payload: RuntimeValue::Null,
            },
        );

        let InterfaceCarrier::Local {
            method_table,
            payload,
            ..
        } = value.carrier()
        else {
            panic!("expected local interface carrier");
        };
        assert_eq!(value.interface(), interface);
        assert_eq!(
            value.diagnostic_label(),
            "any interface reader-interface<string> (local)"
        );
        assert_eq!(method_table.slots()[0].slot(), 0);
        assert_eq!(payload, &RuntimeValue::Null);

        let remote_value = InterfaceValue::new(
            interface.clone(),
            InterfaceCarrier::Remote {
                dependency_ref: "svc.reader".to_string(),
                public_instance_key: "reader#42".to_string(),
                operations: RemoteOperationTable::new(
                    "remote:reader".to_string(),
                    "reader-interface".to_string(),
                    vec![RemoteOperationSlot::new(
                        0,
                        "method:reader:read".to_string(),
                        "operation:reader:read".to_string(),
                    )],
                ),
            },
        );

        let InterfaceCarrier::Remote {
            dependency_ref,
            public_instance_key,
            operations,
        } = remote_value.carrier()
        else {
            panic!("expected remote interface carrier");
        };
        assert_eq!(remote_value.interface(), interface);
        assert_eq!(
            remote_value.diagnostic_label(),
            "any interface reader-interface<string> (remote)"
        );
        assert_eq!(dependency_ref, "svc.reader");
        assert_eq!(public_instance_key, "reader#42");
        assert_eq!(operations.slots()[0].slot(), 0);
        assert_eq!(operations.slots()[0].method_abi_id(), "method:reader:read");
        assert_eq!(
            operations.slots()[0].operation_abi_id(),
            "operation:reader:read"
        );
    }
}
