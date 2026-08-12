//! Bytecode canonical identity (C9) and the validated artifact admission
//! token (§6.1, §5.3 of the Phase 1 artifact schema design).
//!
//! Mirrors the `FileIrIdentityPayload` pattern (`file_ir.rs`): the preimage is
//! a projection of the artifact that **excludes the `bytecode_identity` field
//! itself** but covers everything else (schema marker, schema/ISA versions,
//! opcode table fingerprint, all semantic authority identities and the full
//! image including source-event rows and the debug table, D10/D14).
//! `BTreeMap`-backed maps keep the canonical JSON order stable.
//!
//! `structurally_validate` (C1–C8) runs before any identity computation:
//! identity is only ever derived from structurally valid content, and
//! `validate_bytecode_identity` = C1–C8 + C9 (declared identity == recomputed).

use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    BytecodeArtifact, BytecodeArtifactRef, BytecodeImage, HostEffectRegistryIdentity,
    IntrinsicRegistryIdentity, NativeValueLifecycleRegistryIdentity,
    PlatformErrorProjectionRegistryRef, StructurallyValidatedView, ValidatedFunctionStreamItem,
    ValidatedIntrinsicContract, ValueLifecyclePolicyIdentity,
};

use crate::framing::{canonical_ir_bytes, framed_identity, sha256_hex};
use crate::{
    ArtifactIdentityError, Result, BYTECODE_IDENTITY_PREFIX, BYTECODE_IDENTITY_SCHEMA_MARKER,
};

/// Complete canonical preimage of a bytecode image identity (§6.1).
///
/// Field order is fixed by this struct definition. The `bytecode_identity`
/// field of the artifact never enters the preimage (self-reference).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BytecodeIdentityPayload<'a> {
    /// Schema marker for this identity generation.
    schema: &'static str,
    schema_version: &'a str,
    isa_version: &'a str,
    opcode_table_fingerprint: &'a str,
    native_value_lifecycle_registry: &'a NativeValueLifecycleRegistryIdentity,
    value_lifecycle_policy: &'a ValueLifecyclePolicyIdentity,
    host_effect_registry: &'a HostEffectRegistryIdentity,
    intrinsic_registry: &'a IntrinsicRegistryIdentity,
    platform_error_projection_registry: &'a PlatformErrorProjectionRegistryRef,
    image: &'a BytecodeImage,
    intrinsic_contracts: Option<&'a [ValidatedIntrinsicContract]>,
    function_stream_items: Option<&'a [ValidatedFunctionStreamItem]>,
}

impl<'a> BytecodeIdentityPayload<'a> {
    fn from_view(
        artifact: &'a BytecodeArtifact,
        view: Option<&'a StructurallyValidatedView>,
    ) -> Self {
        Self {
            schema: BYTECODE_IDENTITY_SCHEMA_MARKER,
            schema_version: &artifact.schema_version,
            isa_version: &artifact.isa_version,
            opcode_table_fingerprint: &artifact.opcode_table_fingerprint,
            native_value_lifecycle_registry: &artifact.native_value_lifecycle_registry,
            value_lifecycle_policy: &artifact.value_lifecycle_policy,
            host_effect_registry: &artifact.host_effect_registry,
            intrinsic_registry: &artifact.intrinsic_registry,
            platform_error_projection_registry: &artifact.platform_error_projection_registry,
            image: &artifact.image,
            intrinsic_contracts: view.map(StructurallyValidatedView::intrinsic_contracts),
            function_stream_items: view.map(StructurallyValidatedView::function_stream_items),
        }
    }
}

/// Recomputes the identity of a bytecode artifact after C1–C8 structural
/// validation. The declared `bytecode_identity` field is not part of the
/// preimage; callers use `validate_bytecode_identity` to compare it.
pub fn bytecode_identity(artifact: &BytecodeArtifact) -> Result<String> {
    let view = skiff_artifact_model::structurally_validate(artifact)?;
    bytecode_identity_after_structural(artifact, Some(&view))
}

/// Identity computation without the C1–C8 gate. Only reachable after
/// `structurally_validate` succeeded (or from tests that pin the raw preimage
/// participation of schema/ISA/fingerprint fields).
fn bytecode_identity_after_structural(
    artifact: &BytecodeArtifact,
    view: Option<&StructurallyValidatedView>,
) -> Result<String> {
    let payload = BytecodeIdentityPayload::from_view(artifact, view);
    let bytes = canonical_ir_bytes(&payload, ArtifactIdentityError::SerializeBytecodeIdentity)?;
    Ok(framed_identity(
        BYTECODE_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    ))
}

