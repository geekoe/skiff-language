use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    PackageDependency, PackageSourceModel, PublicationError,
};

fn compile(text: &str) -> Result<PackageSourceModel, PublicationError> {
    compile_sources(&[("main.skiff", "main", text)])
}

fn compile_sources(sources: &[(&str, &str, &str)]) -> Result<PackageSourceModel, PublicationError> {
    let root = PathBuf::from("/test/package-db-schema");
    let parsed_inputs = sources
        .iter()
        .map(|(path, module, text)| {
            CompilerSourceFile::parse(
                PathBuf::from(path),
                (*module).to_string(),
                false,
                false,
                (*text).to_string(),
                *path,
            )
            .expect("DB schema test source should parse")
        })
        .collect::<Vec<_>>();
    let parsed_sources = parse_publication_sources(&root, &parsed_inputs)
        .expect("DB schema test source facts should build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/package-db-schema"),
    })
}

fn assert_compile_error(source: &str, expected: &str) {
    let error = compile(source).expect_err(source);
    assert!(
        error.to_string().contains(expected),
        "expected {expected:?}, got {error}"
    );
}

#[test]
fn production_package_compile_preserves_valid_logical_db_schema_facts() {
    let model = compile_sources(&[
        (
            "domain/user.skiff",
            "domain.user",
            "type User { id: string, active: bool }",
        ),
        (
            "main.skiff",
            "main",
            r#"
                type Thread { id: string, owner: domain.user.User }
                db object Thread {
                  primary key(id)
                  index byOwner(owner.id)
                  unique index byActiveOwner(owner.active, owner.id)
                }
            "#,
        ),
    ])
    .expect("logical package DB schema should compile");

    let metadata = model
        .indexes()
        .publication_db_metadata_index()
        .resolve_qualified("main.Thread")
        .expect("validated DB schema must become typed source metadata");
    assert_eq!(metadata.key.name, "id");
    assert!(metadata.fields.contains("owner"));
}

#[test]
fn production_package_compile_rejects_invalid_db_attachments() {
    for (source, expected) in [
        (
            "db object Missing { primary key(id) }",
            "must attach to a same-module type declaration",
        ),
        (
            "alias Thread = string db object Thread { primary key(id) }",
            "db object Thread must attach to a type declaration, not an alias",
        ),
        (
            "type Thread { id: string } db object Thread {}",
            "db object Thread must declare key",
        ),
        (
            "type Thread { id: string } db object Thread { primary key(missing) }",
            "primary key field missing must be a field on the attached type",
        ),
        (
            "type Thread<T> { id: T } db object Thread { primary key(id) }",
            "db object Thread cannot attach to generic type Thread",
        ),
        (
            "type Thread {} db object Thread { primary key(id) }",
            "must attach to a record type with at least one field",
        ),
        (
            "type Thread { id: string, id: string } db object Thread { primary key(id) }",
            "attached type field id is declared more than once",
        ),
    ] {
        assert_compile_error(source, expected);
    }
}

#[test]
fn production_package_compile_rejects_reserved_id_and_invalid_indexes() {
    for (source, expected) in [
        (
            "type Thread { _id: string } db object Thread { primary key(_id) }",
            "db object Thread key cannot use reserved _id field",
        ),
        (
            "type Thread { id: string, _id: string } db object Thread { primary key(id) }",
            "db object Thread field cannot use reserved _id field",
        ),
        (
            "type Thread { id: string } db object Thread { primary key(id) index byId(id) index byId(id) }",
            "index name byId is declared more than once",
        ),
        (
            "type Thread { id: string } db object Thread { primary key(id) index empty() }",
            "index empty must declare at least one field",
        ),
        (
            "type Thread { id: string } db object Thread { primary key(id) index _id_(id) }",
            "index name _id_ is reserved for the primary key",
        ),
        (
            "type Thread { id: string, owner: string } db object Thread { primary key(id) index byOwner(owner, owner desc) }",
            "index byOwner declares field path owner more than once",
        ),
        (
            "type Thread { id: string, owner: string } db object Thread { primary key(id) index byOwner(owner) unique index ownerUnique(owner) }",
            "indexes byOwner and ownerUnique declare the same ordered key specification",
        ),
    ] {
        assert_compile_error(source, expected);
    }
}

