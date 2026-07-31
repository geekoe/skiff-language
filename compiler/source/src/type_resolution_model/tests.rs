use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    expression_type_model::ExpressionTypeModel,
    parsed_sources::{parse_publication_sources, ParsedCompilerSource},
    source_graph::CompilerSourceFile,
    ExpressionSourceMap, PublicationTypeSymbolIndex,
};
use skiff_artifact_model::{InterfaceDeclIr, InterfaceOperationIr, TypeDeclIr, TypeDeclarationIr};

use super::*;

const MODULE: &str = "internal.assignability";

fn parsed_sources(source_text: &str) -> Vec<ParsedCompilerSource> {
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/assignability.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/assignability.skiff",
    )
    .expect("test source should parse");
    parse_publication_sources(&PathBuf::from("/test"), &[source])
        .expect("test source facts should build")
}

fn type_resolution(source_text: &str) -> (Vec<ParsedCompilerSource>, TypeResolutionModel) {
    let parsed_sources = parsed_sources(source_text);
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");
    (parsed_sources, type_resolution)
}

fn package_type_resolution(source_text: &str) -> (Vec<ParsedCompilerSource>, TypeResolutionModel) {
    let parsed_sources = parsed_sources(source_text);
    let package_source = CompilerSourceFile::parse(
        PathBuf::from("pkg/reader.skiff"),
        "pkg.reader".to_string(),
        false,
        false,
        r#"
              interface Reader<T> {
                function read(self: Self, fallback: T) -> T
              }
            "#
        .to_string(),
        "pkg/reader.skiff",
    )
    .expect("package source should parse");
    let package_parsed = parse_publication_sources(&PathBuf::from("/package"), &[package_source])
        .expect("package source facts should build");
    let mut package_unit = FileIrUnit::empty("pkg.reader", "reader-package");
    package_unit.declarations.interfaces.insert(
        "Reader".to_string(),
        InterfaceDeclIr {
            name: "Reader".to_string(),
            type_params: vec!["T".to_string()],
            operations: vec![InterfaceOperationIr {
                name: "read".to_string(),
                type_params: Vec::new(),
                params: vec![
                    FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::builtin("Self"),
                    },
                    FunctionTypeParamIr {
                        name: "fallback".to_string(),
                        ty: TypeRefIr::TypeParam {
                            name: "T".to_string(),
                        },
                    },
                ],
                return_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        },
    );
    let package_facts = vec![TypeResolutionPackageFacts {
        package_id: "dep.pkg",
        dependencies: Vec::new(),
        schema_types: vec![TypeResolutionPackageSchemaTypeFact {
            public_path: "Reader",
            source_module: "pkg.reader",
            source_symbol: "Reader",
            kind: PublicTypeKind::Interface,
            source_ast: package_parsed[0].ast(),
            file_ir_unit: Some(&package_unit),
        }],
        callables: Vec::new(),
    }];
    let mut dependency = PackageDependency::id("dep.pkg");
    dependency.alias = Some("pkg".to_string());
    let package_aliases = BTreeMap::from([("pkg".to_string(), vec![String::new()])]);
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &package_aliases,
        &[dependency],
        Some(&package_facts),
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution with package facts should build");
    (parsed_sources, type_resolution)
}

fn context() -> TypeResolutionContext<'static> {
    TypeResolutionContext::source(MODULE)
}

fn initialize_test_prelude() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources = skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
        .expect("platform sources load");
    crate::prelude_registry::initialize_prelude_registry(&platform_sources)
        .expect("prelude registry initializes");
}

#[test]
fn prelude_registry_is_the_only_source_builtin_spelling_owner() {
    initialize_test_prelude();
    let (_parsed, model) = type_resolution("");

    for builtin in skiff_compiler_core::prelude_registry::file_ir_builtin_source_spellings() {
        let arguments = std::iter::repeat_n("string", builtin.arity)
            .collect::<Vec<_>>()
            .join(", ");
        let source = if arguments.is_empty() {
            builtin.source_spelling.to_string()
        } else {
            format!("{}<{arguments}>", builtin.source_spelling)
        };
        let expected_args =
            std::iter::repeat_n(TypeRefIr::builtin("string"), builtin.arity).collect();
        let resolved = model
            .resolve_type_text(&source, &context())
            .unwrap_or_else(|error| panic!("{source} should resolve: {error}"));
        assert_eq!(
            resolved.ir,
            TypeRefIr::Builtin {
                name: builtin.canonical_name.to_string(),
                args: expected_args,
            },
            "{source} must use its canonical FileIR builtin spelling"
        );
    }

    for (undeclared, expected_error) in [
        ("String", "unresolved type"),
        ("Bytes", "unresolved type"),
        ("std.date.Date", "unknown compiler-owned type"),
    ] {
        match model.resolve_type_text(undeclared, &context()) {
            Ok(resolved) => assert!(
                !matches!(resolved.ir, TypeRefIr::Builtin { .. }),
                "{undeclared} must not become an implicit FileIR builtin alias"
            ),
            Err(error) => assert!(
                error.contains(expected_error),
                "{undeclared} should fail with `{expected_error}`: {error}"
            ),
        }
    }
}

#[test]
fn nullable_union_alias_uses_one_outer_nullable_canonical_identity() {
    let (_parsed, model) = type_resolution(
        r#"
              alias Format = "png" | "jpeg" | "webp"
            "#,
    );
    let literal = |value: &str| TypeRefIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    };
    let union = TypeRefIr::Union {
        items: vec![literal("jpeg"), literal("png"), literal("webp")],
    };
    let expected_nullable = TypeRefIr::Nullable {
        inner: Box::new(union.clone()),
    };

    for spelling in [
        "Format?",
        "Format | null",
        "\"png\" | \"jpeg\" | \"webp\"?",
        "null | \"webp\" | \"png\" | \"jpeg\" | \"png\"",
    ] {
        let resolved = model
            .resolve_type_text(spelling, &context())
            .unwrap_or_else(|error| panic!("{spelling} should resolve: {error}"));
        assert_eq!(
            resolved.ir, expected_nullable,
            "{spelling} must normalize nullable over the complete union"
        );
    }

    let reordered_with_duplicates = model
        .resolve_type_text(
            "\"webp\" | \"png\" | \"jpeg\" | \"png\" | \"webp\"",
            &context(),
        )
        .expect("reordered duplicate union should resolve");
    assert_eq!(
        reordered_with_duplicates.ir, union,
        "non-null union ordering and deduplication must remain stable"
    );
}

#[test]
fn applied_nominal_resolution_preserves_ordered_nested_arguments_and_alias_targets() {
    let (_parsed, model) = type_resolution(
        r#"
              type Id = string
              type Box<T> { value: T }
              type Outer<A, B> { first: A, second: B }
              type Token<T> = string
              alias StringBox = Box<string>
            "#,
    );
    let module = model.modules.get(MODULE).expect("test module is indexed");
    let box_index = module.type_indices["Box"];
    let outer_index = module.type_indices["Outer"];
    let token_index = module.type_indices["Token"];

    let string_box = model
        .resolve_type_text("Box<string>", &context())
        .expect("generic local record resolves");
    let number_box = model
        .resolve_type_text("Box<number>", &context())
        .expect("same declaration with another argument resolves");
    assert_eq!(
        string_box.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: box_index,
            },
            arguments: vec![TypeRefIr::builtin("string")],
        }
    );
    assert_eq!(
        number_box.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: box_index,
            },
            arguments: vec![TypeRefIr::builtin("number")],
        }
    );
    assert_ne!(string_box.ir, number_box.ir);

    let nested = model
        .resolve_type_text("Outer<Box<string>, Array<Id>>", &context())
        .expect("nested nominal arguments resolve structurally");
    assert_eq!(
        nested.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: outer_index,
            },
            arguments: vec![
                string_box.ir.clone(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::LocalType {
                        type_index: module.type_indices["Id"],
                    }],
                },
            ],
        }
    );

    let token = model
        .resolve_type_text("Token<string>", &context())
        .expect("generic representation resolves");
    assert_eq!(
        token.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: token_index,
            },
            arguments: vec![TypeRefIr::builtin("string")],
        },
        "representation use must retain its nominal owner"
    );
    let alias = model
        .resolve_type_text("StringBox", &context())
        .expect("transparent alias to an applied nominal resolves");
    assert_eq!(alias.ir, string_box.ir);

    let string_fields = model
        .resolve_constructor_target_resolved(&string_box, &context())
        .expect("structured applied record is constructible")
        .fields;
    let number_fields = model
        .resolve_constructor_target_resolved(&number_box, &context())
        .expect("structured applied record is constructible")
        .fields;
    assert_eq!(string_fields["value"].ir, TypeRefIr::builtin("string"));
    assert_eq!(number_fields["value"].ir, TypeRefIr::builtin("number"));
}

