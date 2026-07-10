use std::path::{Path, PathBuf};

use crate::{parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile};

use super::*;

fn source(path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
    CompilerSourceFile::parse(
        PathBuf::from(path),
        module_path.to_string(),
        false,
        false,
        text.to_string(),
        path,
    )
    .unwrap()
}

#[test]
fn resolves_qualified_type_key_for_dotted_module_paths() {
    let sources = [
        source(
            "domain/shared.skiff",
            "domain.shared",
            "type Address { street: string }\n",
        ),
        source(
            "domain/orders.skiff",
            "domain.orders",
            "type Order { shipping: domain.shared.Address }\n",
        ),
    ];
    let parsed_sources = parse_publication_sources(Path::new("."), &sources).unwrap();
    let type_index = ServiceTypeIndex::build(&parsed_sources);

    let qualified = type_index
        .resolve_from_module("domain.orders", "domain.shared.Address")
        .unwrap();
    assert_eq!(
        qualified.source_key,
        SourceSymbolKey::new("domain.shared", "Address")
    );
    assert!(type_index
        .resolve_from_module("domain.orders", "Address")
        .is_none());
}

fn validate_storage(text: &str) -> Result<(), PublicationError> {
    let sources = [source("main.skiff", "main", text)];
    let parsed_sources = parse_publication_sources(Path::new("."), &sources).unwrap();
    validate_db_storage_sources(&parsed_sources)
}

#[test]
fn accepts_encrypted_storage_for_string_alias_and_leaves_plain_db_keys_unrestricted() {
    validate_storage(
        r#"
            alias Secret = string
            type Credential { id: string, apiKey: Secret }
            db object Credential {
              primary key(id)
              storage apiKey using encrypted
            }

            type LegacyCounter { id: number, value: string }
            db object LegacyCounter { primary key(id) }
        "#,
    )
    .unwrap();
}

#[test]
fn rejects_invalid_encrypted_storage_field_and_key_contracts() {
    let cases = [
        (
            "type Credential { id: string } db object Credential { primary key(id) storage missing using encrypted }",
            "encrypted storage field `missing` must be a field",
        ),
        (
            "type Credential { id: string } db object Credential { primary key(id) storage id using encrypted }",
            "encrypted storage field `id` cannot be the primary key",
        ),
        (
            "type Credential { id: string, apiKey: number } db object Credential { primary key(id) storage apiKey using encrypted }",
            "encrypted storage field `apiKey` must be a non-null string",
        ),
        (
            "alias SecretNumber = number type Credential { id: string, apiKey: SecretNumber } db object Credential { primary key(id) storage apiKey using encrypted }",
            "encrypted storage field `apiKey` must be a non-null string",
        ),
        (
            "type Credential { id: string, apiKey: string? } db object Credential { primary key(id) storage apiKey using encrypted }",
            "encrypted storage field `apiKey` must be a non-null string",
        ),
        (
            "type Credential { id: number, apiKey: string } db object Credential { primary key(id) storage apiKey using encrypted }",
            "must use a non-null string primary key `id`",
        ),
        (
            "type Credential { id: string, apiKey: ImmutableFile } db object Credential { primary key(id) storage apiKey using encrypted }",
            "encrypted storage field `apiKey` must be a non-null string",
        ),
        (
            "type Credential { id: string, apiKey: string } db object Credential { primary key(id) storage apiKey using encrypted storage apiKey using encrypted }",
            "storage field `apiKey` is declared more than once",
        ),
    ];

    for (text, expected) in cases {
        let error = validate_storage(text).expect_err(text);
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}

#[test]
fn rejects_encrypted_storage_in_indexes_and_partial_index_predicates() {
    let cases = [
        (
            "index byApiKey(apiKey)",
            "cannot be used by index `byApiKey`",
        ),
        (
            "index byOwner(owner) where apiKey != null",
            "cannot be used by partial index `byOwner` where",
        ),
    ];
    for (index, expected) in cases {
        let text = format!(
            "type Credential {{ id: string, owner: string, apiKey: string }} db object Credential {{ primary key(id) storage apiKey using encrypted {index} }}"
        );
        let error = validate_storage(&text).expect_err(&text);
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}