/// Assigns the recomputed identity into `artifact.bytecode_identity` and
/// returns it. Runs C1–C8 first.
pub fn assign_bytecode_identity(artifact: &mut BytecodeArtifact) -> Result<String> {
    let computed = bytecode_identity(artifact)?;
    artifact.bytecode_identity = computed.clone();
    Ok(computed)
}

/// C9: validates the declared `bytecode_identity` against the identity
/// recomputed from the canonical preimage, after C1–C8 structural validation.
pub fn validate_bytecode_identity(artifact: &BytecodeArtifact) -> Result<()> {
    validated_bytecode_view(artifact).map(|_| ())
}

/// C1–C8 + C9 returning the opaque structural view (single validation pass for
/// `ValidatedBytecodeArtifact::admit`).
fn validated_bytecode_view(artifact: &BytecodeArtifact) -> Result<StructurallyValidatedView> {
    let view = skiff_artifact_model::structurally_validate(artifact)?;
    let computed = bytecode_identity_after_structural(artifact, Some(&view))?;
    if artifact.bytecode_identity != computed {
        return Err(ArtifactIdentityError::BytecodeIdentityMismatch {
            declared: artifact.bytecode_identity.clone(),
            computed,
        });
    }
    Ok(view)
}

/// Validates that `identity` is a well-formed framed bytecode identity
/// (`skiff-bytecode-image-v5:sha256:<64 lowercase hex>`). Used when a
/// `PackageArtifact` carries a `BytecodeArtifactRef` (C9 linkage check at the
/// package surface level, before the build projection is computed).
pub fn validate_bytecode_identity_format(identity: &str) -> Result<()> {
    let expected_prefix = format!("{BYTECODE_IDENTITY_PREFIX}:");
    let Some(hash) = identity.strip_prefix(&expected_prefix) else {
        return Err(ArtifactIdentityError::InvalidBytecodeIdentity {
            identity: identity.to_string(),
        });
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ArtifactIdentityError::InvalidBytecodeIdentity {
            identity: identity.to_string(),
        });
    }
    Ok(())
}

/// Opaque, process-local admission for one exact immutable bytecode artifact.
///
/// Mirrors `ValidatedPackageArtifact`: the token owns the typed artifact, its
/// canonical record bytes, the opaque C1–C8 view and the derived reference.
/// Fields are private and the only construction path is `admit` (which runs
/// C1–C9), so callers can reuse a successful validation without manufacturing
/// an "already validated" flag. `reference().artifact_path` is `None` here:
/// the store path is derived later from the package coordinate
/// (`PackageBytecodeRecordPath`) and validated there.
#[derive(Debug, Clone)]
pub struct ValidatedBytecodeArtifact {
    artifact: Arc<BytecodeArtifact>,
    reference: BytecodeArtifactRef,
    canonical_bytes: Arc<[u8]>,
    canonical_sha256: [u8; 32],
    canonical_byte_len: u64,
    view: StructurallyValidatedView,
}

impl ValidatedBytecodeArtifact {
    /// C1–C9 admission. Returns an error on any structural failure or
    /// identity mismatch; never a partial token.
    pub fn admit(artifact: BytecodeArtifact) -> Result<Self> {
        let view = validated_bytecode_view(&artifact)?;
        let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
        let canonical_bytes = skiff_canonical_json::canonical_json_bytes(&artifact)
            .map_err(ArtifactIdentityError::SerializeValidatedBytecodeArtifact)?;
        let canonical_byte_len = u64::try_from(canonical_bytes.len()).map_err(|_| {
            ArtifactIdentityError::InvalidPackageArtifact {
                message: "canonical BytecodeArtifact byte length does not fit u64".to_string(),
            }
        })?;
        let canonical_sha256 = Sha256::digest(&canonical_bytes).into();
        Ok(Self {
            artifact: Arc::new(artifact),
            reference,
            canonical_bytes: Arc::from(canonical_bytes),
            canonical_sha256,
            canonical_byte_len,
            view,
        })
    }

    pub fn artifact(&self) -> &BytecodeArtifact {
        &self.artifact
    }

    pub fn view(&self) -> &StructurallyValidatedView {
        &self.view
    }

    pub fn reference(&self) -> &BytecodeArtifactRef {
        &self.reference
    }

    pub fn exactly_matches(&self, artifact: &BytecodeArtifact) -> bool {
        self.artifact.as_ref() == artifact
    }

    pub fn has_same_exact_content(&self, other: &Self) -> bool {
        self.canonical_byte_len == other.canonical_byte_len
            && self.canonical_sha256 == other.canonical_sha256
            && self.canonical_bytes == other.canonical_bytes
            && self.artifact == other.artifact
    }
}

#[cfg(test)]
mod tests;