#[test]
fn applied_nominal_resolution_keeps_local_and_package_owners_distinct() {
    initialize_test_prelude();
    let parsed_sources = parsed_sources("type Box<T> { value: T }");
    let package_source = CompilerSourceFile::parse(
        PathBuf::from("pkg/box.skiff"),
        "pkg.box".to_string(),
        false,
        false,
        "type Box<T> { value: T }".to_string(),
        "pkg/box.skiff",
    )
    .expect("package source parses");
    let package_parsed = parse_publication_sources(&PathBuf::from("/package"), &[package_source])
        .expect("package source facts build");
    let mut package_unit = FileIrUnit::empty("pkg.box", "generic-package");
    package_unit.type_table.push(TypeDeclIr {
        name: "Box".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        },
        type_params: vec!["T".to_string()],
        implements: Vec::new(),
        source_span: None,
    });
    package_unit.declarations.types.insert(
        "Box".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Box".to_string(),
            source_span: None,
        },
    );
    let package_facts = vec![TypeResolutionPackageFacts {
        package_id: "dep.generic",
        dependencies: Vec::new(),
        schema_types: vec![TypeResolutionPackageSchemaTypeFact {
            public_path: "Box",
            source_module: "pkg.box",
            source_symbol: "Box",
            kind: PublicTypeKind::Type,
            source_ast: package_parsed[0].ast(),
            file_ir_unit: Some(&package_unit),
        }],
        callables: Vec::new(),
    }];
    let mut dependency = PackageDependency::id("dep.generic");
    dependency.alias = Some("pkg".to_string());
    let model = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::from([("pkg".to_string(), vec![String::new()])]),
        &[dependency],
        Some(&package_facts),
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("consumer and package generic facts build");

    let local = model
        .resolve_type_text("Box<string>", &context())
        .expect("local Box resolves");
    let package = model
        .resolve_type_text("pkg.Box<string>", &context())
        .expect("package Box resolves");
    assert!(matches!(
        local.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { .. },
            ..
        }
    ));
    assert_eq!(
        package.ir,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: "pkg".to_string(),
                    },
                    symbol_path: "Box".to_string(),
                    abi_expectation: None,
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        }
    );
    assert_ne!(local.ir, package.ir);
}

#[test]
fn invalid_applied_nominal_bases_arity_and_type_param_scope_fail_closed() {
    initialize_test_prelude();
    let (_parsed, model) = type_resolution(
        r#"
              type Box<T> { value: T }
              type Plain { value: string }
              type WorkerBox { value: string }
              alias Alias = Box<string>
              interface Provider<T> {
                function get(self: Self) -> T
              }
              type Worker { id: string }
              actor Worker { key(id) }
            "#,
    );
    let cases = [
        ("Box", "expects 1 type arguments, found 0"),
        ("Box<string, number>", "expects 1 type arguments, found 2"),
        ("Box<Missing>", "unresolved type `Missing`"),
        ("Plain<string>", "expects 0 type arguments, found 1"),
        ("Alias<string>", "does not accept type arguments"),
        (
            "Provider<string>",
            "cannot be used as an applied nominal base",
        ),
        ("Worker<string>", "expects 0 type arguments, found 1"),
        ("T", "unresolved type `T`"),
    ];
    for (source, expected) in cases {
        let error = match model.resolve_type_text(source, &context()) {
            Ok(resolved) => panic!("`{source}` must fail closed, found {:?}", resolved.ir),
            Err(error) => error,
        };
        assert!(error.contains(expected), "`{source}`: {error}");
    }

    let generic_context =
        TypeResolutionContext::with_type_params(MODULE, BTreeSet::from(["T".to_string()]));
    assert_eq!(
        model
            .resolve_type_text("T", &generic_context)
            .expect("in-scope declaration parameter resolves")
            .ir,
        TypeRefIr::TypeParam {
            name: "T".to_string(),
        }
    );
    assert!(
        model
            .resolve_type_text("T<string>", &generic_context)
            .is_err(),
        "a type parameter cannot become an applied nominal base"
    );
}

#[test]
fn generic_catch_leaves_keep_applied_union_owner_and_substituted_branch_identity() {
    let (_parsed, model) = type_resolution(
        r#"
              type Branch<T> { value: T }
              type Choice<T> discriminator "kind" =
                Branch<T> |
                { kind: "inline", value: T } |
                "literal"
            "#,
    );
    let string_choice = model
        .resolve_type_text("Choice<string>", &context())
        .expect("string choice resolves");
    let number_choice = model
        .resolve_type_text("Choice<number>", &context())
        .expect("number choice resolves");
    let string_leaves = model
        .catch_leaves(&string_choice, &context())
        .expect("fully instantiated generic named union has catch leaves");
    let number_leaves = model
        .catch_leaves(&number_choice, &context())
        .expect("same generic union with another argument has catch leaves");

    assert_eq!(string_leaves.len(), 3);
    assert_eq!(number_leaves.len(), 3);
    assert_ne!(string_leaves, number_leaves);
    assert!(string_leaves.identities().iter().all(|leaf| {
        matches!(
            leaf,
            CatchLeafIdentity::NamedUnionBranch { union_type, .. }
                if union_type == &string_choice.ir
        )
    }));
    assert!(matches!(
        &string_leaves.identities()[0],
        CatchLeafIdentity::NamedUnionBranch {
            branch:
                NamedUnionBranchIr::ConcreteNominal {
                    nominal_type:
                        TypeRefIr::AppliedNominal {
                            arguments,
                            ..
                        },
                },
            ..
        } if arguments == &vec![TypeRefIr::builtin("string")]
    ));
    assert!(matches!(
        &string_leaves.identities()[1],
        CatchLeafIdentity::NamedUnionBranch {
            branch:
                NamedUnionBranchIr::SyntheticDiscriminator {
                    payload_type: TypeRefIr::Record { fields },
                    ..
                },
            ..
        } if fields["value"] == TypeRefIr::builtin("string")
    ));
    assert!(matches!(
        &string_leaves.identities()[2],
        CatchLeafIdentity::NamedUnionBranch {
            branch: NamedUnionBranchIr::Literal { .. },
            ..
        }
    ));
}

