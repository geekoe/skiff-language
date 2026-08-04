use std::{fs, path::PathBuf, sync::Arc};

use mongodb::bson::{spec::BinarySubtype, Binary, Bson, Document};
use serde::Deserialize;

use super::*;

const ROOT_KEY_BASE64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const OTHER_ROOT_KEY_BASE64: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
const SENTINEL_PLAINTEXT: &str = "unique-plaintext-sentinel-4c91";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenVector {
    root_key_hex: String,
    key_id: String,
    storage_profile: String,
    storage_service_id: String,
    collection_name: String,
    field_name: String,
    record_id: String,
    nonce_hex: String,
    plaintext: String,
    hkdf_info_hex: String,
    derived_key_hex: String,
    aad_hex: String,
    ciphertext_and_tag_hex: String,
    fingerprint_input_hex: String,
    fingerprint_hex: String,
}

fn fixture() -> GoldenVector {
    serde_json::from_str(include_str!("../../testdata/encryption_golden_vector.json"))
        .expect("golden vector fixture should parse")
}

fn context<'a>(fixture: &'a GoldenVector) -> DbEncryptedFieldContext<'a> {
    DbEncryptedFieldContext {
        storage_profile: &fixture.storage_profile,
        storage_service_id: &fixture.storage_service_id,
        collection_name: &fixture.collection_name,
        field_name: &fixture.field_name,
        record_id: &fixture.record_id,
    }
}

