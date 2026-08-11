use std::{fs, path::PathBuf};

use skiff_artifact_model::{
    FunctionTypeParamIr, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_compiler_input::CompilerPlatformSources;

use super::*;

const REPOSITORY_KEYS: [&str; 21] = [
    "config.DecodeError",
    "std.actor.ActivationTimeoutError",
    "std.actor.MethodInvocationTimeoutError",
    "std.bytes.DecodeError",
    "std.collection.ArrayIndexOutOfBoundsError",
    "std.collection.JsonObjectPropertyNotFoundError",
    "std.collection.MapKeyNotFoundError",
    "std.db.ConflictError",
    "std.db.ConstraintError",
    "std.db.DecodeError",
    "std.error.InstructionLimitExceededError",
    "std.error.TimeoutError",
    "std.file.FileError",
    "std.http.HttpError",
    "std.http.RequestTimeoutError",
    "std.json.DecodeError",
    "std.number.DecodeError",
    "std.service.ProtocolError",
    "std.service.ProviderUnavailableError",
    "std.time.DecodeError",
    "std.websocket.WebSocketRequestError",
];

#[test]
fn repository_catalog_resolves_exact_ordered_address_free_public_ir() {
    let platform_sources = repository_platform_sources();
    let resolved = resolve_platform_error_projection_catalog(&platform_sources).unwrap();

    assert_eq!(
        resolved
            .entries()
            .iter()
            .map(ResolvedPlatformErrorProjectionEntry::projection_key)
            .collect::<Vec<_>>(),
        REPOSITORY_KEYS
    );
    assert_eq!(resolved.entries().len(), 21);
    for entry in resolved.entries() {
        assert_eq!(entry.projection_key(), entry.nominal_identity());
        assert_eq!(
            entry.canonical_public_type_ir().name.as_str(),
            entry
                .projection_key()
                .rsplit_once('.')
                .expect("catalog keys contain a module")
                .1
        );
        assert!(entry.canonical_public_type_ir().source_span.is_none());
        validate_closed_payload(entry.canonical_public_type_ir()).unwrap();

        let serialized = serde_json::to_string(entry.canonical_public_type_ir()).unwrap();
        assert!(!serialized.contains("sourceSpan"), "{serialized}");
        assert!(
            !serialized.contains(platform_sources.root().to_string_lossy().as_ref()),
            "{serialized}"
        );
    }
}

#[test]
fn policy_is_attached_to_the_exact_public_source_declaration() {
    let fixture = PlatformFixture::new("prelude-public", "std.fixture.PublicProjectionError");
    fixture.write_source(
        "prelude/fixture.skiff",
        "type PublicProjectionError { message: string }\n",
    );
    let (_registry, resolved) = fixture.resolve().unwrap();
    let entry = &resolved.entries()[0];

    assert_eq!(entry.projection_key(), "std.fixture.PublicProjectionError");
    assert_eq!(entry.nominal_identity(), entry.projection_key());
    assert_eq!(entry.producer_family(), "fixtureProducer");
    assert_eq!(entry.semantic_adapter_owner(), "runtime.fixture");
    assert_eq!(entry.public_message_policy(), "semanticAdapterSanitized");
    assert_eq!(entry.envelope_kind(), "platformError");
    assert_eq!(entry.fallback_policy(), "fixedInternalErrorBeforeEnvelope");
    assert!(matches!(
        &entry.canonical_public_type_ir().descriptor,
        TypeDescriptorIr::Record { fields }
            if fields.get("message") == Some(&TypeRefIr::builtin("string"))
    ));
}

#[test]
fn prelude_and_std_same_module_keep_declaration_level_visibility() {
    let fixture = PlatformFixture::new("same-module-private", "std.actor.PrivateProjectionError");
    fixture.append_source(
        "std/actor.skiff",
        "\ntype PrivateProjectionError { message: string }\n",
    );
    let (registry, error) = fixture.resolve_error();

    assert!(registry.is_public_type_declaration("std.actor.ActivationTimeoutError"));
    assert!(registry
        .public_type_decl("std.actor.ActivationTimeoutError")
        .is_some());
    assert!(registry
        .exact_type_decl("std.actor.PrivateProjectionError")
        .is_some());
    assert!(!registry.is_public_type_declaration("std.actor.PrivateProjectionError"));
    assert!(matches!(
        error,
        PlatformErrorProjectionCatalogError::NonPublicTypeDeclaration { .. }
    ));
}

#[test]
fn missing_alias_interface_and_wrong_canonical_symbol_fail_closed() {
    let cases = [
        (
            "missing",
            "std.fixture.MissingProjectionError",
            None,
            "unknown",
        ),
        (
            "alias",
            "std.fixture.AliasProjectionError",
            Some("alias AliasProjectionError = string\n"),
            "alias",
        ),
        (
            "interface",
            "std.fixture.InterfaceProjectionError",
            Some("interface InterfaceProjectionError {}\n"),
            "not-type",
        ),
        ("wrong-canonical", "std.json.Json", None, "non-canonical"),
    ];

    for (name, key, source, expected) in cases {
        let fixture = PlatformFixture::new(name, key);
        if let Some(source) = source {
            fixture.write_source("prelude/fixture.skiff", source);
        }
        let (_, error) = fixture.resolve_error();
        let matches_expected = match expected {
            "unknown" => matches!(
                &error,
                PlatformErrorProjectionCatalogError::UnknownProjectionSymbol { .. }
            ),
            "alias" => matches!(
                &error,
                PlatformErrorProjectionCatalogError::AliasDeclaration { .. }
            ),
            "not-type" => matches!(
                &error,
                PlatformErrorProjectionCatalogError::NotTypeDeclaration { .. }
            ),
            "non-canonical" => matches!(
                &error,
                PlatformErrorProjectionCatalogError::NonCanonicalProjectionSymbol { .. }
            ),
            _ => false,
        };
        assert!(matches_expected, "{name}: {error}");
    }
}

#[test]
fn generic_and_representation_declarations_fail_closed() {
    for (name, key, source, generic) in [
        (
            "generic",
            "std.fixture.GenericProjectionError",
            "type GenericProjectionError<T> { value: T }\n",
            true,
        ),
        (
            "representation",
            "std.fixture.RepresentationProjectionError",
            "type RepresentationProjectionError = string\n",
            false,
        ),
    ] {
        let fixture = PlatformFixture::new(name, key);
        fixture.write_source("prelude/fixture.skiff", source);
        let (_, error) = fixture.resolve_error();
        if generic {
            assert!(matches!(
                error,
                PlatformErrorProjectionCatalogError::GenericTypeDeclaration { .. }
            ));
        } else {
            assert!(matches!(
                error,
                PlatformErrorProjectionCatalogError::OpenPayload { .. }
            ));
        }
    }
}

#[test]
fn source_lowered_forbidden_field_types_fail_closed() {
    for (name, key, source) in [
        (
            "array-field",
            "std.fixture.ArrayProjectionError",
            "type ArrayProjectionError { values: Array<string> }\n",
        ),
        (
            "bytes-field",
            "std.fixture.BytesProjectionError",
            "type BytesProjectionError { value: bytes }\n",
        ),
        (
            "nominal-field",
            "std.fixture.NominalProjectionError",
            "type NominalProjectionError { cause: std.file.FileError }\n",
        ),
    ] {
        let fixture = PlatformFixture::new(name, key);
        fixture.write_source("prelude/fixture.skiff", source);
        let (_, error) = fixture.resolve_error();
        assert!(matches!(
            error,
            PlatformErrorProjectionCatalogError::OpenPayload { .. }
        ));
    }
}

#[test]
fn function_field_source_fails_closed_during_prelude_registry_validation() {
    let fixture = PlatformFixture::new("function-field", "std.fixture.FunctionProjectionError");
    fixture.write_source(
        "prelude/fixture.skiff",
        "type FunctionProjectionError { callback: fn(value: string) -> string }\n",
    );
    let context = fixture.context();
    let snapshot = context.prelude_registry_snapshot().unwrap();
    let error = PreludeRegistry::try_from_platform_sources(&context, &snapshot).unwrap_err();

    assert!(
        error.contains("cannot reference callback function type"),
        "{error}"
    );
}

#[test]
fn closure_rejects_function_nominal_and_opaque_ir_variants() {
    let package_symbol = PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string(),
        },
        symbol_path: "std.file.FileError".to_string(),
        abi_expectation: None,
    };
    let forbidden = [
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "value".to_string(),
                ty: TypeRefIr::builtin("string"),
            }],
            return_type: Box::new(TypeRefIr::builtin("string")),
        },
        TypeRefIr::PackageSymbol {
            symbol: package_symbol.clone(),
        },
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: package_symbol,
            },
            arguments: vec![TypeRefIr::builtin("string")],
        },
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        TypeRefIr::builtin("bytes"),
    ];

    for ty in forbidden {
        assert!(validate_closed_field_type(&ty).is_err(), "accepted {ty:?}");
    }
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    CompilerPlatformSources::new(&root).unwrap()
}

