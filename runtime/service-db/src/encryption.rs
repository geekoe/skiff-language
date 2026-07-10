use std::{
    collections::BTreeMap,
    fmt,
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
    sync::Arc,
};

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use hkdf::Hkdf;
use mongodb::bson::{spec::BinarySubtype, Binary, Bson, Document};
use rand_core::{OsRng, RngCore};
use serde::{
    de::{Error as _, IgnoredAny, MapAccess, Visitor},
    Deserialize, Deserializer,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const SERVICE_DB_ENCRYPTION_KEYRING_FORMAT: &str = "skiff-service-db-keyring-v1";

const FIELD_KDF_SALT: &[u8] = b"skiff-service-db-encrypted-field-v1";
const FIELD_KDF_MARKER: &[u8] = b"skiff-service-db-encrypted-field-hkdf-v1";
const FIELD_AAD_MARKER: &[u8] = b"skiff-service-db-encrypted-field-aad-v1";
const KEYRING_FINGERPRINT_MARKER: &[u8] = b"skiff-service-db-keyring-fingerprint-v1";
const ENVELOPE_FIELD: &str = "_skiff_encrypted";
const ENVELOPE_VERSION: i32 = 1;
const ROOT_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const AUTH_TAG_BYTES: usize = 16;
const CANONICAL_ROOT_KEY_BASE64_BYTES: usize = 44;
const MAX_KEY_ID_BYTES: usize = 64;

/// Non-secret storage identity bound into field-key derivation and AEAD AAD.
///
/// This deliberately has no `Debug` implementation so a record id cannot be
/// included accidentally in diagnostics.
#[derive(Clone, Copy)]
pub struct DbEncryptedFieldContext<'a> {
    pub storage_service_id: &'a str,
    pub collection_name: &'a str,
    pub field_name: &'a str,
    pub record_id: &'a str,
}

/// A runtime-private keyring. Root keys stay in zeroizing containers and this
/// type deliberately has no `Debug` or `Display` implementation.
pub struct DbEncryptionKeyring {
    active_key_id: String,
    keys: BTreeMap<String, Zeroizing<[u8; ROOT_KEY_BYTES]>>,
    fingerprint: String,
}

impl DbEncryptionKeyring {
    fn parse_json(bytes: &[u8]) -> Result<Self, DbEncryptionKeyringError> {
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let raw = RawKeyring::deserialize(&mut deserializer)
            .map_err(|_| DbEncryptionKeyringError::Invalid)?;
        deserializer
            .end()
            .map_err(|_| DbEncryptionKeyringError::Invalid)?;
        Self::from_raw(raw)
    }

    pub fn load(path: &Path) -> Result<Self, DbEncryptionKeyringError> {
        let mut file = open_keyring_file(path)?;
        validate_keyring_file(&file)?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.read_to_end(&mut bytes)
            .map_err(|_| DbEncryptionKeyringError::Unreadable)?;
        Self::parse_json(&bytes)
    }

    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    pub fn format(&self) -> &'static str {
        SERVICE_DB_ENCRYPTION_KEYRING_FORMAT
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn cipher(self: &Arc<Self>) -> DbEncryptionCipher {
        DbEncryptionCipher::new(Arc::clone(self))
    }

    fn from_raw(raw: RawKeyring) -> Result<Self, DbEncryptionKeyringError> {
        if raw.format != SERVICE_DB_ENCRYPTION_KEYRING_FORMAT {
            return Err(DbEncryptionKeyringError::Invalid);
        }
        validate_key_id(&raw.active_key_id)?;

        let mut keys = BTreeMap::new();
        for (key_id, encoded_key) in raw.keys.0 {
            validate_key_id(&key_id)?;
            let key = decode_root_key(&encoded_key)?;
            keys.insert(key_id, key);
        }
        if !keys.contains_key(&raw.active_key_id) {
            return Err(DbEncryptionKeyringError::Invalid);
        }

        let fingerprint = keyring_fingerprint(&raw.active_key_id, &keys)?;
        Ok(Self {
            active_key_id: raw.active_key_id,
            keys,
            fingerprint,
        })
    }