fn signature_rehydration_artifact() -> PackageArtifact {
    use skiff_artifact_model::{
        PackageImplementationLinks, PackageLocalAbi, PackageRuntimeRequirements,
        PackageSchemaIndexRef, TypeExport,
    };

    let file = skiff_artifact_model::FileIrRef {
        file_ir_identity: "provider-file".to_string(),
        artifact_path: Some("types.json".to_string()),
        module_path: "types".to_string(),
        source_ast_hash: Some("provider-source".to_string()),
    };
    let descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::new(),
    };
    let type_symbol = |public_path: &str| PackageLocalAbiSymbol::Type {
        local_type_id: format!("type:{public_path}"),
        descriptor: descriptor.clone(),
        is_alias: false,
        is_interface: false,
        type_params: Vec::new(),
        interface_methods: Vec::new(),
    };
    let type_export = |type_index, symbol: &str| TypeExport {
        file: file.clone(),
        type_index,
        symbol: symbol.to_string(),
        is_interface: false,
        descriptor: Some(descriptor.clone()),
        type_params: Vec::new(),
        interface_methods: Vec::new(),
    };
    PackageArtifact {
        schema_version: "skiff-package-artifact-v9".to_string(),
        package_id: "example.com/provider".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("provider-build"),
        files: vec![file.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("provider-abi"),
            public_symbols: BTreeMap::from([
                ("Bindings".to_string(), type_symbol("Bindings")),
                ("Result".to_string(), type_symbol("Result")),
            ]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.com/provider".to_string(),
            package_schema_index_identity: "provider-schema-index".into(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::from([
                ("Bindings".to_string(), type_export(0, "Bindings")),
                ("Result".to_string(), type_export(1, "Result")),
            ]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

#[test]
fn public_and_top_level_views_are_isolated_and_emit_one_canonical_dependency_ref() {
    initialize_test_prelude();
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let mut dependency = PackageDependency::id("example.com/provider");
    dependency.alias = Some("provider".to_string());
    dependency.top_level_alias = Some("providerImpl".to_string());
    let mut artifact = signature_rehydration_artifact();
    let descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::new(),
    };
    artifact.package_local_abi.implementation_symbols = BTreeMap::from([
        (
            "Bindings".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:example.com/provider:top-level:Bindings".to_string(),
                descriptor: descriptor.clone(),
                is_alias: false,
                is_interface: false,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        ),
        (
            "internal.Private".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:example.com/provider:top-level:internal.Private".to_string(),
                descriptor: descriptor.clone(),
                is_alias: false,
                is_interface: false,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        ),
    ]);
    artifact.implementation_links.types.insert(
        "internal.Private".to_string(),
        skiff_artifact_model::TypeExport {
            file: artifact.files[0].clone(),
            type_index: 2,
            symbol: "Private".to_string(),
            is_interface: false,
            descriptor: Some(descriptor),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    );
    let model = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::from([
            ("provider".to_string(), vec![String::new()]),
            ("providerImpl".to_string(), Vec::new()),
        ]),
        &[dependency],
        None,
        Some(std::slice::from_ref(&artifact)),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("the two source views should index one exact artifact");

    let public = model
        .resolve_type_text("provider.Bindings", &context())
        .expect("primary alias should resolve public symbols");
    let top_level = model
        .resolve_type_text("providerImpl/Bindings", &context())
        .expect("top-level alias should resolve implementation symbols");
    assert_ne!(
        public.ir, top_level.ir,
        "source typing must retain which permission view produced the value"
    );
    assert!(model.assignable_in_context(&public, &top_level, &context()));
    assert!(model.assignable_in_context(&top_level, &public, &context()));
    assert!(matches!(
        top_level.ir,
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency { ref dependency_ref },
                ..
            },
        } if dependency_ref == "providerImpl"
    ));
    assert!(
        model
            .package_type_resolution_for_view("provider", "internal.Private")
            .is_none(),
        "primary alias must not fall back to implementation symbols"
    );
    assert!(
        model
            .package_type_resolution_for_view("providerImpl", "Result")
            .is_none(),
        "top-level alias must not fall back to public symbols"
    );
    assert_eq!(
        model.package_dependency_abi_expectations(),
        BTreeMap::from([("provider".to_string(), "provider-abi".to_string())]),
        "lowering must see one canonical dependency ref"
    );
    assert_eq!(
        model.package_dependency_abi_expectations_by_package_id(),
        BTreeMap::from([(
            "example.com/provider".to_string(),
            "provider-abi".to_string()
        )]),
        "canonical package-id refs must retain the selected exact ABI"
    );
}

#[test]
fn package_receiver_resolution_requires_exact_top_level_owner_and_closed_generics() {
    initialize_test_prelude();
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let mut dependency = PackageDependency::id("example.com/provider");
    dependency.alias = Some("provider".to_string());
    dependency.top_level_alias = Some("providerImpl".to_string());
    let mut artifact = signature_rehydration_artifact();
    let descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::new(),
    };
    artifact.package_local_abi.implementation_symbols.insert(
        "internal.Box".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: "type:example.com/provider:top-level:internal.Box".to_string(),
            descriptor: descriptor.clone(),
            is_alias: false,
            is_interface: false,
            type_params: vec!["T".to_string()],
            interface_methods: Vec::new(),
        },
    );
    artifact.implementation_links.types.insert(
        "internal.Box".to_string(),
        skiff_artifact_model::TypeExport {
            file: artifact.files[0].clone(),
            type_index: 2,
            symbol: "Box".to_string(),
            is_interface: false,
            descriptor: Some(descriptor),
            type_params: vec!["T".to_string()],
            interface_methods: Vec::new(),
        },
    );
    let model = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::from([
            ("provider".to_string(), vec![String::new()]),
            ("providerImpl".to_string(), Vec::new()),
        ]),
        &[dependency],
        None,
        Some(std::slice::from_ref(&artifact)),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("exact package receiver fixture should build");
    let symbol = |dependency_ref: &str, abi_expectation: Option<&str>| PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: dependency_ref.to_string(),
        },
        symbol_path: "internal.Box".to_string(),
        abi_expectation: abi_expectation.map(str::to_string),
    };
    let applied = |symbol: PackageSymbolRef, arguments: Vec<TypeRefIr>| TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol { symbol },
        arguments,
    };

    let exact = applied(
        symbol("providerImpl", Some("provider-abi")),
        vec![TypeRefIr::builtin("string")],
    );
    let resolved = model
        .package_receiver_method_resolution(&exact, "read")
        .expect("direct top-level applied nominal should authorize its exact method");
    assert_eq!(resolved.dependency_ref, "providerImpl");
    assert_eq!(resolved.canonical_dependency_ref, "provider");
    assert_eq!(resolved.expected_local_abi.as_str(), "provider-abi");
    assert_eq!(resolved.expected_package_build.as_str(), "provider-build");
    assert_eq!(resolved.source_method_path, "internal.Box.read");
    assert_eq!(
        resolved.receiver_type_arguments,
        vec![TypeRefIr::builtin("string")]
    );

    for rejected in [
        applied(
            symbol("provider", Some("provider-abi")),
            vec![TypeRefIr::builtin("string")],
        ),
        applied(
            symbol("providerImpl", Some("wrong-abi")),
            vec![TypeRefIr::builtin("string")],
        ),
        applied(symbol("providerImpl", Some("provider-abi")), Vec::new()),
        applied(
            symbol("providerImpl", Some("provider-abi")),
            vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
        ),
        applied(
            symbol("providerImpl", Some("provider-abi")),
            vec![TypeRefIr::TypeParam {
                name: "T".to_string(),
            }],
        ),
        applied(
            PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "providerImpl".to_string(),
                },
                symbol_path: "internal.Other".to_string(),
                abi_expectation: Some("provider-abi".to_string()),
            },
            vec![TypeRefIr::builtin("string")],
        ),
        applied(
            PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/provider".to_string(),
                },
                symbol_path: "internal.Box".to_string(),
                abi_expectation: Some("provider-abi".to_string()),
            },
            vec![TypeRefIr::builtin("string")],
        ),
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "provider".to_string(),
                symbol: "Box".to_string(),
            },
        },
        TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: "interface:provider:Box".to_string(),
                canonical_type_args: vec![TypeRefIr::builtin("string")],
            },
        },
    ] {
        assert!(
            model
                .package_receiver_method_resolution(&rejected, "read")
                .is_none(),
            "receiver discovery must fail closed for {rejected:?}"
        );
    }
}

#[test]
fn local_receiver_resolution_requires_exact_applied_owner_arity_and_closed_arguments() {
    initialize_test_prelude();
    let (_parsed, model) = type_resolution(
        r#"
              type Box<T> { value: T }
              type Other<T> { value: T }

              impl Box<T> {
                function unwrap() -> T {
                  return self.value
                }
              }
            "#,
    );
    let exact = model
        .resolve_type_text("Box<string>", &context())
        .expect("Box<string> resolves");
    let resolved = model
        .local_receiver_method_resolution(&exact.ir, "unwrap", &context())
        .expect("the exact applied receiver should authorize Box<T>.unwrap");
    assert_eq!(
        resolved.source_callable,
        SourceSymbolKey::new(MODULE, "Box<T>.unwrap")
    );
    assert_eq!(
        resolved.receiver_type_arguments,
        vec![TypeRefIr::builtin("string")]
    );

    let TypeRefIr::AppliedNominal { base, .. } = exact.ir else {
        panic!("Box<string> must be an applied nominal");
    };
    let wrong_owner = model
        .resolve_type_text("Other<string>", &context())
        .expect("Other<string> resolves");
    let TypeRefIr::AppliedNominal {
        base: wrong_owner_base,
        ..
    } = wrong_owner.ir
    else {
        panic!("Other<string> must be an applied nominal");
    };
    for rejected in [
        TypeRefIr::AppliedNominal {
            base: wrong_owner_base,
            arguments: vec![TypeRefIr::builtin("string")],
        },
        TypeRefIr::AppliedNominal {
            base: base.clone(),
            arguments: Vec::new(),
        },
        TypeRefIr::AppliedNominal {
            base: base.clone(),
            arguments: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
        },
        TypeRefIr::AppliedNominal {
            base,
            arguments: vec![TypeRefIr::TypeParam {
                name: "Open".to_string(),
            }],
        },
    ] {
        assert!(
            model
                .local_receiver_method_resolution(&rejected, "unwrap", &context())
                .is_none(),
            "local receiver discovery must fail closed for {rejected:?}"
        );
    }
}

