use std::collections::BTreeSet;

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use mongodb::bson::{Bson, Document};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::{
    binary_tuple, generic_binary, validate_key_id, DbEncryptedFieldContext, DbEncryptionCipher,
    DbEncryptionError, DbEncryptionKeyring, AUTH_TAG_BYTES, ENVELOPE_FIELD, NONCE_BYTES,
    ROOT_KEY_BYTES,
};

const V1_FIELD_KDF_SALT: &[u8] = b"skiff-service-db-encrypted-field-v1";
const V1_FIELD_KDF_MARKER: &[u8] = b"skiff-service-db-encrypted-field-hkdf-v1";
const V1_FIELD_AAD_MARKER: &[u8] = b"skiff-service-db-encrypted-field-aad-v1";
const MIGRATION_AUDIT_KDF_SALT: &[u8] = b"skiff-service-db-hardcut-migration-audit-v1";
const MIGRATION_AUDIT_KDF_MARKER: &[u8] = b"skiff-service-db-hardcut-migration-audit-hkdf-v1";
const MIGRATION_DOCUMENT_MARKER: &[u8] =
    b"skiff-service-db-hardcut-migration-document-commitment-v1";
const MIGRATION_PLAN_MARKER: &[u8] = b"skiff-service-db-hardcut-migration-plan-commitment-v1";
const V1_ENVELOPE_VERSION: i64 = 1;

/// Exact v2 storage identity for the migration destination.
///
/// This type deliberately does not implement `Debug`: record data and key
/// material never belong in diagnostics emitted by the offline tool.
#[derive(Clone, Copy)]
pub struct MigrationTargetContext<'a> {
    pub environment: &'a str,
    pub service_id: &'a str,
    pub collection_name: &'a str,
}

/// Opaque, key-bound commitment used to compare source and staged documents
/// without persisting plaintext or a reusable plaintext hash.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MigrationSemanticCommitment([u8; 32]);

impl MigrationSemanticCommitment {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A re-encrypted document plus its opaque semantic commitment.
///
/// No `Debug` implementation is provided because the document can contain
/// business secrets even when all encrypted fields remain enveloped.
pub struct MigrationDocumentResult {
    pub document: Document,
    pub commitment: MigrationSemanticCommitment,
}

/// Offline-only v1-to-v2 crypto seam. It is compiled for the migration binary
/// (and unit tests) but is absent from normal Runtime builds.
///
/// The keyring and cipher intentionally have no `Debug` or `Display`.
pub struct DbMigrationCrypto {
    keyring: std::sync::Arc<DbEncryptionKeyring>,
    v2_cipher: DbEncryptionCipher,
}

impl DbMigrationCrypto {
    pub fn new(keyring: std::sync::Arc<DbEncryptionKeyring>) -> Self {
        let v2_cipher = keyring.cipher();
        Self { keyring, v2_cipher }
    }

    pub fn plan_commitment(
        &self,
        canonical_plan: &[u8],
    ) -> Result<MigrationSemanticCommitment, DbEncryptionError> {
        self.commitment(MIGRATION_PLAN_MARKER, &[], canonical_plan)
    }

    pub fn migrate_v1_document(
        &self,
        source_service_id: &str,
        source_collection_name: &str,
        target: MigrationTargetContext<'_>,
        encrypted_fields: &[String],
        mut document: Document,
    ) -> Result<MigrationDocumentResult, DbEncryptionError> {
        validate_encrypted_fields(encrypted_fields)?;
        let record_id = encrypted_record_id(&document, encrypted_fields)?;
        let mut plaintexts = Vec::with_capacity(encrypted_fields.len());
        for field_name in encrypted_fields {
            let stored = document.get(field_name).ok_or(DbEncryptionError::Decode)?;
            let plaintext = self.decrypt_v1_string(
                source_service_id,
                source_collection_name,
                field_name,
                record_id.as_deref().ok_or(DbEncryptionError::Decode)?,
                stored,
            )?;
            plaintexts.push((field_name.as_str(), plaintext));
        }

        let commitment = self.document_commitment(
            target,
            &document,
            plaintexts
                .iter()
                .map(|(field, plaintext)| (*field, plaintext.as_str())),
        )?;

        for (field_name, plaintext) in &plaintexts {
            let encrypted = self.v2_cipher.encrypt_string(
                DbEncryptedFieldContext {
                    storage_environment: target.environment,
                    storage_service_id: target.service_id,
                    collection_name: target.collection_name,
                    field_name,
                    record_id: record_id.as_deref().ok_or(DbEncryptionError::Decode)?,
                },
                plaintext,
            )?;
            document.insert(*field_name, encrypted);
        }
        Ok(MigrationDocumentResult {
            document,
            commitment,
        })
    }

