use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use hkdf::Hkdf;
use mongodb::bson::{doc, spec::BinarySubtype, Binary, Bson, Document};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::{
    binary_tuple, DbEncryptionKeyring, DbMigrationCrypto, MigrationTargetContext,
    V1_FIELD_AAD_MARKER, V1_FIELD_KDF_MARKER, V1_FIELD_KDF_SALT,
};
use crate::encryption::ROOT_KEY_BYTES;

const ROOT_KEY_BASE64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const PLAINTEXT_SENTINEL: &str = "migration-plaintext-must-not-escape";

#[test]
fn v1_fixture_rewraps_to_v2_and_wrong_target_context_is_rejected() {
    let keyring = Arc::new(
            DbEncryptionKeyring::parse_json(
                format!(
                    r#"{{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{{"test-key":"{ROOT_KEY_BASE64}"}}}}"#
                )
                .as_bytes(),
            )
            .expect("test keyring"),
        );
    let old = v1_fixture(&keyring, PLAINTEXT_SENTINEL);
    let crypto = DbMigrationCrypto::new(keyring);
    let target = MigrationTargetContext {
        environment: "dev",
        service_id: "skiff.run/agine",
        collection_name: "_skiff_c1_target",
    };
    let migrated = crypto
        .migrate_v1_document(
            "skiff.run/agine",
            "providers",
            target,
            &["token".to_string()],
            doc! { "_id": "provider-1", "token": old, "kind": "openai" },
        )
        .expect("v1 fixture must migrate");
    let verified = crypto
        .verify_v2_document(target, &["token".to_string()], &migrated.document)
        .expect("new context must verify");
    assert_eq!(verified.as_bytes(), migrated.commitment.as_bytes());

    let wrong_environment = MigrationTargetContext {
        environment: "prod",
        ..target
    };
    let error = match crypto.verify_v2_document(
        wrong_environment,
        &["token".to_string()],
        &migrated.document,
    ) {
        Ok(_) => panic!("wrong environment must not authenticate"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains(PLAINTEXT_SENTINEL));

    let wrong_collection = MigrationTargetContext {
        collection_name: "_skiff_c1_other",
        ..target
    };
    assert!(
        crypto
            .verify_v2_document(wrong_collection, &["token".to_string()], &migrated.document,)
            .is_err(),
        "wrong physical target must not authenticate"
    );

    let encoded = serde_json::to_string(&migrated.document).expect("serialize envelope");
    assert!(!encoded.contains(PLAINTEXT_SENTINEL));
    assert!(encoded.contains("\"version\":2"));
}

#[test]
fn ordinary_document_copy_preserves_non_string_id_without_plaintext_lane() {
    let keyring = Arc::new(
            DbEncryptionKeyring::parse_json(
                format!(
                    r#"{{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{{"test-key":"{ROOT_KEY_BASE64}"}}}}"#
                )
                .as_bytes(),
            )
            .expect("test keyring"),
        );
    let crypto = DbMigrationCrypto::new(keyring);
    let target = MigrationTargetContext {
        environment: "dev",
        service_id: "skiff.run/plain",
        collection_name: "_skiff_c1_plain",
    };
    let original = doc! { "_id": 42_i64, "value": "ordinary" };
    let migrated = crypto
        .migrate_v1_document("skiff.run/plain", "plain", target, &[], original.clone())
        .expect("ordinary record must copy");
    assert_eq!(migrated.document, original);
    let verified = crypto
        .verify_v2_document(target, &[], &migrated.document)
        .expect("ordinary target must verify");
    assert_eq!(verified.as_bytes(), migrated.commitment.as_bytes());
}

fn v1_fixture(keyring: &DbEncryptionKeyring, plaintext: &str) -> Bson {
    let key_id = keyring.active_key_id();
    let root_key = keyring.root_key(key_id).expect("test root key");
    let info = binary_tuple(&[
        V1_FIELD_KDF_MARKER,
        key_id.as_bytes(),
        b"skiff.run/agine",
        b"providers",
        b"token",
    ])
    .expect("v1 info");
    let hkdf = Hkdf::<Sha256>::new(Some(V1_FIELD_KDF_SALT), root_key);
    let mut field_key = Zeroizing::new([0_u8; ROOT_KEY_BYTES]);
    hkdf.expand(&info, field_key.as_mut()).expect("v1 HKDF");
    let aad = binary_tuple(&[
        V1_FIELD_AAD_MARKER,
        key_id.as_bytes(),
        b"skiff.run/agine",
        b"providers",
        b"token",
        b"provider-1",
    ])
    .expect("v1 AAD");
    let nonce = [7_u8; 12];
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(field_key.as_ref()));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .expect("v1 fixture encrypt");
    let mut inner = Document::new();
    inner.insert("version", 1);
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
    Bson::Document(doc! { "_skiff_encrypted": inner })
}