#[test]
fn package_signature_exact_symbols_rehydrate_and_ownerless_slots_fail_closed() {
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let mut dependency = PackageDependency::id("example.com/provider");
    dependency.alias = Some("provider".to_string());
    let artifact = signature_rehydration_artifact();
    let model = TypeResolutionModel::build(
        &parsed_sources,
        &BTreeMap::from([("provider".to_string(), vec![String::new()])]),
        &[dependency],
        None,
        Some(std::slice::from_ref(&artifact)),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("artifact-only dependency type facts should build");

    let dependency_symbol = |symbol_path: &str| TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "provider".to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: Some("provider-abi".to_string()),
        },
    };
    let interface_identity = serde_json::to_string(&TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "types".to_string(),
            symbol: "Bindings".to_string(),
        },
    })
    .unwrap();
    let signature_type = PackageTypeRef::Local {
        local_type: TypeRefIr::Function {
            params: vec![
                FunctionTypeParamIr {
                    name: "service".to_string(),
                    ty: TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "types".to_string(),
                            symbol: "Bindings".to_string(),
                        },
                    },
                },
                FunctionTypeParamIr {
                    name: "publication".to_string(),
                    ty: TypeRefIr::PublicationType {
                        module_path: "types".to_string(),
                        type_index: 1,
                    },
                },
                FunctionTypeParamIr {
                    name: "nested".to_string(),
                    ty: TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::Nullable {
                            inner: Box::new(TypeRefIr::Union {
                                items: vec![
                                    TypeRefIr::ServiceSymbol {
                                        symbol: ServiceSymbolRef {
                                            module_path: "types".to_string(),
                                            symbol: "Bindings".to_string(),
                                        },
                                    },
                                    TypeRefIr::PublicationType {
                                        module_path: "types".to_string(),
                                        type_index: 1,
                                    },
                                ],
                            }),
                        }],
                    },
                },
            ],
            return_type: Box::new(TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "service".to_string(),
                        TypeRefIr::ServiceSymbol {
                            symbol: ServiceSymbolRef {
                                module_path: "types".to_string(),
                                symbol: "Bindings".to_string(),
                            },
                        },
                    ),
                    (
                        "package".to_string(),
                        TypeRefIr::PackageSymbol {
                            symbol: PackageSymbolRef {
                                package: PackageRefIr::PackageId {
                                    package_id: "example.com/provider".to_string(),
                                },
                                symbol_path: "types.Result".to_string(),
                                abi_expectation: None,
                            },
                        },
                    ),
                    (
                        "interface".to_string(),
                        TypeRefIr::AnyInterface {
                            interface: InterfaceInstantiationRef {
                                interface_abi_id: interface_identity,
                                canonical_type_args: vec![TypeRefIr::PublicationType {
                                    module_path: "types".to_string(),
                                    type_index: 1,
                                }],
                            },
                        },
                    ),
                ]),
            }),
        },
    };
    let normalized = model
        .rehydrate_package_signature_type_for_dependency("provider", &signature_type)
        .expect("all public owner-local references should normalize");
    let PackageTypeRef::Local {
        local_type: TypeRefIr::Function {
            params,
            return_type,
        },
    } = normalized
    else {
        panic!("normalized signature should retain its function shape")
    };
    assert_eq!(params[0].ty, dependency_symbol("Bindings"));
    assert_eq!(params[1].ty, dependency_symbol("Result"));
    assert_eq!(
        params[2].ty,
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::Union {
                    items: vec![dependency_symbol("Bindings"), dependency_symbol("Result"),],
                }),
            }],
        }
    );
    let TypeRefIr::Record { fields } = return_type.as_ref() else {
        panic!("normalized return should retain its record shape")
    };
    assert_eq!(fields["service"], dependency_symbol("Bindings"));
    assert_eq!(fields["package"], dependency_symbol("Result"));
    let TypeRefIr::AnyInterface { interface } = &fields["interface"] else {
        panic!("normalized nested interface should retain its existential shape")
    };
    assert_eq!(
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).unwrap(),
        dependency_symbol("Bindings")
    );
    assert_eq!(
        interface.canonical_type_args,
        vec![dependency_symbol("Result")]
    );
    let wrapped = PackageTypeRef::Container {
        name: "Array".to_string(),
        arguments: vec![PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::AnyInterface {
                interface: Box::new(PackageTypeRef::Local {
                    local_type: TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "types".to_string(),
                            symbol: "Bindings".to_string(),
                        },
                    },
                }),
                arguments: vec![PackageTypeRef::Local {
                    local_type: TypeRefIr::PublicationType {
                        module_path: "types".to_string(),
                        type_index: 1,
                    },
                }],
            }),
        }],
    };
    assert_eq!(
        model
            .rehydrate_package_signature_type_for_dependency("provider", &wrapped)
            .unwrap(),
        PackageTypeRef::Container {
            name: "Array".to_string(),
            arguments: vec![PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::AnyInterface {
                    interface: Box::new(PackageTypeRef::Local {
                        local_type: dependency_symbol("Bindings"),
                    }),
                    arguments: vec![PackageTypeRef::Local {
                        local_type: dependency_symbol("Result"),
                    }],
                }),
            }],
        }
    );

    let exact_schema = PackageTypeRef::PackageSchema {
        package_id: "example.com/provider".to_string(),
        stable_schema_key: "Result".to_string(),
        package_schema_type_id: "schema-result".into(),
    };
    assert_eq!(
        model
            .rehydrate_package_signature_type_for_dependency("provider", &exact_schema)
            .unwrap(),
        exact_schema,
        "exact PackageSchema owner/key/type id must remain unchanged"
    );

    let error = model
        .rehydrate_package_signature_type_for_dependency(
            "provider",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 0 },
            },
        )
        .unwrap_err();
    assert!(
        error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
        "{error}"
    );

    let mut ambiguous = model.clone();
    ambiguous.package_type_slots.insert(
        ("provider".to_string(), "other".to_string(), 0),
        "other.Bindings".to_string(),
    );
    let error = ambiguous
        .rehydrate_package_signature_type_for_dependency(
            "provider",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 0 },
            },
        )
        .unwrap_err();
    assert!(
        error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
        "{error}"
    );

    let error = model
        .rehydrate_package_signature_type_for_dependency(
            "provider",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "private".to_string(),
                        symbol: "Hidden".to_string(),
                    },
                },
            },
        )
        .unwrap_err();
    assert!(error.contains("no unique public Local ABI type"), "{error}");
}

#[test]
fn compiler_owned_package_owner_rejects_ownerless_package_signature_slots() {
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let artifact = signature_rehydration_artifact();
    let dependencies = compiler_owned_dependencies(&artifact);
    let model = TypeResolutionModel::build_with_compiler_owned_packages(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        Some(std::slice::from_ref(&artifact)),
        &dependencies,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("compiler-owned artifact owner should build");

    assert_eq!(
        model.package_dependencies.get("std").map(String::as_str),
        Some("example.com/provider")
    );
    let error = model
        .rehydrate_package_signature_type_for_dependency(
            "std",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 0 },
            },
        )
        .unwrap_err();
    assert!(
        error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
        "{error}"
    );
}

#[test]
fn compiler_owned_package_owner_requires_one_exact_artifact() {
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let artifact = signature_rehydration_artifact();
    let dependencies = compiler_owned_dependencies(&artifact);
    for (artifacts, expected_count) in [
        (Vec::new(), 0),
        (vec![artifact.clone(), artifact.clone()], 2),
    ] {
        let error = TypeResolutionModel::build_with_compiler_owned_packages(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            Some(&artifacts),
            &dependencies,
            &PublicationTypeSymbolIndex::default(),
        )
        .unwrap_err();
        assert!(
            error.contains(&format!(
                "requires exactly one verified package artifact owner, found {expected_count}"
            )),
            "{error}"
        );
    }
}

#[test]
fn compiler_owned_available_artifacts_require_explicit_owner_facts() {
    let parsed_sources = parsed_sources("function noop() -> void {}");
    let artifact = signature_rehydration_artifact();
    let model = TypeResolutionModel::build_with_compiler_owned_packages(
        &parsed_sources,
        &BTreeMap::new(),
        &[],
        None,
        Some(std::slice::from_ref(&artifact)),
        &SourceDependencyAnalysisInput::default(),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("unselected available artifacts must stay outside type resolution");
    assert!(model.package_dependencies.is_empty());
    assert!(model.package_artifact_identities.is_empty());
    assert!(model.package_types.is_empty());
}

fn compiler_owned_dependencies(artifact: &PackageArtifact) -> SourceDependencyAnalysisInput {
    SourceDependencyAnalysisInput::new(
        [(
            "std".to_string(),
            crate::PackageDependencyAnalysisFacts::new(
                artifact.package_build_id.clone(),
                artifact.package_local_abi.local_abi_identity.clone(),
                BTreeMap::new(),
            )
            .compiler_owned(),
        )],
        [],
    )
    .unwrap()
}

fn conformance_source() -> &'static str {
    r#"
          interface I<T> {}

          type Box<T> implements I<T> {
            value: T,
          }

          type Payload {
            value: string,
          }

          type Wrapped = Box<string>
        "#
}

fn resolved_test_interface(argument: TypeRefIr) -> ResolvedTypeRef {
    let identity = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: MODULE.to_string(),
            symbol: "I".to_string(),
        },
    };
    let text = format!("I<{}>", debug_text(&argument));
    ResolvedTypeRef::with_text(
        TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(identity, vec![argument]),
        },
        text,
    )
}

fn object_safe_interface_source() -> &'static str {
    r#"
          interface Provider {
            function name(self: Self) -> string
          }

          interface Box<T> {
            function get(self: Self) -> T
          }

          type Concrete {
            value: string,
          }

          alias ProviderAlias = Provider
        "#
}

fn package_reader_conformance_source() -> &'static str {
    r#"
          type Host implements pkg.Reader<string> {
            value: string,
          }

          impl Host {
            function read(fallback: string) -> string {
              return fallback
            }
          }
        "#
}

#[test]
fn any_interface_selector_resolution_rejects_non_interface_targets() {
    let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
    let context = context();

    let any_provider = type_resolution
        .resolve_type_text("any Provider", &context)
        .expect("object-safe interface selector should resolve");
    assert!(
        matches!(any_provider.ir, TypeRefIr::AnyInterface { .. }),
        "any Provider should resolve to TypeRefIr::AnyInterface"
    );
    let provider = type_resolution
        .resolve_type_text("Provider", &context)
        .expect("bare Provider should resolve as a named type");
    type_resolution
        .resolve_canonical_interface_selector_resolved_type_ref(&provider, &context)
        .expect("resolved Provider should validate as a canonical interface selector");

    for (raw, expected) in [
        ("any string", "primitive/builtin"),
        ("any Concrete", "concrete type"),
        ("any ProviderAlias", "alias"),
        ("any { value: string }", "anonymous record"),
        ("any any Provider", "nested `any`"),
        ("any Box", "expects 1 type arguments"),
    ] {
        let error = type_resolution
            .resolve_type_text(raw, &context)
            .expect_err("invalid interface selector should fail");
        assert!(
            error.contains(expected),
            "expected `{raw}` error to contain `{expected}`, got: {error}"
        );
    }
}

#[test]
fn externalized_any_interface_source_text_remains_parseable() {
    let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
    let context = context();
    let resolved = type_resolution
        .resolve_type_text("any Provider", &context)
        .expect("any Provider should resolve");

    let externalized = type_resolution.externalize_local_type_refs(&resolved, MODULE);

    assert_eq!(
        externalized.to_string(),
        "any internal.assignability.Provider"
    );
    let reparsed = type_resolution
        .resolve_type_text(&externalized.to_string(), &context)
        .expect("externalized interface text should remain valid source syntax");
    assert_eq!(reparsed.ir, externalized.ir);
}

#[test]
fn map_key_rejects_any_interface_without_rejecting_map_value() {
    let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
    let context = context();

    type_resolution
        .resolve_type_text("Map<string, any Provider>", &context)
        .expect("any interface should be allowed in Map value position");
    let error = type_resolution
        .resolve_type_text("Map<any Provider, string>", &context)
        .expect_err("any interface map key should fail at source type resolution");
    assert!(
        error.contains("Map key type"),
        "unexpected Map key diagnostic: {error}"
    );
}