    fn root_key(&self, key_id: &str) -> Option<&[u8; ROOT_KEY_BYTES]> {
        self.keys.get(key_id).map(|key| &**key)
    }
}

/// Shared encrypted-field cipher handle. It holds only an in-memory keyring
/// handle and deliberately has no `Debug` or `Display` implementation.
#[derive(Clone)]
pub struct DbEncryptionCipher {
    keyring: Arc<DbEncryptionKeyring>,
}

impl DbEncryptionCipher {
    pub fn new(keyring: Arc<DbEncryptionKeyring>) -> Self {
        Self { keyring }
    }

    pub fn encrypt_string(
        &self,
        context: DbEncryptedFieldContext<'_>,
        plaintext: &str,
    ) -> Result<Bson, DbEncryptionError> {
        let mut nonce = [0_u8; NONCE_BYTES];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| DbEncryptionError::Encode)?;
        self.encrypt_string_with_nonce(context, plaintext, nonce)
    }

    pub fn decrypt_string(
        &self,
        context: DbEncryptedFieldContext<'_>,
        stored: &Bson,
    ) -> Result<String, DbEncryptionError> {
        let envelope = parse_envelope(stored)?;
        let root_key = self
            .keyring
            .root_key(envelope.key_id)
            .ok_or(DbEncryptionError::Decode)?;
        let field_key = derive_field_key(root_key, envelope.key_id, context)
            .map_err(|_| DbEncryptionError::Decode)?;
        let aad = field_aad(envelope.key_id, context).map_err(|_| DbEncryptionError::Decode)?;
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
        Ok(plaintext.to_owned())
    }

    fn encrypt_string_with_nonce(
        &self,
        context: DbEncryptedFieldContext<'_>,
        plaintext: &str,
        nonce: [u8; NONCE_BYTES],
    ) -> Result<Bson, DbEncryptionError> {
        let key_id = self.keyring.active_key_id();
        let root_key = self
            .keyring
            .root_key(key_id)
            .ok_or(DbEncryptionError::Encode)?;
        let field_key =
            derive_field_key(root_key, key_id, context).map_err(|_| DbEncryptionError::Encode)?;
        let aad = field_aad(key_id, context).map_err(|_| DbEncryptionError::Encode)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(field_key.as_ref()));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| DbEncryptionError::Encode)?;
        Ok(envelope_bson(key_id, nonce, ciphertext))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DbEncryptionError {
    #[error("service DB encrypted field encode failed")]
    Encode,
    #[error("service DB encrypted field decode failed")]
    Decode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DbEncryptionKeyringError {
    #[error("service DB encryption keyring is invalid")]
    Invalid,
    #[error("service DB encryption keyring file is unreadable")]
    Unreadable,
    #[error("service DB encryption keyring file must be a regular file")]
    NotRegularFile,
    #[error("service DB encryption keyring file permissions are insecure")]
    InsecurePermissions,
}

struct RawKeyring {
    format: String,
    active_key_id: String,
    keys: RawKeys,
}

struct RawKeys(Vec<(String, Zeroizing<String>)>);

impl<'de> Deserialize<'de> for RawKeyring {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawKeyringVisitor;

        impl<'de> Visitor<'de> for RawKeyringVisitor {
            type Value = RawKeyring;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a service DB encryption keyring object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut format = None;
                let mut active_key_id = None;
                let mut keys = None;
                while let Some(field) = map.next_key::<String>()? {
                    match field.as_str() {
                        "format" if format.is_none() => format = Some(map.next_value()?),
                        "activeKeyId" if active_key_id.is_none() => {
                            active_key_id = Some(map.next_value()?)
                        }
                        "keys" if keys.is_none() => keys = Some(map.next_value()?),
                        "format" | "activeKeyId" | "keys" => {
                            return Err(A::Error::custom("duplicate keyring field"))
                        }
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                            return Err(A::Error::unknown_field(
                                field.as_str(),
                                &["format", "activeKeyId", "keys"],
                            ));
                        }
                    }
                }
                Ok(RawKeyring {
                    format: format.ok_or_else(|| A::Error::missing_field("format"))?,
                    active_key_id: active_key_id
                        .ok_or_else(|| A::Error::missing_field("activeKeyId"))?,
                    keys: keys.ok_or_else(|| A::Error::missing_field("keys"))?,
                })
            }
        }

        deserializer.deserialize_map(RawKeyringVisitor)
    }
}

