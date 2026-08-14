use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use skiff_artifact_model::{HostEffectExecutorIdentity, MetadataValue};

use crate::{
    ArtifactCallbackCaptureIndex, ArtifactFunctionKey, CallbackCaptureLayoutIndex, FrameSlotIndex,
    FunctionIndex, HostEffectAdapterIndex, InterfaceTableIndex, LinkedArtifactPoolOrigin,
    LinkedCallableSignature, LinkedInterfaceMethodAbiId, LinkedNativeCallableSignature,
    LinkedValueTransferPlan, SyntheticCallbackIndex, TypeIndex,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallbackCapture {
    slot: FrameSlotIndex,
    ty: TypeIndex,
    plan: LinkedValueTransferPlan,
}

impl LinkedCallbackCapture {
    pub fn new(slot: FrameSlotIndex, ty: TypeIndex, plan: LinkedValueTransferPlan) -> Self {
        Self { slot, ty, plan }
    }

    pub const fn slot(&self) -> FrameSlotIndex {
        self.slot
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

/// Concrete form of one artifact callback-capture pool row. `function` binds
/// its target slots to the exact synthetic-function specialization, while the
/// origin retains the artifact pool index used to recheck the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallbackCaptureLayout {
    index: CallbackCaptureLayoutIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactCallbackCaptureIndex>,
    artifact_function_key: ArtifactFunctionKey,
    function: FunctionIndex,
    captures: Box<[LinkedCallbackCapture]>,
}

impl LinkedCallbackCaptureLayout {
    pub fn try_new(
        index: CallbackCaptureLayoutIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactCallbackCaptureIndex>,
        artifact_function_key: ArtifactFunctionKey,
        function: FunctionIndex,
        captures: Box<[LinkedCallbackCapture]>,
    ) -> Result<Self, LinkedCallbackCaptureLayoutError> {
        let mut seen = BTreeSet::new();
        for capture in &captures {
            if !seen.insert(capture.slot()) {
                return Err(LinkedCallbackCaptureLayoutError::DuplicateCaptureSlot {
                    slot: capture.slot(),
                });
            }
        }
        Ok(Self {
            index,
            origin,
            artifact_function_key,
            function,
            captures,
        })
    }

    pub const fn index(&self) -> CallbackCaptureLayoutIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactCallbackCaptureIndex> {
        &self.origin
    }

    pub const fn artifact_function_key(&self) -> &ArtifactFunctionKey {
        &self.artifact_function_key
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub fn captures(&self) -> &[LinkedCallbackCapture] {
        &self.captures
    }
}

/// Callable facts for one exact synthetic-function specialization. Capture
/// layout facts remain in their own artifact-origin-bearing image table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSyntheticCallbackTarget {
    index: SyntheticCallbackIndex,
    artifact_function_key: ArtifactFunctionKey,
    function: FunctionIndex,
    interface_method: Option<LinkedCallbackInterfaceMethod>,
    signature: LinkedCallableSignature,
}

/// Exact callback-carrier correlation needed to compare callback, local and
/// remote implementations of one interface method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallbackInterfaceMethod {
    interface_table: InterfaceTableIndex,
    method_slot: u32,
    method_abi_id: LinkedInterfaceMethodAbiId,
}

impl LinkedCallbackInterfaceMethod {
    pub fn new(
        interface_table: InterfaceTableIndex,
        method_slot: u32,
        method_abi_id: LinkedInterfaceMethodAbiId,
    ) -> Self {
        Self {
            interface_table,
            method_slot,
            method_abi_id,
        }
    }

    pub const fn interface_table(&self) -> InterfaceTableIndex {
        self.interface_table
    }

    pub const fn method_slot(&self) -> u32 {
        self.method_slot
    }

    pub const fn method_abi_id(&self) -> &LinkedInterfaceMethodAbiId {
        &self.method_abi_id
    }
}

impl LinkedSyntheticCallbackTarget {
    pub fn new(
        index: SyntheticCallbackIndex,
        artifact_function_key: ArtifactFunctionKey,
        function: FunctionIndex,
        interface_method: Option<LinkedCallbackInterfaceMethod>,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            artifact_function_key,
            function,
            interface_method,
            signature,
        }
    }

    pub const fn index(&self) -> SyntheticCallbackIndex {
        self.index
    }

    pub const fn artifact_function_key(&self) -> &ArtifactFunctionKey {
        &self.artifact_function_key
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn interface_method(&self) -> Option<&LinkedCallbackInterfaceMethod> {
        self.interface_method.as_ref()
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedCallbackCaptureLayoutError {
    DuplicateCaptureSlot { slot: FrameSlotIndex },
}

impl fmt::Display for LinkedCallbackCaptureLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCaptureSlot { slot } => {
                write!(
                    formatter,
                    "callback capture slot {} is repeated",
                    slot.get()
                )
            }
        }
    }
}

