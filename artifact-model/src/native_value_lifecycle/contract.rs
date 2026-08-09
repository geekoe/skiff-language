use std::fmt;

use serde::{Deserialize, Serialize};

/// Owned identity pinned into every bytecode image. The fingerprint covers
/// the registry id, version, and sorted canonical entry projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueLifecycleRegistryIdentity {
    pub registry_id: String,
    pub version: String,
    pub fingerprint: String,
}

/// Exact constructor identity admitted by the native lifecycle registry.
/// Package entries use resolved package ids, never caller-local dependency
/// aliases.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NativeValueTypeConstructor {
    Builtin {
        name: String,
    },
    PackageSymbol {
        package_id: String,
        symbol_path: String,
        abi_identity: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeValueArgumentPolicy {
    RequireSnapshotShare,
    Phantom,
}

/// Constructor plus exact generic arity/policies. Vector length is arity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueTypePattern {
    pub constructor: NativeValueTypeConstructor,
    pub argument_policies: Vec<NativeValueArgumentPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeValueAdapterRole {
    CloneLease,
    ValueDrop,
    ResourceDrop,
}

/// Exact native lifecycle adapter identity. `binding_key` is globally unique
/// within one registry identity, so the compact bytecode key is lossless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueLifecycleAdapter {
    pub binding_key: String,
    pub role: NativeValueAdapterRole,
    pub abi_version: u32,
}

/// Drop actions valid for snapshot-share and move-only values. Deliberately
/// no pool-local `shape_ref` can enter this registry-owned type surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NativeValueDropPlan {
    Trivial,
    SnapshotRelease,
    NativeAdapter {
        adapter: NativeValueLifecycleAdapter,
    },
}

/// Drop actions valid for affine resources and explicit clone leases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NativeResourceDropPlan {
    ResourceTableRelease,
    NativeAdapter {
        adapter: NativeValueLifecycleAdapter,
    },
}

/// Registry template. The four concrete ownership states are explicit;
/// `FromType` is the only generic plan expression and is eliminated by
/// lookup before a plan can enter linked code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NativeValueLifecycleTemplate {
    SnapshotShare {
        drop: NativeValueDropPlan,
    },
    MoveOnly {
        drop: NativeValueDropPlan,
    },
    AffineResource {
        drop: NativeResourceDropPlan,
    },
    ExplicitCloneLease {
        clone_adapter: NativeValueLifecycleAdapter,
        drop: NativeResourceDropPlan,
    },
    FromType {
        argument_index: u32,
    },
}

/// Fully instantiated ownership state returned by registry lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NativeValueLifecycleConcrete {
    SnapshotShare {
        drop: NativeValueDropPlan,
    },
    MoveOnly {
        drop: NativeValueDropPlan,
    },
    AffineResource {
        drop: NativeResourceDropPlan,
    },
    ExplicitCloneLease {
        clone_adapter: NativeValueLifecycleAdapter,
        drop: NativeResourceDropPlan,
    },
}

impl NativeValueLifecycleConcrete {
    pub const fn kind(&self) -> NativeValueLifecycleKind {
        match self {
            Self::SnapshotShare { .. } => NativeValueLifecycleKind::SnapshotShare,
            Self::MoveOnly { .. } => NativeValueLifecycleKind::MoveOnly,
            Self::AffineResource { .. } => NativeValueLifecycleKind::AffineResource,
            Self::ExplicitCloneLease { .. } => NativeValueLifecycleKind::ExplicitCloneLease,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeValueLifecycleKind {
    SnapshotShare,
    MoveOnly,
    AffineResource,
    ExplicitCloneLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeValueEmbedding {
    Ordinary,
    Forbidden,
    Privileged,
}

/// Exact ownership and aggregate-embedding semantics for one resolved type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueLifecycleResolution {
    pub lifecycle: NativeValueLifecycleConcrete,
    pub embedding: NativeValueEmbedding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueLifecycleEntry {
    pub pattern: NativeValueTypePattern,
    pub lifecycle: NativeValueLifecycleTemplate,
    pub embedding: NativeValueEmbedding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeValueLifecycleRegistryError {
    EmptyRegistryId,
    EmptyVersion,
    EmptyConstructorField {
        entry: usize,
        field: &'static str,
    },
    TooManyArguments {
        entry: usize,
        actual: usize,
    },
    DuplicateConstructorArity {
        constructor: NativeValueTypeConstructor,
        arity: usize,
    },
    InvalidFromType {
        entry: usize,
        argument_index: u32,
        message: &'static str,
    },
    InvalidAdapter {
        entry: usize,
        binding_key: String,
        message: &'static str,
    },
    ConflictingAdapter {
        binding_key: String,
        first_role: NativeValueAdapterRole,
        first_abi_version: u32,
        next_role: NativeValueAdapterRole,
        next_abi_version: u32,
    },
    FingerprintProjection {
        message: String,
    },
}

impl fmt::Display for NativeValueLifecycleRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "native value lifecycle registry is invalid: {self:?}"
        )
    }
}

impl std::error::Error for NativeValueLifecycleRegistryError {}

#[derive(Debug, Clone, PartialEq)]
pub enum NativeValueLifecycleLookupError {
    UnsupportedType {
        message: String,
    },
    Missing {
        constructor: NativeValueTypeConstructor,
    },
    ArityMismatch {
        constructor: NativeValueTypeConstructor,
        expected: Vec<usize>,
        actual: usize,
    },
    Argument {
        index: usize,
        source: Box<NativeValueLifecycleLookupError>,
    },
    ArgumentPolicyMismatch {
        index: usize,
        policy: NativeValueArgumentPolicy,
        actual: NativeValueLifecycleKind,
    },
    NestingLimit,
}

impl fmt::Display for NativeValueLifecycleLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "native value lifecycle lookup failed: {self:?}")
    }
}

impl std::error::Error for NativeValueLifecycleLookupError {}