    pub fn verify_v2_document(
        &self,
        target: MigrationTargetContext<'_>,
        encrypted_fields: &[String],
        document: &Document,
    ) -> Result<MigrationSemanticCommitment, DbEncryptionError> {
        validate_encrypted_fields(encrypted_fields)?;
        let record_id = encrypted_record_id(document, encrypted_fields)?;
        let mut plaintexts = Vec::with_capacity(encrypted_fields.len());
        for field_name in encrypted_fields {
            let stored = document.get(field_name).ok_or(DbEncryptionError::Decode)?;
            let plaintext = Zeroizing::new(self.v2_cipher.decrypt_string(
                DbEncryptedFieldContext {
                    storage_environment: target.environment,
                    storage_service_id: target.service_id,
                    collection_name: target.collection_name,
                    field_name,
                    record_id: record_id.as_deref().ok_or(DbEncryptionError::Decode)?,
                },
                stored,
            )?);
            plaintexts.push((field_name.as_str(), plaintext));
        }
        self.document_commitment(
            target,
            document,
            plaintexts
                .iter()
                .map(|(field, plaintext)| (*field, plaintext.as_str())),
        )
    }

    fn decrypt_v1_string(
        &self,
        service_id: &str,
        collection_name: &str,
        field_name: &str,
        record_id: &str,
        stored: &Bson,
    ) -> Result<Zeroizing<String>, DbEncryptionError> {
        let envelope = parse_v1_envelope(stored)?;
        let root_key = self
            .keyring
            .root_key(envelope.key_id)
            .ok_or(DbEncryptionError::Decode)?;
        let info = binary_tuple(&[
            V1_FIELD_KDF_MARKER,
            envelope.key_id.as_bytes(),
            service_id.as_bytes(),
            collection_name.as_bytes(),
            field_name.as_bytes(),
        ])
        .map_err(|_| DbEncryptionError::Decode)?;
        let hkdf = Hkdf::<Sha256>::new(Some(V1_FIELD_KDF_SALT), root_key);
        let mut field_key = Zeroizing::new([0_u8; ROOT_KEY_BYTES]);
        hkdf.expand(&info, field_key.as_mut())
            .map_err(|_| DbEncryptionError::Decode)?;
        let aad = binary_tuple(&[
            V1_FIELD_AAD_MARKER,
            envelope.key_id.as_bytes(),
            service_id.as_bytes(),
            collection_name.as_bytes(),
            field_name.as_bytes(),
            record_id.as_bytes(),
        ])
        .map_err(|_| DbEncryptionError::Decode)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(field_key.as_ref()));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(envelope.nonce),
                Payload {
                    msg: envelope.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| DbEncryptionError::Decode)?;
        let plaintext = Zeroizing::new(plaintext);
        let plaintext = std::str::from_utf8(&plaintext).map_err(|_| DbEncryptionError::Decode)?;
        Ok(Zeroizing::new(plaintext.to_owned()))
    }

    fn document_commitment<'a>(
        &self,
        target: MigrationTargetContext<'_>,
        document: &Document,
        plaintexts: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<MigrationSemanticCommitment, DbEncryptionError> {
        let plaintexts = plaintexts.collect::<std::collections::BTreeMap<_, _>>();
        let mut encoded = Zeroizing::new(Vec::new());
        for (field_name, value) in document
            .iter()
            .collect::<std::collections::BTreeMap<_, _>>()
        {
            append_framed(&mut encoded, field_name.as_bytes())?;
            if let Some(plaintext) = plaintexts.get(field_name.as_str()) {
                append_framed(&mut encoded, b"encrypted-plaintext")?;
                append_framed(&mut encoded, plaintext.as_bytes())?;
            } else {
                append_framed(&mut encoded, b"bson")?;
                let serialized = Zeroizing::new(
                    serde_json::to_vec(value).map_err(|_| DbEncryptionError::Encode)?,
                );
                append_framed(&mut encoded, &serialized)?;
            }
        }
        let context = binary_tuple(&[
            target.environment.as_bytes(),
            target.service_id.as_bytes(),
            target.collection_name.as_bytes(),
        ])
        .map_err(|_| DbEncryptionError::Encode)?;
        self.commitment(MIGRATION_DOCUMENT_MARKER, &context, &encoded)
    }

