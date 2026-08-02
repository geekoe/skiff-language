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