#[test]
fn any_package_interface_method_signature_substitutes_interface_type_args() {
    let (_parsed_sources, type_resolution) =
        package_type_resolution(package_reader_conformance_source());
    let context = context();
    let any_reader = type_resolution
        .resolve_type_text("any pkg.Reader<string>", &context)
        .expect("package any interface should resolve");

    let read = type_resolution
        .any_interface_method_signature(&any_reader.ir, "read")
        .expect("Reader.read should resolve on any package interface");

    assert_eq!(read.params.len(), 2);
    assert_eq!(read.params[0].name, "self");
    assert_eq!(read.params[0].ty, TypeRefIr::builtin("Self"));
    assert_eq!(read.params[1].name, "fallback");
    assert_eq!(read.params[1].ty, TypeRefIr::builtin("string"));
    assert_eq!(read.return_type, TypeRefIr::builtin("string"));
    assert!(!read.method_abi_id.is_empty());
}

#[test]
fn local_conformance_lookup_accepts_package_interface_selector() {
    let (_parsed_sources, type_resolution) =
        package_type_resolution(package_reader_conformance_source());
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Host", &context)
        .expect("Host should resolve");
    let expected = type_resolution
        .resolve_type_text("any pkg.Reader<string>", &context)
        .expect("package interface should resolve");

    let conformance = type_resolution
        .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
        .expect("package selector conformance lookup should not report source-only selector errors")
        .expect("Host should conform to pkg.Reader<string>");

    assert_eq!(conformance.receiver, SourceSymbolKey::new(MODULE, "Host"));
    assert!(matches!(
        serde_json::from_str::<TypeRefIr>(&conformance.interface.interface_abi_id)
            .expect("interface abi id should decode"),
        TypeRefIr::PackageSymbol { .. }
    ));
    assert_eq!(
        conformance.interface.canonical_type_args,
        vec![TypeRefIr::builtin("string")]
    );
    assert_eq!(conformance.slots.len(), 1);
    let slot = &conformance.slots[0];
    assert_eq!(slot.slot, 0);
    assert_eq!(slot.name, "read");
    assert_eq!(
        slot.params,
        vec![
            FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::ServiceSymbol {
                    symbol: service_symbol_ref_from_source_key(&SourceSymbolKey::new(
                        MODULE, "Host"
                    )),
                },
            },
            FunctionTypeParamIr {
                name: "fallback".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        ]
    );
    assert_eq!(slot.return_type, TypeRefIr::builtin("string"));
}

#[test]
fn package_interface_conformance_matches_public_alias_signature_types() {
    let parsed_sources = parsed_sources(
        r#"
              import agent
              import api

              type Host implements agent.llm.Client {}

              impl Host {
                function stream(input: agent.llm.Request) -> Stream<agent.llm.Event> {
                  return null
                }
              }
            "#,
    );
    let api_source = CompilerSourceFile::parse(
        PathBuf::from("api/types.skiff"),
        "api.types".to_string(),
        false,
        false,
        r#"
              type Request {
                text: string,
              }

              type Event {
                text: string,
              }
            "#
        .to_string(),
        "api/types.skiff",
    )
    .expect("api package source should parse");
    let api_parsed = parse_publication_sources(&PathBuf::from("/api"), &[api_source])
        .expect("api package source facts should build");
    let agent_source = CompilerSourceFile::parse(
        PathBuf::from("agent/llm.skiff"),
        "agent.llm".to_string(),
        false,
        false,
        r#"
              import api

              alias Request = api.Request
              alias Event = api.Event

              interface Client {
                function stream(self: Self, input: Request) -> Stream<Event>
              }
            "#
        .to_string(),
        "agent/llm.skiff",
    )
    .expect("agent package source should parse");
    let agent_parsed = parse_publication_sources(&PathBuf::from("/agent"), &[agent_source])
        .expect("agent package source facts should build");
    let api_request = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "api".to_string(),
            },
            symbol_path: "Request".to_string(),
            abi_expectation: None,
        },
    };
    let api_event = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "api".to_string(),
            },
            symbol_path: "Event".to_string(),
            abi_expectation: None,
        },
    };
    let mut agent_unit = FileIrUnit::empty("agent.llm", "agent-package");
    agent_unit.declarations.interfaces.insert(
        "Client".to_string(),
        InterfaceDeclIr {
            name: "Client".to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: "stream".to_string(),
                type_params: Vec::new(),
                params: vec![
                    FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::builtin("Self"),
                    },
                    FunctionTypeParamIr {
                        name: "input".to_string(),
                        ty: api_request,
                    },
                ],
                return_type: TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![api_event],
                },
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        },
    );
    let package_facts = vec![
        TypeResolutionPackageFacts {
            package_id: "api.pkg",
            dependencies: Vec::new(),
            schema_types: vec![
                TypeResolutionPackageSchemaTypeFact {
                    public_path: "Request",
                    source_module: "api.types",
                    source_symbol: "Request",
                    kind: PublicTypeKind::Type,
                    source_ast: api_parsed[0].ast(),
                    file_ir_unit: None,
                },
                TypeResolutionPackageSchemaTypeFact {
                    public_path: "Event",
                    source_module: "api.types",
                    source_symbol: "Event",
                    kind: PublicTypeKind::Type,
                    source_ast: api_parsed[0].ast(),
                    file_ir_unit: None,
                },
            ],
            callables: Vec::new(),
        },
        TypeResolutionPackageFacts {
            package_id: "agent.pkg",
            dependencies: vec![TypeResolutionPackageDependencyFact {
                alias: "api",
                package_id: "api.pkg",
            }],
            schema_types: vec![
                TypeResolutionPackageSchemaTypeFact {
                    public_path: "llm.Request",
                    source_module: "agent.llm",
                    source_symbol: "Request",
                    kind: PublicTypeKind::Alias,
                    source_ast: agent_parsed[0].ast(),
                    file_ir_unit: None,
                },
                TypeResolutionPackageSchemaTypeFact {
                    public_path: "llm.Event",
                    source_module: "agent.llm",
                    source_symbol: "Event",
                    kind: PublicTypeKind::Alias,
                    source_ast: agent_parsed[0].ast(),
                    file_ir_unit: None,
                },
                TypeResolutionPackageSchemaTypeFact {
                    public_path: "llm.Client",
                    source_module: "agent.llm",
                    source_symbol: "Client",
                    kind: PublicTypeKind::Interface,
                    source_ast: agent_parsed[0].ast(),
                    file_ir_unit: Some(&agent_unit),
                },
            ],
            callables: Vec::new(),
        },
    ];
    let mut agent_dependency = PackageDependency::id("agent.pkg");
    agent_dependency.alias = Some("agent".to_string());
    let mut api_dependency = PackageDependency::id("api.pkg");
    api_dependency.alias = Some("api".to_string());
    let package_aliases = BTreeMap::from([
        ("agent".to_string(), vec![String::new()]),
        ("api".to_string(), vec![String::new()]),
    ]);
    let type_resolution = TypeResolutionModel::build(
        &parsed_sources,
        &package_aliases,
        &[agent_dependency, api_dependency],
        Some(&package_facts),
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution with package alias facts should build");
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Host", &context)
        .expect("Host should resolve");
    let expected = type_resolution
        .resolve_type_text("agent.llm.Client", &context)
        .expect("package interface should resolve");

    assert!(
            type_resolution
                .concrete_type_conforms_to_interface(&actual, &expected, &context)
                .expect("conformance lookup should not fail")
                .is_some(),
            "package public aliases in interface method signatures should match service implementation signatures"
        );
}

#[test]
fn package_interface_conformance_rejects_local_impl_signature_mismatch() {
    let (_parsed_sources, type_resolution) = package_type_resolution(
        r#"
              type Host implements pkg.Reader<string> {
                value: string,
              }

              impl Host {
                function read(fallback: number) -> string {
                  return "bad"
                }
              }
            "#,
    );
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Host", &context)
        .expect("Host should resolve");
    let expected = type_resolution
        .resolve_type_text("any pkg.Reader<string>", &context)
        .expect("package interface should resolve");

    assert!(
        type_resolution
            .concrete_type_conforms_to_interface(&actual, &expected, &context)
            .expect("package conformance lookup should not fail")
            .is_none(),
        "package conformance must fail closed when local impl method signature mismatches"
    );
    assert!(
        type_resolution
            .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
            .expect("package selector conformance lookup should not fail")
            .is_none(),
        "local method table slots must not be generated for mismatched package conformance"
    );
}

#[test]
fn ordinary_assignability_does_not_use_interface_conformance() {
    let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Box<string>", &context)
        .expect("actual type should resolve");
    let expected = resolved_test_interface(TypeRefIr::builtin("string"));

    assert!(
        !type_resolution.assignable_in_context(&actual, &expected, &context),
        "ordinary value assignability must not treat implements I as implicit interface boxing"
    );
}

#[test]
fn concrete_type_conformance_matches_declared_interface_instantiation() {
    let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Box<string>", &context)
        .expect("actual type should resolve");
    let expected = resolved_test_interface(TypeRefIr::builtin("string"));

    let matched = type_resolution
        .concrete_type_conforms_to_interface(&actual, &expected, &context)
        .expect("conformance lookup should not fail")
        .expect("Box<string> should conform to I<string>");

    assert_eq!(
        matched.receiver,
        SourceSymbolKey::new(MODULE, "Box"),
        "match should report the concrete receiver symbol"
    );
    assert_eq!(
        matched.implemented_interface_args,
        vec![TypeRefIr::builtin("string")]
    );
    assert_eq!(
        matched.expected_interface_args,
        vec![TypeRefIr::builtin("string")]
    );
}