struct PlatformFixture {
    base: PathBuf,
    root: PathBuf,
}

impl PlatformFixture {
    fn new(name: &str, projection_key: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "skiff-platform-error-resolver-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("platform");
        let repository = repository_platform_sources();
        for directory in ["prelude", "std"] {
            fs::create_dir_all(root.join(directory)).unwrap();
            for entry in fs::read_dir(repository.root().join(directory)).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    fs::copy(entry.path(), root.join(directory).join(entry.file_name())).unwrap();
                }
            }
        }
        fs::write(
            root.join("std/error-projections.yml"),
            format!(
                "schemaVersion: skiff-platform-error-projection-catalog-v1\nentries:\n  - projectionKey: {projection_key}\n    producerFamily: fixtureProducer\n    semanticAdapterOwner: runtime.fixture\n    publicMessagePolicy: semanticAdapterSanitized\n    envelopeKind: platformError\n    fallbackPolicy: fixedInternalErrorBeforeEnvelope\n"
            ),
        )
        .unwrap();
        Self { base, root }
    }

    fn write_source(&self, relative: &str, source: &str) {
        fs::write(self.root.join(relative), source).unwrap();
    }

    fn append_source(&self, relative: &str, source: &str) {
        let path = self.root.join(relative);
        let mut contents = fs::read_to_string(&path).unwrap();
        contents.push_str(source);
        fs::write(path, contents).unwrap();
    }

    fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }

    fn registry_and_catalog(&self) -> (PreludeRegistry, PlatformErrorProjectionCatalog) {
        initialize_prelude_registry(&repository_platform_sources()).unwrap();
        let context = self.context();
        let snapshot = context.prelude_registry_snapshot().unwrap();
        let registry = PreludeRegistry::try_from_platform_sources(&context, &snapshot).unwrap();
        let catalog = context.read_platform_error_projection_catalog().unwrap();
        (registry, catalog)
    }

    fn resolve(
        &self,
    ) -> Result<
        (PreludeRegistry, ResolvedPlatformErrorProjectionCatalog),
        PlatformErrorProjectionCatalogError,
    > {
        let (registry, catalog) = self.registry_and_catalog();
        let resolved = resolve_catalog_against_registry(&catalog, &registry)?;
        Ok((registry, resolved))
    }

    fn resolve_error(&self) -> (PreludeRegistry, PlatformErrorProjectionCatalogError) {
        let (registry, catalog) = self.registry_and_catalog();
        let error = resolve_catalog_against_registry(&catalog, &registry).unwrap_err();
        (registry, error)
    }
}

impl Drop for PlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
