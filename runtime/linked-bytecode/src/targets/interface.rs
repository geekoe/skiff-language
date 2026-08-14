use std::fmt;

use skiff_artifact_model::{
    ContractOperationId, InterfaceInstantiationRef, ReceiverCallAbi, ServiceProtocolIdentity,
    ServiceRequirementKey,
};

use crate::{FunctionIndex, InterfaceTableIndex, LinkedCallableSignature, TypeIndex};

/// Linked form of an artifact interface instantiation. The exact typed DTO is
/// retained verbatim for hydration comparison; its type arguments are also
/// resolved to image-local types for execution-side checking.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedInterfaceInstantiation {
    artifact: InterfaceInstantiationRef,
    concrete_type_arguments: Box<[TypeIndex]>,
}

impl LinkedInterfaceInstantiation {
    pub fn new(
        artifact: InterfaceInstantiationRef,
        concrete_type_arguments: Box<[TypeIndex]>,
    ) -> Result<Self, LinkedInterfaceTableError> {
        if artifact.canonical_type_args.len() != concrete_type_arguments.len() {
            return Err(
                LinkedInterfaceTableError::InstantiationTypeArgumentCountMismatch {
                    artifact_count: artifact.canonical_type_args.len(),
                    linked_count: concrete_type_arguments.len(),
                },
            );
        }
        Ok(Self {
            artifact,
            concrete_type_arguments,
        })
    }

    pub const fn artifact(&self) -> &InterfaceInstantiationRef {
        &self.artifact
    }

    pub fn concrete_type_arguments(&self) -> &[TypeIndex] {
        &self.concrete_type_arguments
    }
}

/// Validated textual method ABI leaf. This wrapper is not ABI authority; link
/// construction retains its exact hydrated declaration owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedInterfaceMethodAbiId(Box<str>);

impl LinkedInterfaceMethodAbiId {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedInterfaceTextError> {
        validate_text(value.into(), LinkedInterfaceTextKind::MethodAbi).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated public-instance lookup leaf. It remains consumer-owned and does
/// not identify a provider deployment or executable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedPublicInstanceKey(Box<str>);

impl LinkedPublicInstanceKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedInterfaceTextError> {
        validate_text(value.into(), LinkedInterfaceTextKind::PublicInstance).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedInterfaceTextKind {
    MethodAbi,
    PublicInstance,
    MethodName,
}

impl LinkedInterfaceTextKind {
    const fn name(self) -> &'static str {
        match self {
            Self::MethodAbi => "interface method ABI id",
            Self::PublicInstance => "public instance key",
            Self::MethodName => "interface method name",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedInterfaceTextError {
    Empty {
        kind: LinkedInterfaceTextKind,
    },
    WhitespaceOrControl {
        kind: LinkedInterfaceTextKind,
        value: String,
        character_index: usize,
    },
}

impl fmt::Display for LinkedInterfaceTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { kind } => write!(formatter, "{} must not be empty", kind.name()),
            Self::WhitespaceOrControl {
                kind,
                value,
                character_index,
            } => write!(
                formatter,
                "{} {value:?} contains whitespace or a control character at character index {character_index}",
                kind.name()
            ),
        }
    }
}

impl std::error::Error for LinkedInterfaceTextError {}

fn validate_text(
    value: String,
    kind: LinkedInterfaceTextKind,
) -> Result<Box<str>, LinkedInterfaceTextError> {
    if value.is_empty() {
        return Err(LinkedInterfaceTextError::Empty { kind });
    }
    if let Some((character_index, _)) = value
        .chars()
        .enumerate()
        .find(|(_, character)| character.is_whitespace() || character.is_control())
    {
        return Err(LinkedInterfaceTextError::WhitespaceOrControl {
            kind,
            value,
            character_index,
        });
    }
    Ok(value.into_boxed_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInterfaceRequirementMethod {
    method_slot: u32,
    method_abi_id: LinkedInterfaceMethodAbiId,
    signature: LinkedCallableSignature,
}

impl LinkedInterfaceRequirementMethod {
    pub fn new(
        method_slot: u32,
        method_abi_id: LinkedInterfaceMethodAbiId,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            method_slot,
            method_abi_id,
            signature,
        }
    }

    pub const fn method_slot(&self) -> u32 {
        self.method_slot
    }

    pub const fn method_abi_id(&self) -> &LinkedInterfaceMethodAbiId {
        &self.method_abi_id
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedLocalInterfaceMethod {
    method_slot: u32,
    method_name: Box<str>,
    method_abi_id: LinkedInterfaceMethodAbiId,
    signature: LinkedCallableSignature,
    function: FunctionIndex,
    receiver_call_abi: ReceiverCallAbi,
}

impl LinkedLocalInterfaceMethod {
    pub fn new(
        method_slot: u32,
        method_name: impl Into<String>,
        method_abi_id: LinkedInterfaceMethodAbiId,
        signature: LinkedCallableSignature,
        function: FunctionIndex,
        receiver_call_abi: ReceiverCallAbi,
    ) -> Result<Self, LinkedInterfaceTextError> {
        let method_name = validate_text(method_name.into(), LinkedInterfaceTextKind::MethodName)?;
        Ok(Self {
            method_slot,
            method_name,
            method_abi_id,
            signature,
            function,
            receiver_call_abi,
        })
    }

    pub const fn method_slot(&self) -> u32 {
        self.method_slot
    }

    pub fn method_name(&self) -> &str {
        &self.method_name
    }

    pub const fn method_abi_id(&self) -> &LinkedInterfaceMethodAbiId {
        &self.method_abi_id
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn receiver_call_abi(&self) -> ReceiverCallAbi {
        self.receiver_call_abi
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedRemoteInterfaceMethod {
    method_slot: u32,
    method_abi_id: LinkedInterfaceMethodAbiId,
    signature: LinkedCallableSignature,
    contract_operation_id: ContractOperationId,
}

impl LinkedRemoteInterfaceMethod {
    pub fn new(
        method_slot: u32,
        method_abi_id: LinkedInterfaceMethodAbiId,
        signature: LinkedCallableSignature,
        contract_operation_id: ContractOperationId,
    ) -> Self {
        Self {
            method_slot,
            method_abi_id,
            signature,
            contract_operation_id,
        }
    }

    pub const fn method_slot(&self) -> u32 {
        self.method_slot
    }

    pub const fn method_abi_id(&self) -> &LinkedInterfaceMethodAbiId {
        &self.method_abi_id
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }

    pub const fn contract_operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInterfaceRequirementTable {
    methods: Box<[LinkedInterfaceRequirementMethod]>,
}

impl LinkedInterfaceRequirementTable {
    pub fn new(
        methods: Box<[LinkedInterfaceRequirementMethod]>,
    ) -> Result<Self, LinkedInterfaceTableError> {
        validate_method_order(&methods, |method| method.method_slot())?;
        Ok(Self { methods })
    }

    pub fn methods(&self) -> &[LinkedInterfaceRequirementMethod] {
        &self.methods
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedLocalInterfaceTable {
    concrete_type: TypeIndex,
    methods: Box<[LinkedLocalInterfaceMethod]>,
}

impl LinkedLocalInterfaceTable {
    pub fn new(
        concrete_type: TypeIndex,
        methods: Box<[LinkedLocalInterfaceMethod]>,
    ) -> Result<Self, LinkedInterfaceTableError> {
        validate_method_order(&methods, |method| method.method_slot())?;
        Ok(Self {
            concrete_type,
            methods,
        })
    }

    pub const fn concrete_type(&self) -> TypeIndex {
        self.concrete_type
    }

    pub fn methods(&self) -> &[LinkedLocalInterfaceMethod] {
        &self.methods
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedRemoteInterfaceTable {
    service_requirement_key: ServiceRequirementKey,
    public_instance_key: LinkedPublicInstanceKey,
    methods: Box<[LinkedRemoteInterfaceMethod]>,
    callee_protocol_identity: ServiceProtocolIdentity,
}

impl LinkedRemoteInterfaceTable {
    pub fn new(
        service_requirement_key: ServiceRequirementKey,
        public_instance_key: LinkedPublicInstanceKey,
        methods: Box<[LinkedRemoteInterfaceMethod]>,
        callee_protocol_identity: ServiceProtocolIdentity,
    ) -> Result<Self, LinkedInterfaceTableError> {
        validate_method_order(&methods, |method| method.method_slot())?;
        Ok(Self {
            service_requirement_key,
            public_instance_key,
            methods,
            callee_protocol_identity,
        })
    }

    pub const fn service_requirement_key(&self) -> &ServiceRequirementKey {
        &self.service_requirement_key
    }

    pub const fn public_instance_key(&self) -> &LinkedPublicInstanceKey {
        &self.public_instance_key
    }

    pub fn methods(&self) -> &[LinkedRemoteInterfaceMethod] {
        &self.methods
    }

    pub const fn callee_protocol_identity(&self) -> &ServiceProtocolIdentity {
        &self.callee_protocol_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedInterfaceTableKind {
    Requirement(LinkedInterfaceRequirementTable),
    Local(LinkedLocalInterfaceTable),
    Remote(LinkedRemoteInterfaceTable),
    /// Canonical method surface for a callback-capability carrier. It remains
    /// distinct from a plain requirement so construction retains carrier
    /// provenance across all three dispatch paths.
    Callback(LinkedInterfaceRequirementTable),
}

/// Interface target table. A requirement variant deliberately has no
/// executable; local, remote and callback carrier rows retain distinct exact
/// facts.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedInterfaceTable {
    index: InterfaceTableIndex,
    interface: LinkedInterfaceInstantiation,
    kind: LinkedInterfaceTableKind,
}

impl LinkedInterfaceTable {
    pub fn new(
        index: InterfaceTableIndex,
        interface: LinkedInterfaceInstantiation,
        kind: LinkedInterfaceTableKind,
    ) -> Self {
        Self {
            index,
            interface,
            kind,
        }
    }

    pub const fn index(&self) -> InterfaceTableIndex {
        self.index
    }

    pub const fn interface(&self) -> &LinkedInterfaceInstantiation {
        &self.interface
    }

    pub const fn kind(&self) -> &LinkedInterfaceTableKind {
        &self.kind
    }
}

fn validate_method_order<T>(
    methods: &[T],
    slot: impl Fn(&T) -> u32,
) -> Result<(), LinkedInterfaceTableError> {
    let mut previous = None;
    for method in methods {
        let current = slot(method);
        if let Some(previous) = previous {
            if previous >= current {
                return Err(LinkedInterfaceTableError::NonCanonicalMethodSlotOrder {
                    previous,
                    current,
                });
            }
        }
        previous = Some(current);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedInterfaceTableError {
    InstantiationTypeArgumentCountMismatch {
        artifact_count: usize,
        linked_count: usize,
    },
    NonCanonicalMethodSlotOrder {
        previous: u32,
        current: u32,
    },
}

impl fmt::Display for LinkedInterfaceTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstantiationTypeArgumentCountMismatch {
                artifact_count,
                linked_count,
            } => write!(
                formatter,
                "interface instantiation has {artifact_count} artifact type arguments but {linked_count} linked arguments"
            ),
            Self::NonCanonicalMethodSlotOrder { previous, current } => write!(
                formatter,
                "interface method slot {current} must sort after {previous}"
            ),
        }
    }
}

impl std::error::Error for LinkedInterfaceTableError {}