#[test]
fn concrete_type_conformance_rejects_mismatched_interface_args() {
    let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
    let context = context();
    let actual = type_resolution
        .resolve_type_text("Box<string>", &context)
        .expect("actual type should resolve");
    let expected = resolved_test_interface(TypeRefIr::builtin("number"));

    assert!(
        type_resolution
            .concrete_type_conforms_to_interface(&actual, &expected, &context)
            .expect("conformance lookup should not fail")
            .is_none(),
        "Box<string> must not conform to I<number>"
    );
}

#[test]
fn concrete_type_conformance_requires_exact_nominal_receiver_and_interface() {
    let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
    let context = context();
    let expected = resolved_test_interface(TypeRefIr::builtin("string"));

    let nullable = type_resolution
        .resolve_type_text("Box<string>?", &context)
        .expect("nullable actual should resolve");
    let union = type_resolution
        .resolve_type_text("Box<string> | null", &context)
        .expect("union actual should resolve");
    let record = ResolvedTypeRef::with_text(
        TypeRefIr::Record {
            fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("string"))]),
        },
        "{ value: string }".to_string(),
    );
    let representation = type_resolution
        .resolve_type_text("Wrapped", &context)
        .expect("representation actual should resolve");
    let non_interface = type_resolution
        .resolve_type_text("Payload", &context)
        .expect("non-interface expected should resolve");

    for actual in [&nullable, &union, &record, &representation] {
        assert!(
                type_resolution
                    .concrete_type_conforms_to_interface(actual, &expected, &context)
                    .expect("conformance lookup should not fail")
                    .is_none(),
                "{:?} must not conform through nullable, union, record shape, or representation payload",
                actual.ir
            );
    }
    assert!(
        type_resolution
            .concrete_type_conforms_to_interface(&representation, &non_interface, &context)
            .expect("non-interface expected should not fail")
            .is_none(),
        "non-interface expected type should return None"
    );
}

#[test]
fn json_contextual_assignability_remains_ordinary_value_behavior() {
    let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
    let context = context();
    let payload = type_resolution
        .resolve_type_text("Payload", &context)
        .expect("payload should resolve");
    let json = type_resolution
        .resolve_type_text("Json", &context)
        .expect("Json should resolve");
    let json_object = type_resolution
        .resolve_type_text("JsonObject", &context)
        .expect("JsonObject should resolve");

    assert!(type_resolution.assignable_in_context(&payload, &json, &context));
    assert!(type_resolution.assignable_in_context(&payload, &json_object, &context));
}

#[test]
fn function_argument_check_does_not_implicitly_box_concrete_to_interface() {
    let (parsed_sources, type_resolution) = type_resolution(
        r#"
              interface I {}

              type Concrete implements I {
                value: string,
              }

              function accepts(input: I) -> void {}

              function run() -> void {
                accepts(Concrete { value: "x" })
              }
            "#,
    );
    let expression_sources =
        ExpressionSourceMap::build(&parsed_sources).expect("expression source map should build");

    let error = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &crate::PublicationDbMetadataIndex::default(),
        None,
    )
    .expect_err("Concrete argument should not be assignable to bare interface parameter");

    let message = error.message();
    assert!(
        message.contains("argument"),
        "expected an argument assignability diagnostic, got: {message}"
    );
}

fn test_artifact_type_kind(
    descriptor: &TypeDescriptorIr,
    symbolic_types: &BTreeMap<(String, String), String>,
    is_alias: bool,
) -> Result<SourceTypeKind, String> {
    let symbolic_types = ArtifactSymbolicTypeIndex {
        by_symbol: symbolic_types.clone(),
        ..ArtifactSymbolicTypeIndex::default()
    };
    artifact_type_kind(
        descriptor,
        &symbolic_types,
        "example.pkg",
        &PackageTypeSymbolIndex::default(),
        "types",
        "PublicType",
        is_alias,
    )
}

#[test]
fn artifact_descriptors_preserve_nested_records_arrays_aliases_and_literal_unions() {
    let descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([
            (
                "items".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "label".to_string(),
                            TypeRefIr::builtin("string"),
                        )]),
                    }],
                },
            ),
            (
                "state".to_string(),
                TypeRefIr::Union {
                    items: vec![
                        TypeRefIr::Literal {
                            value: LiteralIr::String {
                                value: "ready".to_string(),
                            },
                        },
                        TypeRefIr::Literal {
                            value: LiteralIr::String {
                                value: "done".to_string(),
                            },
                        },
                    ],
                },
            ),
            (
                "format".to_string(),
                TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::Union {
                        items: vec![
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "chat".to_string(),
                                },
                            },
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "responses".to_string(),
                                },
                            },
                        ],
                    }),
                },
            ),
            (
                "header".to_string(),
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
                        },
                        symbol_path: "std.http.HttpHeader".to_string(),
                        abi_expectation: None,
                    },
                },
            ),
        ]),
    };
    let SourceTypeKind::Record {
        fields,
        canonical_fields: Some(canonical_fields),
    } = test_artifact_type_kind(&descriptor, &BTreeMap::new(), false)
        .expect("descriptor should be self-contained")
    else {
        panic!("record descriptor should remain a record")
    };
    assert_eq!(fields["items"], "Array<{ label: string }>");
    assert_eq!(fields["state"], "\"ready\" | \"done\"");
    assert!(matches!(
        &canonical_fields["format"],
        TypeRefIr::Nullable { inner }
            if matches!(inner.as_ref(), TypeRefIr::Union { items } if items.len() == 2)
    ));
    assert_eq!(fields["header"], "std.http.HttpHeader");

    let alias = test_artifact_type_kind(
        &TypeDescriptorIr::Alias {
            target: TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            },
        },
        &BTreeMap::new(),
        true,
    )
    .expect("alias descriptor should be self-contained");
    assert!(matches!(
        alias,
        SourceTypeKind::Alias { target, .. } if target == "Array<string>"
    ));

    let representation = test_artifact_type_kind(
        &TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("string"),
        },
        &BTreeMap::new(),
        false,
    )
    .expect("a nominal representation keeps its declaration kind");
    assert!(matches!(
        representation,
        SourceTypeKind::Representation { target, .. } if target == "string"
    ));

    let callback = test_artifact_type_kind(
        &TypeDescriptorIr::Alias {
            target: TypeRefIr::Function {
                params: vec![FunctionTypeParamIr {
                    name: "status".to_string(),
                    ty: TypeRefIr::Union {
                        items: vec![
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "running".to_string(),
                                },
                            },
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "completed".to_string(),
                                },
                            },
                        ],
                    },
                }],
                return_type: Box::new(TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                }),
            },
        },
        &BTreeMap::new(),
        true,
    )
    .expect("callback alias descriptor should stay exact");
    assert!(matches!(
        callback,
        SourceTypeKind::Alias {
            canonical_target: Some(TypeRefIr::Function { params, return_type }),
            ..
        } if matches!(
            params.as_slice(),
            [FunctionTypeParamIr {
                ty: TypeRefIr::Union { items },
                ..
            }] if items.len() == 2
        ) && matches!(
            return_type.as_ref(),
            TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1
        )
    ));
}

#[test]
fn aliases_expand_exactly_through_callbacks_and_nested_structural_types() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    crate::prelude_registry::initialize_prelude_registry(
        &skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
            .expect("platform sources should load"),
    )
    .expect("prelude registry should initialize");

    let (_parsed, model) = type_resolution(
        r#"
              type Payload {
                value: string
              }

              alias Status = "running" | "completed"
              alias Payloads = Array<Payload?>
              alias Handler = fn(status: Status) -> Payloads
            "#,
    );

    let handler = model
        .resolve_type_text("Handler", &context())
        .expect("callback alias should resolve to its exact RHS");
    let TypeRefIr::Function {
        params,
        return_type,
    } = handler.ir
    else {
        panic!("Handler must expand to a callback type");
    };
    assert!(matches!(
        params.as_slice(),
        [FunctionTypeParamIr {
            name,
            ty: TypeRefIr::Union { items },
        }] if name == "status"
            && items.len() == 2
            && items.iter().all(|item| matches!(
                item,
                TypeRefIr::Literal {
                    value: LiteralIr::String { .. }
                }
            ))
    ));
    assert!(matches!(
        return_type.as_ref(),
        TypeRefIr::Builtin { name, args }
            if name == "Array"
                && matches!(
                    args.as_slice(),
                    [TypeRefIr::Nullable { inner }]
                        if matches!(inner.as_ref(), TypeRefIr::LocalType { .. })
                )
    ));

    let missing = type_resolution("alias MissingAlias = Missing");
    let error = missing
        .1
        .resolve_type_text("MissingAlias", &context())
        .expect_err("an alias with an unresolved RHS must fail closed");
    assert!(error.contains("unresolved type `Missing`"));
}

#[test]
fn artifact_descriptors_reject_non_self_describing_local_indices() {
    let error = test_artifact_type_kind(
        &TypeDescriptorIr::Alias {
            target: TypeRefIr::LocalType { type_index: 7 },
        },
        &BTreeMap::new(),
        true,
    )
    .expect_err("ambient FileIR lookup must not be used");
    assert!(error.contains("not self-describing"));
}