impl<'de> Deserialize<'de> for RawKeys {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawKeysVisitor;

        impl<'de> Visitor<'de> for RawKeysVisitor {
            type Value = RawKeys;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of key ids and root keys")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(key_id) = map.next_key::<String>()? {
                    if entries
                        .iter()
                        .any(|(existing, _): &(String, Zeroizing<String>)| existing == &key_id)
                    {
                        return Err(A::Error::custom("duplicate key id"));
                    }
                    entries.push((key_id, Zeroizing::new(map.next_value::<String>()?)));
                }
                Ok(RawKeys(entries))
            }
        }

        deserializer.deserialize_map(RawKeysVisitor)
    }
}

fn decode_root_key(
    encoded: &str,
) -> Result<Zeroizing<[u8; ROOT_KEY_BYTES]>, DbEncryptionKeyringError> {
    let bytes = encoded.as_bytes();
    if bytes.len() != CANONICAL_ROOT_KEY_BASE64_BYTES
        || bytes.last() != Some(&b'=')
        || bytes.get(CANONICAL_ROOT_KEY_BASE64_BYTES - 2) == Some(&b'=')
    {
        return Err(DbEncryptionKeyringError::Invalid);
    }
    let decoded = Zeroizing::new(
        BASE64_STANDARD
            .decode(bytes)
            .map_err(|_| DbEncryptionKeyringError::Invalid)?,
    );
    let canonical = Zeroizing::new(BASE64_STANDARD.encode(&decoded));
    if decoded.len() != ROOT_KEY_BYTES || canonical.as_str() != encoded {
        return Err(DbEncryptionKeyringError::Invalid);
    }
    let mut key = Zeroizing::new([0_u8; ROOT_KEY_BYTES]);
    key.copy_from_slice(&decoded);
    Ok(key)
}

fn validate_key_id(key_id: &str) -> Result<(), DbEncryptionKeyringError> {
    if key_id.is_empty()
        || key_id.len() > MAX_KEY_ID_BYTES
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(DbEncryptionKeyringError::Invalid);
    }
    Ok(())
}

fn open_keyring_file(path: &Path) -> Result<File, DbEncryptionKeyringError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A read-only open of a FIFO blocks before we can reject it via fstat.
        // O_NONBLOCK is inert for regular files and lets the opened-fd type
        // check below fail closed for FIFOs without waiting for a writer.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            #[cfg(unix)]
            Some(libc::ELOOP) => DbEncryptionKeyringError::NotRegularFile,
            _ => DbEncryptionKeyringError::Unreadable,
        })
}

fn validate_keyring_file(file: &File) -> Result<(), DbEncryptionKeyringError> {
    let metadata = file
        .metadata()
        .map_err(|_| DbEncryptionKeyringError::Unreadable)?;
    if !metadata.file_type().is_file() {
        return Err(DbEncryptionKeyringError::NotRegularFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(DbEncryptionKeyringError::InsecurePermissions);
        }
    }
    Ok(())
}