impl std::error::Error for LinkedCallbackCaptureLayoutError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedHostBindingKey(Box<str>);

impl LinkedHostBindingKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedHostTargetError> {
        let value = value.into();
        if value.is_empty() {
            return Err(LinkedHostTargetError::EmptyBindingKey);
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(LinkedHostTargetError::InvalidBindingKey {
                value,
                character_index,
            });
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact host target facts. Text leaves are private and lexically checked but
/// remain untrusted registry claims.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedHostEffectAdapterTarget {
    index: HostEffectAdapterIndex,
    executor_identity: HostEffectExecutorIdentity,
    namespace: Box<str>,
    symbol: Box<str>,
    binding_key: LinkedHostBindingKey,
    metadata: BTreeMap<String, MetadataValue>,
    signature: LinkedNativeCallableSignature,
}

impl LinkedHostEffectAdapterTarget {
    pub fn new(
        index: HostEffectAdapterIndex,
        executor_identity: HostEffectExecutorIdentity,
        namespace: impl Into<String>,
        symbol: impl Into<String>,
        binding_key: LinkedHostBindingKey,
        metadata: BTreeMap<String, MetadataValue>,
        signature: LinkedNativeCallableSignature,
    ) -> Result<Self, LinkedHostTargetError> {
        let namespace = namespace.into();
        if namespace.is_empty() {
            return Err(LinkedHostTargetError::EmptyNamespace);
        }
        let symbol = symbol.into();
        if symbol.is_empty() {
            return Err(LinkedHostTargetError::EmptySymbol);
        }
        Ok(Self {
            index,
            executor_identity,
            namespace: namespace.into_boxed_str(),
            symbol: symbol.into_boxed_str(),
            binding_key,
            metadata,
            signature,
        })
    }

    pub const fn index(&self) -> HostEffectAdapterIndex {
        self.index
    }

    /// Closed execution authority minted from the exact pinned registry row.
    pub const fn executor_identity(&self) -> HostEffectExecutorIdentity {
        self.executor_identity
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub const fn binding_key(&self) -> &LinkedHostBindingKey {
        &self.binding_key
    }

    pub const fn metadata(&self) -> &BTreeMap<String, MetadataValue> {
        &self.metadata
    }

    pub const fn signature(&self) -> &LinkedNativeCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedHostTargetError {
    EmptyNamespace,
    EmptySymbol,
    EmptyBindingKey,
    InvalidBindingKey {
        value: String,
        character_index: usize,
    },
}

impl fmt::Display for LinkedHostTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNamespace => formatter.write_str("host target namespace must not be empty"),
            Self::EmptySymbol => formatter.write_str("host target symbol must not be empty"),
            Self::EmptyBindingKey => {
                formatter.write_str("host target binding key must not be empty")
            }
            Self::InvalidBindingKey {
                value,
                character_index,
            } => write!(
                formatter,
                "host target binding key {value:?} contains whitespace or a control character at character index {character_index}"
            ),
        }
    }
}

impl std::error::Error for LinkedHostTargetError {}