fn keyring_json(active: &str, entries: &[(&str, &str)]) -> String {
    let keys = entries
        .iter()
        .map(|(id, key)| format!("{id:?}:{key:?}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"format":"{SERVICE_DB_ENCRYPTION_KEYRING_FORMAT}","activeKeyId":{active:?},"keys":{{{keys}}}}}"#
    )
}

fn keyring(active: &str, entries: &[(&str, &str)]) -> Arc<DbEncryptionKeyring> {
    Arc::new(
        DbEncryptionKeyring::parse_json(keyring_json(active, entries).as_bytes())
            .expect("test keyring should parse"),
    )
}

fn envelope_inner_mut(value: &mut Bson) -> &mut Document {
    value
        .as_document_mut()
        .expect("outer envelope")
        .get_document_mut(ENVELOPE_FIELD)
        .expect("inner envelope")
}

#[test]
fn normative_encryption_and_fingerprint_vectors_match_fixed_fixture() {
    let fixture = fixture();
    let root_key: [u8; ROOT_KEY_BYTES] = hex::decode(&fixture.root_key_hex)
        .expect("root key hex")
        .try_into()
        .expect("32-byte root key");
    let nonce: [u8; NONCE_BYTES] = hex::decode(&fixture.nonce_hex)
        .expect("nonce hex")
        .try_into()
        .expect("12-byte nonce");
    let ctx = context(&fixture);

    let info = binary_tuple(&[
        FIELD_KDF_MARKER,
        fixture.key_id.as_bytes(),
        fixture.storage_profile.as_bytes(),
        fixture.storage_service_id.as_bytes(),
        fixture.collection_name.as_bytes(),
        fixture.field_name.as_bytes(),
    ])
    .expect("HKDF info tuple");
    assert_eq!(hex::encode(&info), fixture.hkdf_info_hex);
    let derived = derive_field_key(&root_key, &fixture.key_id, ctx).expect("derived key");
    assert_eq!(hex::encode(derived.as_ref()), fixture.derived_key_hex);
    let aad = field_aad(&fixture.key_id, ctx).expect("AAD tuple");
    assert_eq!(hex::encode(&aad), fixture.aad_hex);

    let ring = keyring(&fixture.key_id, &[(&fixture.key_id, ROOT_KEY_BASE64)]);
    let cipher = ring.cipher();
    let envelope = cipher
        .encrypt_string_with_nonce(ctx, &fixture.plaintext, nonce)
        .expect("golden encryption");
    let parsed = parse_envelope(&envelope).expect("golden envelope");
    assert_eq!(
        hex::encode(parsed.ciphertext),
        fixture.ciphertext_and_tag_hex
    );
    assert_eq!(
        cipher
            .decrypt_string(ctx, &envelope)
            .expect("golden decrypt"),
        fixture.plaintext
    );

    let keys = &ring.keys;
    let fingerprint_parts = [
        KEYRING_FINGERPRINT_MARKER,
        SERVICE_DB_ENCRYPTION_KEYRING_FORMAT.as_bytes(),
        fixture.key_id.as_bytes(),
        fixture.key_id.as_bytes(),
        keys.get(&fixture.key_id).expect("golden root key").as_ref(),
    ];
    let fingerprint_input = binary_tuple(&fingerprint_parts).expect("fingerprint tuple");
    assert_eq!(
        hex::encode(fingerprint_input),
        fixture.fingerprint_input_hex
    );
    assert_eq!(ring.fingerprint(), fixture.fingerprint_hex);
}

#[test]
fn random_nonce_encryptions_are_distinct_and_round_trip() {
    let fixture = fixture();
    let ring = keyring("test-key", &[("test-key", ROOT_KEY_BASE64)]);
    let cipher = ring.cipher();
    let first = cipher
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("first encryption");
    let second = cipher
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("second encryption");

    assert_ne!(first, second);
    assert_eq!(
        cipher
            .decrypt_string(context(&fixture), &first)
            .expect("first decrypt"),
        SENTINEL_PLAINTEXT
    );
    assert_eq!(
        cipher
            .decrypt_string(context(&fixture), &second)
            .expect("second decrypt"),
        SENTINEL_PLAINTEXT
    );
    let envelope_debug = format!("{first:?}");
    assert!(!envelope_debug.contains(SENTINEL_PLAINTEXT));
    assert!(!envelope_debug.contains(ROOT_KEY_BASE64));
}

#[test]
fn old_key_envelopes_decrypt_and_new_writes_use_active_key() {
    let fixture = fixture();
    let old_ring = keyring("old-key", &[("old-key", ROOT_KEY_BASE64)]);
    let old_envelope = old_ring
        .cipher()
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("old encryption");
    let rotating_ring = keyring(
        "new-key",
        &[
            ("old-key", ROOT_KEY_BASE64),
            ("new-key", OTHER_ROOT_KEY_BASE64),
        ],
    );
    let cipher = rotating_ring.cipher();

    assert_eq!(
        cipher
            .decrypt_string(context(&fixture), &old_envelope)
            .expect("old key decrypt"),
        SENTINEL_PLAINTEXT
    );
    let new_envelope = cipher
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("new encryption");
    assert_eq!(
        parse_envelope(&new_envelope).expect("new envelope").key_id,
        "new-key"
    );
}

#[test]
fn ciphertext_nonce_tag_key_id_and_all_context_changes_fail_closed() {
    let fixture = fixture();
    let ring = keyring(
        "test-key",
        &[
            ("test-key", ROOT_KEY_BASE64),
            ("other-key", ROOT_KEY_BASE64),
        ],
    );
    let cipher = ring.cipher();
    let original = cipher
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("encryption");
    let assert_decode_fails = |stored: &Bson, ctx| {
        assert_eq!(
            cipher.decrypt_string(ctx, stored),
            Err(DbEncryptionError::Decode)
        );
    };

    let mut changed_ciphertext = original.clone();
    if let Some(Bson::Binary(value)) =
        envelope_inner_mut(&mut changed_ciphertext).get_mut("ciphertext")
    {
        value.bytes[0] ^= 1;
    }
    assert_decode_fails(&changed_ciphertext, context(&fixture));

    let mut changed_tag = original.clone();
    if let Some(Bson::Binary(value)) = envelope_inner_mut(&mut changed_tag).get_mut("ciphertext") {
        let last = value.bytes.len() - 1;
        value.bytes[last] ^= 1;
    }
    assert_decode_fails(&changed_tag, context(&fixture));

    let mut changed_nonce = original.clone();
    if let Some(Bson::Binary(value)) = envelope_inner_mut(&mut changed_nonce).get_mut("nonce") {
        value.bytes[0] ^= 1;
    }
    assert_decode_fails(&changed_nonce, context(&fixture));

    let mut changed_key_id = original.clone();
    envelope_inner_mut(&mut changed_key_id).insert("keyId", "other-key");
    assert_decode_fails(&changed_key_id, context(&fixture));

    for changed_context in [
        DbEncryptedFieldContext {
            record_id: "other-record",
            ..context(&fixture)
        },
        DbEncryptedFieldContext {
            storage_profile: "other",
            ..context(&fixture)
        },
        DbEncryptedFieldContext {
            storage_service_id: "other.example/service",
            ..context(&fixture)
        },
        DbEncryptedFieldContext {
            collection_name: "other_collection",
            ..context(&fixture)
        },
        DbEncryptedFieldContext {
            field_name: "otherField",
            ..context(&fixture)
        },
    ] {
        assert_decode_fails(&original, changed_context);
    }

    let wrong_root = keyring("test-key", &[("test-key", OTHER_ROOT_KEY_BASE64)]);
    assert_eq!(
        wrong_root
            .cipher()
            .decrypt_string(context(&fixture), &original),
        Err(DbEncryptionError::Decode)
    );
}

#[test]
fn malformed_envelopes_and_invalid_utf8_have_one_sanitized_decode_error() {
    let fixture = fixture();
    let ring = keyring("test-key", &[("test-key", ROOT_KEY_BASE64)]);
    let cipher = ring.cipher();
    let valid = cipher
        .encrypt_string(context(&fixture), SENTINEL_PLAINTEXT)
        .expect("encryption");
    let mut cases = vec![
        Bson::String(SENTINEL_PLAINTEXT.to_string()),
        Bson::Document(Document::new()),
    ];
    for (field, value) in [
        ("version", Bson::Int32(1)),
        ("version", Bson::String("1".to_string())),
        ("keyId", Bson::String("unknown-key".to_string())),
        ("keyId", Bson::String("invalid/key".to_string())),
        ("keyId", Bson::Int32(1)),
        (
            "nonce",
            Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![0; NONCE_BYTES - 1],
            }),
        ),
        ("nonce", Bson::String("not-binary".to_string())),
        (
            "nonce",
            Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes: vec![0; NONCE_BYTES],
            }),
        ),
        (
            "ciphertext",
            Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![0; AUTH_TAG_BYTES - 1],
            }),
        ),
        ("ciphertext", Bson::String("not-binary".to_string())),
        (
            "ciphertext",
            Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes: vec![0; AUTH_TAG_BYTES],
            }),
        ),
    ] {
        let mut changed = valid.clone();
        envelope_inner_mut(&mut changed).insert(field, value);
        cases.push(changed);
    }
    let mut missing = valid.clone();
    envelope_inner_mut(&mut missing).remove("nonce");
    cases.push(missing);
    let mut extra = valid.clone();
    envelope_inner_mut(&mut extra).insert("extra", true);
    cases.push(extra);
    let mut outer_extra = valid.clone();
    outer_extra
        .as_document_mut()
        .expect("outer envelope")
        .insert("extra", true);
    cases.push(outer_extra);

    for case in cases {
        let error = cipher
            .decrypt_string(context(&fixture), &case)
            .expect_err("malformed envelope must fail");
        assert_eq!(error, DbEncryptionError::Decode);
        assert_eq!(
            error.to_string(),
            "service DB encrypted field decode failed"
        );
        assert!(!format!("{error:?}").contains(SENTINEL_PLAINTEXT));
    }

    let nonce = [7_u8; NONCE_BYTES];
    let key_id = ring.active_key_id();
    let ctx = context(&fixture);
    let key = derive_field_key(ring.root_key(key_id).expect("root key"), key_id, ctx)
        .expect("derived key");
    let aad = field_aad(key_id, ctx).expect("AAD");
    let raw_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let ciphertext = raw_cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &[0xff],
                aad: &aad,
            },
        )
        .expect("raw invalid UTF-8 encryption");
    let stored = envelope_bson(key_id, nonce, ciphertext);
    assert_eq!(
        cipher.decrypt_string(ctx, &stored),
        Err(DbEncryptionError::Decode)
    );
}