fn derive_field_key(
    root_key: &[u8; ROOT_KEY_BYTES],
    key_id: &str,
    context: DbEncryptedFieldContext<'_>,
) -> Result<Zeroizing<[u8; ROOT_KEY_BYTES]>, DbEncryptionError> {
    let info = binary_tuple(&[
        FIELD_KDF_MARKER,
        key_id.as_bytes(),
        context.storage_service_id.as_bytes(),
        context.collection_name.as_bytes(),
        context.field_name.as_bytes(),
    ])
    .map_err(|_| DbEncryptionError::Encode)?;
    let hkdf = Hkdf::<Sha256>::new(Some(FIELD_KDF_SALT), root_key);
    let mut key = Zeroizing::new([0_u8; ROOT_KEY_BYTES]);
    hkdf.expand(&info, key.as_mut())
        .map_err(|_| DbEncryptionError::Encode)?;
    Ok(key)
}

fn field_aad(
    key_id: &str,
    context: DbEncryptedFieldContext<'_>,
) -> Result<Vec<u8>, DbEncryptionError> {
    binary_tuple(&[
        FIELD_AAD_MARKER,
        key_id.as_bytes(),
        context.storage_service_id.as_bytes(),
        context.collection_name.as_bytes(),
        context.field_name.as_bytes(),
        context.record_id.as_bytes(),
    ])
    .map_err(|_| DbEncryptionError::Encode)
}

fn binary_tuple(parts: &[&[u8]]) -> Result<Vec<u8>, TupleLengthError> {
    let total_len = parts.iter().try_fold(0_usize, |total, part| {
        let _ = u32::try_from(part.len()).map_err(|_| TupleLengthError)?;
        total
            .checked_add(4)
            .and_then(|value| value.checked_add(part.len()))
            .ok_or(TupleLengthError)
    })?;
    let mut encoded = Vec::with_capacity(total_len);
    for part in parts {
        let len = u32::try_from(part.len()).map_err(|_| TupleLengthError)?;
        encoded.extend_from_slice(&len.to_be_bytes());
        encoded.extend_from_slice(part);
    }
    Ok(encoded)
}

#[derive(Clone, Copy, Debug)]
struct TupleLengthError;

fn keyring_fingerprint(
    active_key_id: &str,
    keys: &BTreeMap<String, Zeroizing<[u8; ROOT_KEY_BYTES]>>,
) -> Result<String, DbEncryptionKeyringError> {
    let mut parts = Vec::<&[u8]>::with_capacity(3 + keys.len() * 2);
    parts.push(KEYRING_FINGERPRINT_MARKER);
    parts.push(SERVICE_DB_ENCRYPTION_KEYRING_FORMAT.as_bytes());
    parts.push(active_key_id.as_bytes());
    for (key_id, key) in keys {
        parts.push(key_id.as_bytes());
        parts.push(key.as_ref());
    }
    let input =
        Zeroizing::new(binary_tuple(&parts).map_err(|_| DbEncryptionKeyringError::Invalid)?);
    Ok(hex::encode(Sha256::digest(input.as_slice())))
}

fn envelope_bson(key_id: &str, nonce: [u8; NONCE_BYTES], ciphertext: Vec<u8>) -> Bson {
    let mut inner = Document::new();
    inner.insert("version", ENVELOPE_VERSION);
    inner.insert("keyId", key_id);
    inner.insert(
        "nonce",
        Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: nonce.to_vec(),
        }),
    );
    inner.insert(
        "ciphertext",
        Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: ciphertext,
        }),
    );
    let mut outer = Document::new();
    outer.insert(ENVELOPE_FIELD, inner);
    Bson::Document(outer)
}

struct EnvelopeRef<'a> {
    key_id: &'a str,
    nonce: &'a [u8],
    ciphertext: &'a [u8],
}

fn parse_envelope(stored: &Bson) -> Result<EnvelopeRef<'_>, DbEncryptionError> {
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
    if version != i64::from(ENVELOPE_VERSION) {
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
    Ok(EnvelopeRef {
        key_id,
        nonce,
        ciphertext,
    })
}

fn generic_binary(value: Option<&Bson>) -> Result<&[u8], DbEncryptionError> {
    match value {
        Some(Bson::Binary(binary)) if binary.subtype == BinarySubtype::Generic => Ok(&binary.bytes),
        _ => Err(DbEncryptionError::Decode),
    }
}

#[cfg(test)]
mod tests;