    fn commitment(
        &self,
        marker: &[u8],
        context: &[u8],
        payload: &[u8],
    ) -> Result<MigrationSemanticCommitment, DbEncryptionError> {
        let key_id = self.keyring.active_key_id();
        let root_key = self
            .keyring
            .root_key(key_id)
            .ok_or(DbEncryptionError::Encode)?;
        let info = binary_tuple(&[
            MIGRATION_AUDIT_KDF_MARKER,
            key_id.as_bytes(),
            marker,
            context,
        ])
        .map_err(|_| DbEncryptionError::Encode)?;
        let hkdf = Hkdf::<Sha256>::new(Some(MIGRATION_AUDIT_KDF_SALT), root_key);
        let mut audit_key = Zeroizing::new([0_u8; ROOT_KEY_BYTES]);
        hkdf.expand(&info, audit_key.as_mut())
            .map_err(|_| DbEncryptionError::Encode)?;
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(audit_key.as_ref())
            .map_err(|_| DbEncryptionError::Encode)?;
        mac.update(payload);
        let bytes = mac.finalize().into_bytes();
        let mut commitment = [0_u8; 32];
        commitment.copy_from_slice(&bytes);
        Ok(MigrationSemanticCommitment(commitment))
    }
}

fn append_framed(output: &mut Vec<u8>, value: &[u8]) -> Result<(), DbEncryptionError> {
    let len = u32::try_from(value.len()).map_err(|_| DbEncryptionError::Encode)?;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn validate_encrypted_fields(fields: &[String]) -> Result<(), DbEncryptionError> {
    let mut unique = BTreeSet::new();
    if fields.iter().any(|field| {
        field.is_empty()
            || field != field.trim()
            || field == "_id"
            || !unique.insert(field.as_str())
    }) {
        return Err(DbEncryptionError::Decode);
    }
    Ok(())
}

fn encrypted_record_id(
    document: &Document,
    encrypted_fields: &[String],
) -> Result<Option<String>, DbEncryptionError> {
    if encrypted_fields.is_empty() {
        Ok(None)
    } else {
        document
            .get_str("_id")
            .map(ToOwned::to_owned)
            .map(Some)
            .map_err(|_| DbEncryptionError::Decode)
    }
}

struct V1EnvelopeRef<'a> {
    key_id: &'a str,
    nonce: &'a [u8],
    ciphertext: &'a [u8],
}

fn parse_v1_envelope(stored: &Bson) -> Result<V1EnvelopeRef<'_>, DbEncryptionError> {
    let outer = stored.as_document().ok_or(DbEncryptionError::Decode)?;
    if outer.len() != 1 {
        return Err(DbEncryptionError::Decode);
    }
    let inner = outer
        .get_document(ENVELOPE_FIELD)
        .map_err(|_| DbEncryptionError::Decode)?;
    if inner.len() != 4 {
        return Err(DbEncryptionError::Decode);
    }
    let version = match inner.get("version") {
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Int64(value)) => *value,
        _ => return Err(DbEncryptionError::Decode),
    };
    if version != V1_ENVELOPE_VERSION {
        return Err(DbEncryptionError::Decode);
    }
    let key_id = inner
        .get_str("keyId")
        .map_err(|_| DbEncryptionError::Decode)?;
    validate_key_id(key_id).map_err(|_| DbEncryptionError::Decode)?;
    let nonce = generic_binary(inner.get("nonce"))?;
    if nonce.len() != NONCE_BYTES {
        return Err(DbEncryptionError::Decode);
    }
    let ciphertext = generic_binary(inner.get("ciphertext"))?;
    if ciphertext.len() < AUTH_TAG_BYTES {
        return Err(DbEncryptionError::Decode);
    }
    Ok(V1EnvelopeRef {
        key_id,
        nonce,
        ciphertext,
    })
}

#[cfg(test)]
mod tests;
