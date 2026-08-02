//! Shared fixtures/helpers for the W-actor `actor_*` test targets.
//!
//! Test-only; no production code is imported from here.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_router::actor::{
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, CommitFenceFacts,
};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader, ActorOwnerFileFrameHeader,
    ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::protocol::{ActivationIdentityFrameMetadata, ActorKeyFrameMetadata};

pub const ROUTE_ASSEMBLY_IDENTITY: &str =
    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const ROUTE_ASSEMBLY_GENERATION: u64 = 42;

pub fn abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)))
}

pub fn actor_implementation_identity() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        "b".repeat(64)
    ))
}

pub fn method_identity() -> ActorMethodIdentity {
    ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:{}", "c".repeat(64)))
}

pub fn declaration_owner() -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: ActorOwnerUnitFrameHeader::Service,
        file: ActorOwnerFileFrameHeader::FileIrIdentity("file:1".to_string()),
        actor_symbol: "Counter".to_string(),
    }
}

pub fn actor_key() -> ActorLogicalKey {
    ActorLogicalKey {
        service_id: "example.com/docs".to_string(),
        actor_type_identity: "CounterActor".to_string(),
        actor_id_type_identity: "CounterId".to_string(),
        actor_id_encoding_version: "skiff-actor-id-encoding-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "AQID".to_string(),
        actor_id_hash: format!("sha256:{}", "1".repeat(64)),
    }
}

pub fn key_wire() -> ActorKeyFrameMetadata {
    let key = actor_key();
    ActorKeyFrameMetadata {
        service_id: key.service_id,
        actor_type_identity: key.actor_type_identity,
        actor_id_type_identity: key.actor_id_type_identity,
        actor_id_encoding_version: key.actor_id_encoding_version,
        canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64,
        actor_id_hash: Some(key.actor_id_hash),
    }
}

pub fn actor_ref_wire(epoch: u64) -> ActorLogicalRefFrameHeader {
    let key = actor_key();
    ActorLogicalRefFrameHeader {
        service_id: key.service_id,
        actor_type_identity: key.actor_type_identity,
        actor_id_type_identity: key.actor_id_type_identity,
        actor_id_encoding_version: key.actor_id_encoding_version,
        canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64,
        actor_id_hash: key.actor_id_hash,
        epoch,
    }
}

pub fn route_authority() -> ActorOwnerRouteAuthority {
    ActorOwnerRouteAuthority {
        assembly_identity: ROUTE_ASSEMBLY_IDENTITY.to_string(),
        assembly_generation: ROUTE_ASSEMBLY_GENERATION,
    }
}

pub fn route_authority_wire(
) -> skiff_runtime_transport::actor_owner::ActorOwnerRouteAuthorityFrameHeader {
    route_authority().to_wire()
}

pub fn activation_identity_wire(runtime_id: &str) -> ActivationIdentityFrameMetadata {
    ActivationIdentityFrameMetadata {
        assembly_identity: ROUTE_ASSEMBLY_IDENTITY.to_string(),
        generation: ROUTE_ASSEMBLY_GENERATION,
        runtime_replica_id: runtime_id.to_string(),
        deployment_revision: "rev-1".to_string(),
    }
}

pub fn fence_facts() -> CommitFenceFacts {
    CommitFenceFacts {
        actor_abi_identity: abi(),
        actor_implementation_identity: actor_implementation_identity(),
        declaration_owner: declaration_owner(),
    }
}

pub fn fence(runtime_id: &str, epoch: u64, lease_expires_at: u64) -> ActorOwnerFence {
    ActorOwnerFence {
        epoch,
        owner_runtime_id: runtime_id.to_string(),
        owner_lease_id: "owner-lease-test".to_string(),
        lease_expires_at,
        actor_abi_identity: abi(),
        actor_implementation_identity: actor_implementation_identity(),
        declaration_owner: declaration_owner(),
    }
}

pub fn actor_wire_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/actor-wire")
}

pub fn spawn_wire_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/spawn-wire")
}

pub fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

pub fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn read_scenario(dir: &Path, name: &str) -> serde_json::Value {
    let path = dir.join("scenarios").join(name);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    serde_json::from_str(&raw).expect("scenario must decode")
}
