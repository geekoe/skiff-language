//! Actor-method task target helpers (E2b): snapshot key / create-input
//! decoding and store-model declaration owner ↔ wire frame conversion.

use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::protocol::ActorKeyFrameMetadata;
use skiff_task_control::model::{
    ActorDeclarationOwner, ActorDeclarationOwnerFile, ActorDeclarationOwnerUnit,
    RecoverablePayload,
};

use crate::actor::ActorLogicalKey;

/// Decodes the frozen snapshot `key` payload (canonical JSON projection of the
/// actor logical key, E2a `actor_activation_key_payload`) into the router's
/// logical key.
pub fn snapshot_actor_key(payload: &RecoverablePayload) -> Result<ActorLogicalKey, String> {
    let key: ActorKeyFrameMetadata = serde_json::from_slice(payload.as_bytes()).map_err(|error| {
        format!("actor task snapshot key is not canonical logical-key JSON: {error}")
    })?;
    Ok(ActorLogicalKey::from_wire(&key))
}

/// Converts the task-control store declaration-owner projection back to the
/// wire frame consumed by `actor.owner.control` / `actor.owner.invoke`.
pub fn store_declaration_owner_to_frame(
    owner: &ActorDeclarationOwner,
) -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: match owner.unit {
            ActorDeclarationOwnerUnit::Service => ActorOwnerUnitFrameHeader::Service,
            ActorDeclarationOwnerUnit::Package(slot) => {
                ActorOwnerUnitFrameHeader::Package(slot)
            }
        },
        file: match &owner.file {
            ActorDeclarationOwnerFile::LoadedFileIndex(index) => {
                ActorOwnerFileFrameHeader::LoadedFileIndex(*index)
            }
            ActorDeclarationOwnerFile::FileIrIdentity(identity) => {
                ActorOwnerFileFrameHeader::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: owner.actor_symbol.clone(),
    }
}