#[test]
fn artifact_descriptors_resolve_only_exported_symbolic_type_closure() {
    let symbol = ServiceSymbolRef {
        module_path: "types".to_string(),
        symbol: "LlmContentPart".to_string(),
    };
    let descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "content".to_string(),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::ServiceSymbol {
                    symbol: symbol.clone(),
                }],
            },
        )]),
    };
    let symbolic_types = BTreeMap::from([(
        (symbol.module_path.clone(), symbol.symbol.clone()),
        "LlmContentPart".to_string(),
    )]);
    let SourceTypeKind::Record { fields, .. } =
        test_artifact_type_kind(&descriptor, &symbolic_types, false)
            .expect("public symbolic type should reconstruct")
    else {
        panic!("record descriptor should remain a record")
    };
    assert_eq!(fields["content"], "Array<LlmContentPart>");

    let error = test_artifact_type_kind(&descriptor, &BTreeMap::new(), false)
        .expect_err("a private or missing symbolic type must fail closed");
    assert!(error.contains("identity-validated selected artifact type"));

    let db_error = test_artifact_type_kind(
        &TypeDescriptorIr::Alias {
            target: TypeRefIr::DbObjectSymbol { symbol },
        },
        &symbolic_types,
        true,
    )
    .expect_err("db object symbols are not package-public type facts");
    assert!(db_error.contains("no package type semantics"));
}

#[test]
fn package_record_field_qualification_uses_the_exact_dependency_root() {
    assert_eq!(
        qualify_package_type_text(
            "chatgptPlan.OauthError?",
            "llmProviders",
            &BTreeSet::from(["chatgptPlan.OauthError".to_string()]),
        ),
        "llmProviders.chatgptPlan.OauthError?"
    );
    assert_eq!(
        qualify_package_type_text(
            "chatgptPlan.OauthError?",
            "llmProviders/",
            &BTreeSet::from(["chatgptPlan.OauthError".to_string()]),
        ),
        "llmProviders/chatgptPlan.OauthError?"
    );
}