#[test]
fn production_package_compile_rejects_partial_indexes_and_invalid_index_paths() {
    for (source, expected) in [
        (
            "type Thread { id: string } db object Thread { primary key(id) index byMissing(missing) }",
            "db object index missing on Thread references unknown field missing",
        ),
        (
            "type Owner { id: string } type Thread { id: string, owner: Owner } db object Thread { primary key(id) index byOwner(owner.missing) }",
            "db object index owner.missing on Thread references unknown field missing",
        ),
        (
            "type Thread { id: string } db object Thread { primary key(id) index byId(id.value) }",
            "db object index id.value on Thread cannot traverse non-record field id",
        ),
        (
            "type Owner { id: string } type Thread { id: string, owner: Owner } db object Thread { primary key(id) index byOwner(owner.id) where owner.missing != null }",
            "index byOwner uses unsupported partial index authoring; remove the where clause",
        ),
    ] {
        assert_compile_error(source, expected);
    }
}

#[test]
fn production_package_compile_accepts_only_scalar_index_keys() {
    compile(
        r#"
            type UserId = string
            alias OptionalDate = Date?
            type Nested { label: string }
            type Thread {
              id: UserId
              label: string?
              count: integer
              active: bool
              at: OptionalDate
              data: bytes
              nested: Nested?
            }
            db object Thread {
              primary key(id)
              index byLabel(label)
              index byCount(count)
              index byActive(active)
              index byAt(at)
              index byData(data)
              index byNestedLabel(nested.label)
            }
        "#,
    )
    .expect("scalar, nullable scalar, representation, alias, and nested scalar paths are valid");

    for (source, expected) in [
        (
            "type Thread { id: string, values: Array<string> } db object Thread { primary key(id) index byValues(values) }",
            "index byValues field values must be an indexable scalar or nullable scalar",
        ),
        (
            "type Thread { id: string, values: Map<string, string> } db object Thread { primary key(id) index byValues(values) }",
            "index byValues field values must be an indexable scalar or nullable scalar",
        ),
        (
            "type Nested { label: string } type Thread { id: string, nested: Nested } db object Thread { primary key(id) index byNested(nested) }",
            "index byNested field nested must be an indexable scalar or nullable scalar",
        ),
        (
            "type Thread { id: string, payload: Json } db object Thread { primary key(id) index byPayload(payload) }",
            "index byPayload field payload must be an indexable scalar or nullable scalar",
        ),
        (
            "type Thread { id: Array<string> } db object Thread { primary key(id) }",
            "primary key field id must be a non-null indexable scalar",
        ),
        (
            "type Thread { id: string? } db object Thread { primary key(id) }",
            "primary key field id must be a non-null indexable scalar",
        ),
    ] {
        assert_compile_error(source, expected);
    }
}

#[test]
fn production_package_compile_keeps_encrypted_storage_schema_rules() {
    compile(
        r#"
            alias Secret = string
            type Credential { id: string, apiKey: Secret }
            db object Credential {
              primary key(id)
              storage apiKey using encrypted
            }
        "#,
    )
    .expect("string aliases are valid encrypted storage fields");

    for (source, expected) in [
        (
            "type Credential { id: string } db object Credential { primary key(id) storage missing using encrypted }",
            "encrypted storage field `missing` must be a field",
        ),
        (
            "type Credential { id: string, secret: number } db object Credential { primary key(id) storage secret using encrypted }",
            "encrypted storage field `secret` must be a non-null string",
        ),
        (
            "type Credential { id: string } db object Credential { primary key(id) storage id using encrypted }",
            "encrypted storage field `id` cannot be the primary key",
        ),
        (
            "type Credential { id: number, secret: string } db object Credential { primary key(id) storage secret using encrypted }",
            "must use a non-null string primary key `id`",
        ),
        (
            "type Credential { id: string, secret: string } db object Credential { primary key(id) storage secret using encrypted storage secret using encrypted }",
            "storage field `secret` is declared more than once",
        ),
        (
            "type Credential { id: string, secret: string } db object Credential { primary key(id) storage secret using encrypted index bySecret(secret) }",
            "encrypted storage field `secret` cannot be used by index `bySecret`",
        ),
        (
            "type Credential { id: string, owner: string, secret: string } db object Credential { primary key(id) storage secret using encrypted index byOwner(owner) where secret != null }",
            "index byOwner uses unsupported partial index authoring; remove the where clause",
        ),
    ] {
        assert_compile_error(source, expected);
    }
}
