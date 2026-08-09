use serde::{Deserialize, Serialize};

use crate::{
    BuiltinReceiverThrowSemantics, BytecodeIntrinsicRef, CallableRegistryMatch,
    CallableRegistryMatchError, CallableRegistrySignature, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, ValueProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntrinsicRegistryIdentity {
    pub registry_id: String,
    pub version: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IntrinsicPublicReturnType {
    Fixed { builtin: String },
    Receiver,
    ArrayItem,
    MapValue,
    MapKeyArray,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IntrinsicReceiverSemantics {
    Static,
    Receiver {
        parameter_ordinal: u32,
        mutates_receiver: bool,
        throws: BuiltinReceiverThrowSemantics,
        public_return_type: IntrinsicPublicReturnType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntrinsicRegistryEntry {
    pub target: BytecodeIntrinsicRef,
    pub introduced_capability_version: u32,
    pub receiver: IntrinsicReceiverSemantics,
    pub signature: CallableRegistrySignature,
    pub return_provenance: ValueProvenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntrinsicRegistryMatch {
    pub entry: &'static IntrinsicRegistryEntry,
    pub signature: CallableRegistryMatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IntrinsicRegistryMatchError {
    UnknownTarget,
    Signature { source: CallableRegistryMatchError },
}

pub(super) fn match_entry<R: ValueLifecycleFactResolver>(
    entry: &'static IntrinsicRegistryEntry,
    reference: &crate::bytecode::IntrinsicReference,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<IntrinsicRegistryMatch, IntrinsicRegistryMatchError> {
    let signature = crate::match_callable_registry_signature(
        &entry.signature,
        &reference.signature,
        resolver,
        budget,
    )
    .map_err(|source| IntrinsicRegistryMatchError::Signature { source })?;
    Ok(IntrinsicRegistryMatch { entry, signature })
}