#[test]
fn artifact_exported_interface_facts_preserve_classification_and_methods() {
    use skiff_artifact_model::{
        PackageBuildId, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, TypeExport,
    };

    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    crate::prelude_registry::initialize_prelude_registry(
        &skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
            .expect("platform sources should load"),
    )
    .expect("prelude registry should initialize");

    let file = skiff_artifact_model::FileIrRef {
        file_ir_identity: "file-ir".to_string(),
        artifact_path: Some("llm.json".to_string()),
        module_path: "llm".to_string(),
        source_ast_hash: Some("source".to_string()),
    };
    let method = InterfaceMethodSignature {
        name: "complete".to_string(),
        type_params: Vec::new(),
        params: vec![
            FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::TypeParam {
                    name: "Self".to_string(),
                },
            },
            FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::Nullable {
                        inner: Box::new(TypeRefIr::LocalType { type_index: 7 }),
                    }],
                },
            },
        ],
        return_type: TypeRefIr::Union {
            items: vec![
                TypeRefIr::LocalType { type_index: 7 },
                TypeRefIr::builtin("null"),
            ],
        },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    };
    let mut linked_method = method.clone();
    linked_method.params[1].ty = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "llm-api".to_string(),
                    },
                    symbol_path: "tools.ToolDeclaration".to_string(),
                    abi_expectation: None,
                },
            }),
        }],
    };
    linked_method.return_type = TypeRefIr::Union {
        items: vec![
            TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "llm-api".to_string(),
                    },
                    symbol_path: "tools.ToolDeclaration".to_string(),
                    abi_expectation: None,
                },
            },
            TypeRefIr::builtin("null"),
        ],
    };
    let descriptor = TypeDescriptorIr::Interface;
    let tool_descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
    };
    let role_descriptor = TypeDescriptorIr::Alias {
        target: TypeRefIr::Union {
            items: vec![
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "user".to_string(),
                    },
                },
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "assistant".to_string(),
                    },
                },
            ],
        },
    };
    let message_descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "role".to_string(),
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "llm".to_string(),
                    symbol: "LlmRole".to_string(),
                },
            },
        )]),
    };
    let artifact = PackageArtifact {
        schema_version: "skiff-package-artifact-v9".to_string(),
        package_id: "llm-api".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build"),
        files: vec![file.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            public_symbols: BTreeMap::from([
                (
                    "types.LlmClient".to_string(),
                    PackageLocalAbiSymbol::Type {
                        local_type_id: "type:types.LlmClient".to_string(),
                        descriptor: descriptor.clone(),
                        is_alias: false,
                        is_interface: true,
                        type_params: Vec::new(),
                        interface_methods: vec![method.clone()],
                    },
                ),
                (
                    "tools.ToolDeclaration".to_string(),
                    PackageLocalAbiSymbol::Type {
                        local_type_id: "type:tools.ToolDeclaration".to_string(),
                        descriptor: tool_descriptor.clone(),
                        is_alias: false,
                        is_interface: false,
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
                (
                    "LlmRole".to_string(),
                    PackageLocalAbiSymbol::Type {
                        local_type_id: "type:LlmRole".to_string(),
                        descriptor: role_descriptor.clone(),
                        is_alias: true,
                        is_interface: false,
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
                (
                    "LlmMessage".to_string(),
                    PackageLocalAbiSymbol::Type {
                        local_type_id: "type:LlmMessage".to_string(),
                        descriptor: message_descriptor.clone(),
                        is_alias: false,
                        is_interface: false,
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
            ]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: skiff_artifact_model::PackageSchemaIndexRef {
            package_id: "llm-api".to_string(),
            package_schema_index_identity: "index".into(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::from([
                (
                    "types.LlmClient".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 0,
                        symbol: "LlmClient".to_string(),
                        is_interface: true,
                        descriptor: Some(descriptor),
                        type_params: Vec::new(),
                        interface_methods: vec![linked_method],
                    },
                ),
                (
                    "tools.ToolDeclaration".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 7,
                        symbol: "ToolDeclaration".to_string(),
                        is_interface: false,
                        descriptor: Some(tool_descriptor),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
                (
                    "LlmRole".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 8,
                        symbol: "LlmRole".to_string(),
                        is_interface: false,
                        descriptor: Some(role_descriptor),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
                (
                    "LlmMessage".to_string(),
                    TypeExport {
                        file,
                        type_index: 9,
                        symbol: "LlmMessage".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "role".to_string(),
                                TypeRefIr::PackageSymbol {
                                    symbol: PackageSymbolRef {
                                        package: PackageRefIr::PackageId {
                                            package_id: "llm-api".to_string(),
                                        },
                                        symbol_path: "LlmRole".to_string(),
                                        abi_expectation: None,
                                    },
                                },
                            )]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                ),
            ]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    let mut package_types = BTreeMap::new();
    let mut package_interfaces = BTreeMap::new();
    index_artifact_package_types(
        &artifact,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut package_types,
        &mut package_interfaces,
        &mut BTreeMap::new(),
    )
    .expect("identity-verified artifact ABI facts should index");

    let interface = package_interfaces
        .get(&PackageSymbolKey {
            dependency_ref: "llm-api".to_string(),
            symbol_path: "LlmClient".to_string(),
        })
        .expect("exported interface classification should survive publication");
    assert_eq!(interface.methods.len(), 1);
    assert_eq!(interface.methods[0].name, "complete");
    assert!(matches!(
        interface.methods[0].return_type,
        TypeRefIr::Union { .. }
    ));
    assert_eq!(interface.methods[0].params[0].name, "self");
    assert_eq!(
        interface.methods[0].params[0].ty,
        TypeRefIr::builtin("Self")
    );

    let consumer_sources = parsed_sources(
        r#"
              import llmApi

              type LocalClient implements llmApi.LlmClient {}

              impl LocalClient {
                function complete(input: Array<llmApi.ToolDeclaration?>) -> llmApi.ToolDeclaration | null {
                  return null
                }
              }
            "#,
    );
    let mut dependency = PackageDependency::id("llm-api");
    dependency.alias = Some("llmApi".to_string());
    let model = TypeResolutionModel::build(
        &consumer_sources,
        &BTreeMap::from([("llmApi".to_string(), vec![String::new()])]),
        &[dependency],
        None,
        Some(std::slice::from_ref(&artifact)),
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("artifact-only package interface facts should build");
    let message = model
        .resolve_type_text("llmApi.LlmMessage", &context())
        .expect("package record should resolve");
    let role = model
        .record_field_type(&message, "role", &context())
        .expect("package record projection should recover its nominal field");
    let expected_role = model
        .resolve_type_text("llmApi.LlmRole", &context())
        .expect("package alias should resolve");
    assert_eq!(role.ir, expected_role.ir);
    assert!(model.assignable(&role, &expected_role));
    assert!(
        !model.assignable(
            &ResolvedTypeRef::with_text(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: "other.example/llm-api".to_string(),
                        },
                        symbol_path: "LlmRole".to_string(),
                        abi_expectation: None,
                    },
                },
                "otherRole.LlmRole".to_string(),
            ),
            &expected_role,
        ),
        "same-shaped type from another package owner must remain nominally distinct"
    );
    let actual = model
        .resolve_type_text("LocalClient", &context())
        .expect("local implementation type should resolve");
    let expected = model
        .resolve_type_text("llmApi.LlmClient", &context())
        .expect("imported public interface should resolve");
    let conformance = model
        .local_any_interface_conformance_for_boxing(&actual, &expected, &context())
        .expect("artifact-backed interface conformance lookup should not fail")
        .expect("declared imported interface implementation should match for boxing");
    let TypeRefIr::PackageSymbol { symbol } =
        serde_json::from_str::<TypeRefIr>(&conformance.interface.interface_abi_id)
            .expect("interface ABI identity should decode")
    else {
        panic!("imported interface must retain package identity")
    };
    assert_eq!(
        symbol.package,
        PackageRefIr::PackageId {
            package_id: "llm-api".to_string()
        }
    );
    assert_eq!(symbol.symbol_path, "types.LlmClient");

    let mut tampered_method = artifact.clone();
    tampered_method
        .implementation_links
        .types
        .get_mut("types.LlmClient")
        .unwrap()
        .interface_methods
        .clear();
    let error = index_artifact_package_types(
        &tampered_method,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("mismatched artifact interface facts must fail closed");
    assert!(error.contains("interface facts disagree"));

    let mut tampered_nested_path = artifact.clone();
    let TypeDescriptorIr::Record { fields } = tampered_nested_path
        .implementation_links
        .types
        .get_mut("LlmMessage")
        .unwrap()
        .descriptor
        .as_mut()
        .unwrap()
    else {
        panic!("message descriptor must remain a record")
    };
    let TypeRefIr::PackageSymbol { symbol } = fields.get_mut("role").unwrap() else {
        panic!("role must remain a package symbol")
    };
    symbol.symbol_path = "WrongRole".to_string();
    let error = index_artifact_package_types(
        &tampered_nested_path,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("a different nested package path must fail closed");
    assert!(error.contains("descriptor disagrees"));

    let mut tampered_nested_owner = artifact.clone();
    let TypeDescriptorIr::Record { fields } = tampered_nested_owner
        .implementation_links
        .types
        .get_mut("LlmMessage")
        .unwrap()
        .descriptor
        .as_mut()
        .unwrap()
    else {
        panic!("message descriptor must remain a record")
    };
    let TypeRefIr::PackageSymbol { symbol } = fields.get_mut("role").unwrap() else {
        panic!("role must remain a package symbol")
    };
    symbol.package = PackageRefIr::PackageId {
        package_id: "other.example/llm-api".to_string(),
    };
    let error = index_artifact_package_types(
        &tampered_nested_owner,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("a different nested package owner must fail closed");
    assert!(error.contains("descriptor disagrees"));

    let mut tampered_nested_abi = artifact.clone();
    let TypeDescriptorIr::Record { fields } = tampered_nested_abi
        .implementation_links
        .types
        .get_mut("LlmMessage")
        .unwrap()
        .descriptor
        .as_mut()
        .unwrap()
    else {
        panic!("message descriptor must remain a record")
    };
    let TypeRefIr::PackageSymbol { symbol } = fields.get_mut("role").unwrap() else {
        panic!("role must remain a package symbol")
    };
    symbol.abi_expectation = Some("wrong-abi".to_string());
    let error = index_artifact_package_types(
        &tampered_nested_abi,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("a different nested package ABI must fail closed");
    assert!(error.contains("descriptor disagrees"));

    let mut tampered_slot = artifact;
    tampered_slot
        .implementation_links
        .types
        .get_mut("types.LlmClient")
        .unwrap()
        .type_index = 7;
    let error = index_artifact_package_types(
        &tampered_slot,
        "llm-api",
        PackageDependencyView::Public,
        ArtifactPackageTypePathMode::DeclaredPublic,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("two selected types sharing one file/type slot must fail closed");
    assert!(error.contains("ambiguously identify"), "{error}");
}

#[test]
fn artifact_interface_receiver_reconstruction_fails_closed() {
    let method = |params, implicit_self| InterfaceMethodSignature {
        name: "streamChat".to_string(),
        type_params: vec!["Chunk".to_string()],
        params,
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self,
    };
    let self_param = FunctionTypeParamIr {
        name: "self".to_string(),
        ty: TypeRefIr::TypeParam {
            name: "Self".to_string(),
        },
    };

    let missing =
        reconstruct_artifact_interface_methods("llm-api", "LlmClient", &[method(Vec::new(), None)])
            .expect_err("missing receiver must fail closed");
    assert!(missing.contains("missing self: Self"));

    let wrong = reconstruct_artifact_interface_methods(
        "llm-api",
        "LlmClient",
        &[method(
            vec![FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("string"),
            }],
            None,
        )],
    )
    .expect_err("non-Self receiver must fail closed");
    assert!(wrong.contains("non-Self receiver"));

    let duplicate = reconstruct_artifact_interface_methods(
        "llm-api",
        "LlmClient",
        &[method(
            vec![self_param],
            Some(TypeRefIr::TypeParam {
                name: "Self".to_string(),
            }),
        )],
    )
    .expect_err("duplicate receiver must fail closed");
    assert!(duplicate.contains("duplicate receivers"));
}

#[test]
fn publication_type_slots_use_their_exact_owner_module() {
    let cleanup_source = CompilerSourceFile::parse(
        PathBuf::from("child_cleanup.skiff"),
        "child_cleanup".to_string(),
        false,
        false,
        r#"
              alias ChildCleanupEligibilityScope = "force" | "global" | "parent"

              type ChildCleanupConsumeResult {
                consumed: Bool
              }
            "#
        .to_string(),
        "child_cleanup.skiff",
    )
    .expect("cleanup source should parse");
    let consumer_source = CompilerSourceFile::parse(
        PathBuf::from("consumer.skiff"),
        "consumer".to_string(),
        false,
        false,
        "type Unrelated { value: String }".to_string(),
        "consumer.skiff",
    )
    .expect("consumer source should parse");
    let parsed =
        parse_publication_sources(&PathBuf::from("/test"), &[cleanup_source, consumer_source])
            .expect("multi-file publication should parse");
    let model = TypeResolutionModel::build(
        &parsed,
        &BTreeMap::new(),
        &[],
        None,
        None,
        &PublicationTypeSymbolIndex::default(),
    )
    .expect("type resolution should build");

    let consume_result = model.canonicalize_type_ref_for_module(
        "consumer",
        &TypeRefIr::PublicationType {
            module_path: "child_cleanup".to_string(),
            type_index: 0,
        },
    );
    assert_eq!(
        consume_result,
        canonical_named_symbol("child_cleanup.ChildCleanupConsumeResult")
    );
    let eligibility_alias = model
        .canonicalize_package_interface_signature_type(
            "child_cleanup",
            &skiff_artifact_model::PackageTypeRef::Local {
                local_type: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "child_cleanup".to_string(),
                        symbol: "ChildCleanupEligibilityScope".to_string(),
                    },
                },
            },
        )
        .expect("same-publication alias should canonicalize");
    assert!(matches!(
        eligibility_alias,
        skiff_artifact_model::PackageTypeRef::Local {
            local_type: TypeRefIr::Union { ref items }
        } if items.len() == 3
    ));

    let unknown = TypeRefIr::PublicationType {
        module_path: "missing".to_string(),
        type_index: 0,
    };
    assert_eq!(
        model.canonicalize_type_ref_for_module("consumer", &unknown),
        unknown,
        "an unknown owner module must not fall back to the caller module"
    );
}

#[test]
fn record_field_type_resolves_synthetic_catch_and_upsert_fields_via_union_shapes() {
    // Phase 3 pair 5: the unified core record_field_type adds the
    // CatchResult/DbUpsertResult synthetic branches that the trm private copy
    // lacked. The public model method reaches them through a Union shape
    // (type_shape_ir returns Union shapes verbatim), so this test locks the
    // newly observable behavior.
    let (_parsed, model) = type_resolution("");
    let user = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "name".to_string(),
            TypeRefIr::Builtin {
                name: "string".to_string(),
                args: Vec::new(),
            },
        )]),
    };
    let upsert = TypeRefIr::Builtin {
        name: "DbUpsertResult".to_string(),
        args: vec![user.clone()],
    };
    let catch = TypeRefIr::Builtin {
        name: "CatchResult".to_string(),
        args: vec![
            user.clone(),
            TypeRefIr::Builtin {
                name: "Exception".to_string(),
                args: vec![TypeRefIr::Builtin {
                    name: "string".to_string(),
                    args: Vec::new(),
                }],
            },
        ],
    };

    let field = |ty: &TypeRefIr, name: &str| {
        let resolved = ResolvedTypeRef::with_text(
            TypeRefIr::Union {
                items: vec![ty.clone()],
            },
            debug_text(ty),
        );
        model
            .record_field_type(&resolved, name, &context())
            .expect("synthetic field should resolve through the union shape")
            .ir
    };

    assert_eq!(
        field(&upsert, "value"),
        user,
        "DbUpsertResult.value must resolve to the value argument"
    );
    assert_eq!(
        field(&upsert, "inserted"),
        TypeRefIr::Builtin {
            name: "bool".to_string(),
            args: Vec::new(),
        },
        "DbUpsertResult.inserted must resolve to bool"
    );
    assert_eq!(
        field(&catch, "tag"),
        normalize_union(TypeRefIr::Union {
            items: vec![
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "ok".to_string(),
                    },
                },
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "err".to_string(),
                    },
                },
            ],
        }),
        "CatchResult.tag must resolve to the ok/err literal union"
    );
}