#[test]
fn keyring_parser_is_strict_and_errors_do_not_echo_input() {
    let valid = keyring_json("test-key", &[("test-key", ROOT_KEY_BASE64)]);
    let parsed = DbEncryptionKeyring::parse_json(valid.as_bytes()).expect("valid keyring");
    assert_eq!(parsed.format(), SERVICE_DB_ENCRYPTION_KEYRING_FORMAT);
    assert_eq!(parsed.active_key_id(), "test-key");

    let invalid_inputs = [
        "",
        "[]",
        r#"{"format":"skiff-service-db-keyring-v1","format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","activeKeyId":"other","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=","test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"wrong","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"missing","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"","keys":{"":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","keys":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"bad/id","keys":{"bad/id":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8_"}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"c2hvcnQ="}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":42}}"#,
        r#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="},"extra":"unique-key-material-sentinel"}"#,
    ];
    for input in invalid_inputs {
        let error = match DbEncryptionKeyring::parse_json(input.as_bytes()) {
            Ok(_) => panic!("invalid keyring must fail"),
            Err(error) => error,
        };
        if !input.is_empty() {
            assert!(!error.to_string().contains(input));
        }
        assert!(!format!("{error:?}").contains("unique-key-material-sentinel"));
    }
}

#[test]
fn fingerprint_is_order_independent_and_covers_active_set_and_material() {
    let first = keyring("a", &[("b", OTHER_ROOT_KEY_BASE64), ("a", ROOT_KEY_BASE64)]);
    let reordered = keyring("a", &[("a", ROOT_KEY_BASE64), ("b", OTHER_ROOT_KEY_BASE64)]);
    let changed_material = keyring(
        "a",
        &[("a", OTHER_ROOT_KEY_BASE64), ("b", OTHER_ROOT_KEY_BASE64)],
    );
    let missing_old = keyring("a", &[("a", ROOT_KEY_BASE64)]);
    let changed_active = keyring("b", &[("a", ROOT_KEY_BASE64), ("b", OTHER_ROOT_KEY_BASE64)]);

    assert_eq!(first.fingerprint(), reordered.fingerprint());
    assert_ne!(first.fingerprint(), changed_material.fingerprint());
    assert_ne!(first.fingerprint(), missing_old.fingerprint());
    assert_ne!(first.fingerprint(), changed_active.fingerprint());
    assert_eq!(first.fingerprint().len(), 64);
    assert!(first
        .fingerprint()
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-service-db-keyring-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn keyring_loader_requires_a_secure_regular_file() {
    let temp = TestDir::new();
    let path = temp.0.join("keyring.json");
    fs::write(
        &path,
        keyring_json("test-key", &[("test-key", ROOT_KEY_BASE64)]),
    )
    .expect("write keyring");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("secure mode");
        let loaded = DbEncryptionKeyring::load(&path).expect("secure keyring loads");
        assert_eq!(loaded.active_key_id(), "test-key");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("insecure mode");
        let error = match DbEncryptionKeyring::load(&path) {
            Ok(_) => panic!("group-readable must fail"),
            Err(error) => error,
        };
        assert_eq!(error, DbEncryptionKeyringError::InsecurePermissions);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore mode");
        let link = temp.0.join("keyring-link.json");
        symlink(&path, &link).expect("symlink");
        let error = match DbEncryptionKeyring::load(&link) {
            Ok(_) => panic!("symlink must fail"),
            Err(error) => error,
        };
        assert_eq!(error, DbEncryptionKeyringError::NotRegularFile);
    }

    let directory_error = match DbEncryptionKeyring::load(&temp.0) {
        Ok(_) => panic!("directory must fail"),
        Err(error) => error,
    };
    assert_eq!(directory_error, DbEncryptionKeyringError::NotRegularFile);
    let missing_error = match DbEncryptionKeyring::load(&temp.0.join("missing.json")) {
        Ok(_) => panic!("missing file must fail"),
        Err(error) => error,
    };
    assert_eq!(missing_error, DbEncryptionKeyringError::Unreadable);
}

#[cfg(unix)]
#[test]
fn keyring_loader_rejects_fifo_without_blocking() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, sync::mpsc, time::Duration};

    let temp = TestDir::new();
    let path = temp.0.join("keyring.fifo");
    let c_path = CString::new(path.as_os_str().as_bytes()).expect("FIFO path has no NUL byte");
    let status = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    assert_eq!(
        status,
        0,
        "mkfifo failed: {}",
        std::io::Error::last_os_error()
    );

    let (sender, receiver) = mpsc::channel();
    let loader = std::thread::spawn(move || {
        let error = match DbEncryptionKeyring::load(&path) {
            Ok(_) => None,
            Err(error) => Some(error),
        };
        sender.send(error).expect("FIFO test receiver remains live");
    });

    let error = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("FIFO keyring load must not block waiting for a writer");
    loader.join().expect("FIFO loader thread should finish");
    assert_eq!(error, Some(DbEncryptionKeyringError::NotRegularFile));
}
