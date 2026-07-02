use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::file_ir::{assign_file_ir_identity, FileIrUnit};
use skiff_compiler_source::{
    parsed_sources::{parse_publication_sources, ParsedCompilerSource},
    semantic::{
        DbAttachmentIndex, PublicationSemanticContext, SemanticPublication, SemanticSource,
        SourceOrigin, SourceSemanticContext,
    },
    source_graph::CompilerSourceFile,
    type_indices, ExpressionSourceMap, ExpressionTypeModel, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
    TypeResolutionModel,
};
use skiff_syntax::{
    ast::{ConstDecl, SourceFile},
    error::{CompileError, Result},
    parser::parse_source,
};

use super::{
    callable_return_types::{extend_callable_return_types_for_source, CallableReturnType},
    db_lowering::{lower_db_declarations, LoweredPublicationDbMetadataIndex},
    declaration_lowering::{local_type_field_index, lower_type_declarations},
    dependency_operation_indexes::{PackageOperationIndex, ServiceDependencyOperationIndex},
    executable_declaration_lowering::{
        lower_const_declarations, lower_executables, lowered_executable_signatures,
    },
    external_refs::{external_refs_for_file_ir_unit, required_receiver_builtin_capability_version},
    source_unit_lowering::{push_source_map_source, source_ast_hash},
    suspend_analysis::suspend_index_for_source,
};

pub struct PublicationSourceLoweringInput<'a, 'context, 'publication> {
    pub source: &'a str,
    pub role: &'a str,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_interface_methods: &'a PackageInterfaceMethodIndex,
    pub package_operations: &'a PackageOperationIndex,
    pub service_dependency_operations: &'a ServiceDependencyOperationIndex,
    pub external_type_symbols: &'a PublicationTypeSymbolIndex,
    pub service_dependency_aliases: &'a BTreeSet<String>,
    pub publication_db_metadata: &'a PublicationDbMetadataIndex,
    pub semantic_context: &'a SourceSemanticContext<'context, 'publication>,
    pub source_alias_targets: &'a BTreeMap<String, String>,
    pub type_resolution: &'a TypeResolutionModel,
    pub expression_types: Option<&'a ExpressionTypeModel>,
    pub callable_return_types: &'a BTreeMap<String, CallableReturnType>,
}

struct SourceFileLoweringContext<'a> {
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    package_interface_methods: &'a PackageInterfaceMethodIndex,
    package_operations: &'a PackageOperationIndex,
    service_dependency_operations: &'a ServiceDependencyOperationIndex,
    external_type_symbols: &'a PublicationTypeSymbolIndex,
    service_dependency_aliases: &'a BTreeSet<String>,
    publication_db_metadata: &'a PublicationDbMetadataIndex,
}

static EMPTY_PACKAGE_ALIASES: std::sync::LazyLock<BTreeMap<String, Vec<String>>> =
    std::sync::LazyLock::new(BTreeMap::new);
static EMPTY_PACKAGE_INTERFACE_METHODS: std::sync::LazyLock<PackageInterfaceMethodIndex> =
    std::sync::LazyLock::new(PackageInterfaceMethodIndex::default);
static EMPTY_PACKAGE_OPERATIONS: std::sync::LazyLock<PackageOperationIndex> =
    std::sync::LazyLock::new(PackageOperationIndex::default);
static EMPTY_SERVICE_DEPENDENCY_OPERATIONS: std::sync::LazyLock<ServiceDependencyOperationIndex> =
    std::sync::LazyLock::new(ServiceDependencyOperationIndex::default);
static EMPTY_EXTERNAL_TYPE_SYMBOLS: std::sync::LazyLock<PublicationTypeSymbolIndex> =
    std::sync::LazyLock::new(PublicationTypeSymbolIndex::default);
static EMPTY_SERVICE_DEPENDENCY_ALIASES: std::sync::LazyLock<BTreeSet<String>> =
    std::sync::LazyLock::new(BTreeSet::new);
static EMPTY_PUBLICATION_DB_METADATA: std::sync::LazyLock<PublicationDbMetadataIndex> =
    std::sync::LazyLock::new(PublicationDbMetadataIndex::default);

impl<'a> SourceFileLoweringContext<'a> {
    fn none() -> SourceFileLoweringContext<'static> {
        SourceFileLoweringContext {
            package_aliases: &EMPTY_PACKAGE_ALIASES,
            package_interface_methods: &EMPTY_PACKAGE_INTERFACE_METHODS,
            package_operations: &EMPTY_PACKAGE_OPERATIONS,
            service_dependency_operations: &EMPTY_SERVICE_DEPENDENCY_OPERATIONS,
            external_type_symbols: &EMPTY_EXTERNAL_TYPE_SYMBOLS,
            service_dependency_aliases: &EMPTY_SERVICE_DEPENDENCY_ALIASES,
            publication_db_metadata: &EMPTY_PUBLICATION_DB_METADATA,
        }
    }
}

pub fn compile_publication_source_file_ir_unit(
    input: PublicationSourceLoweringInput<'_, '_, '_>,
) -> Result<FileIrUnit> {
    validate_file_ir_unit_role(input.role)?;
    let source_ast_hash = source_ast_hash(input.source)?;
    let mut unit = lower_source_file_ir_unit(
        input.semantic_context,
        source_ast_hash,
        input.package_aliases,
        input.package_interface_methods,
        input.package_operations,
        input.service_dependency_operations,
        input.external_type_symbols,
        input.service_dependency_aliases,
        input.publication_db_metadata,
        input.source_alias_targets,
        input.type_resolution,
        input.expression_types,
        input.callable_return_types,
    )?;
    assign_file_ir_identity(&mut unit);
    Ok(unit)
}

fn compile_parsed_source_file_ir_unit_with_lowering_context(
    ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
    ctx: &SourceFileLoweringContext<'_>,
) -> Result<FileIrUnit> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    validate_file_ir_unit_role(&role)?;
    let source_file = CompilerSourceFile::from_parsed_ast(
        PathBuf::from(&source_path),
        module_path.clone(),
        false,
        source_path.ends_with(".test.skiff"),
        source.to_string(),
        ast,
    );
    let root = PathBuf::from("/");
    let parsed_sources = parse_publication_sources(&root, &[source_file]).map_err(|error| {
        CompileError::Semantic(format!(
            "single-file source parsing failed before File IR lowering:\n{error}"
        ))
    })?;
    let parsed = parsed_sources
        .iter()
        .find(|parsed| parsed.module_path() == module_path.as_str())
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "single-file source model did not include module `{module_path}`"
            ))
        })?;
    let type_resolution = single_file_type_resolution(
        &parsed_sources,
        ctx.package_aliases,
        ctx.external_type_symbols,
    )?;
    let expression_sources = ExpressionSourceMap::build(&parsed_sources).map_err(|message| {
        CompileError::Semantic(format!(
            "single-file expression source model failed:\n- {message}"
        ))
    })?;
    let expression_types =
        ExpressionTypeModel::build(&parsed_sources, &expression_sources, &type_resolution, None)
            .map_err(|error| {
                CompileError::Semantic(format!(
                    "single-file expression type model failed:\n- {}",
                    error.message()
                ))
            })?;

    let semantic_source = SemanticSource::new(
        parsed.relative_path().display().to_string(),
        &module_path,
        SourceOrigin::Service,
        parsed.ast(),
        parsed.alias_targets(),
    );
    let semantic_publication = SemanticPublication::new(vec![semantic_source]);
    let publication_semantic_context = PublicationSemanticContext::build(&semantic_publication)?;
    let source_semantic_context = publication_semantic_context.source_context(&module_path)?;
    let mut callable_return_types = BTreeMap::new();
    extend_callable_return_types_for_source(&mut callable_return_types, &module_path, parsed.ast());
    compile_publication_source_file_ir_unit(PublicationSourceLoweringInput {
        source,
        role: &role,
        package_aliases: ctx.package_aliases,
        package_interface_methods: ctx.package_interface_methods,
        package_operations: ctx.package_operations,
        service_dependency_operations: ctx.service_dependency_operations,
        external_type_symbols: ctx.external_type_symbols,
        service_dependency_aliases: ctx.service_dependency_aliases,
        publication_db_metadata: ctx.publication_db_metadata,
        semantic_context: &source_semantic_context,
        source_alias_targets: parsed.alias_targets(),
        type_resolution: &type_resolution,
        expression_types: Some(&expression_types),
        callable_return_types: &callable_return_types,
    })
}

fn single_file_type_resolution(
    parsed_sources: &[ParsedCompilerSource],
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
) -> Result<TypeResolutionModel> {
    TypeResolutionModel::build(
        parsed_sources,
        package_aliases,
        &[],
        None,
        external_type_symbols,
    )
    .map_err(|message| {
        CompileError::Semantic(format!(
            "single-file type resolution model failed:\n- {message}"
        ))
    })
}

pub fn compile_source_file_ir_unit(
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> Result<FileIrUnit> {
    let ast = parse_source(source)?;
    compile_parsed_source_file_ir_unit_with_lowering_context(
        ast,
        source,
        source_path,
        module_path,
        role,
        &SourceFileLoweringContext::none(),
    )
}

pub fn compile_parsed_source_file_ir_unit(
    ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> Result<FileIrUnit> {
    compile_parsed_source_file_ir_unit_with_lowering_context(
        ast,
        source,
        source_path,
        module_path,
        role,
        &SourceFileLoweringContext::none(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compile_parsed_source_file_ir_unit_with_publication_db_context(
    ast: SourceFile,
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
) -> Result<FileIrUnit> {
    compile_parsed_source_file_ir_unit_with_lowering_context(
        ast,
        source,
        source_path,
        module_path,
        role,
        &SourceFileLoweringContext {
            package_aliases,
            external_type_symbols,
            publication_db_metadata,
            ..SourceFileLoweringContext::none()
        },
    )
}

fn validate_file_ir_unit_role(role: &str) -> Result<()> {
    if role.trim().is_empty() {
        return Err(CompileError::Semantic(
            "file IR unit role cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn lower_source_file_ir_unit(
    semantic_context: &SourceSemanticContext<'_, '_>,
    source_ast_hash: String,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
) -> Result<FileIrUnit> {
    let source = semantic_context.source;
    let ast = source.ast;
    let source_path = source.source_path.as_ref().to_string();
    let module_path = source.module_path;
    let executable_index = semantic_context.executable_index;
    validate_supported_top_level(ast)?;

    let type_indices = type_indices(ast);
    let const_indices = const_indices(&ast.consts);
    let executable_indices = executable_index.indices();
    let mut callable_return_types = callable_return_types.clone();
    extend_callable_return_types_for_source(&mut callable_return_types, module_path, ast);
    let db_attachments = DbAttachmentIndex::build(module_path, ast)?;
    let local_db_objects = LocalDbObjectIndex::from_attachments(&db_attachments);
    let lowered_publication_db_metadata = LoweredPublicationDbMetadataIndex::from_source_index(
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
    )?;
    let executable_signatures = lowered_executable_signatures(
        &ast.functions,
        &ast.impls,
        executable_index,
        &type_indices,
        &local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
    )?;
    let mut unit = FileIrUnit::empty(module_path.to_string(), source_ast_hash.clone());
    push_source_map_source(&mut unit, source_path, module_path, source_ast_hash);

    let mut next_span_id = 0u64;
    lower_type_declarations(
        &ast.types,
        &ast.aliases,
        &ast.interfaces,
        &type_indices,
        module_path,
        &local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        &mut unit,
        &mut next_span_id,
    )?;
    let local_type_fields = local_type_field_index(&unit);
    let db_metadata = lower_db_declarations(
        &db_attachments,
        &type_indices,
        package_aliases,
        external_type_symbols,
        &local_db_objects,
        publication_db_metadata,
        source_alias_targets,
        &mut unit,
        &mut next_span_id,
    )?;
    lower_const_declarations(
        &ast.consts,
        &const_indices,
        &executable_indices,
        &db_metadata,
        publication_db_metadata,
        &lowered_publication_db_metadata,
        &type_indices,
        package_aliases,
        package_interface_methods,
        package_operations,
        service_dependency_operations,
        external_type_symbols,
        service_dependency_aliases,
        module_path,
        &local_db_objects,
        semantic_context.interface_semantics,
        source_alias_targets,
        type_resolution,
        expression_types,
        &callable_return_types,
        &local_type_fields,
        &executable_signatures,
        &mut unit,
        &mut next_span_id,
    )?;
    let suspend_index = suspend_index_for_source(
        ast,
        module_path,
        package_aliases,
        service_dependency_aliases,
        expression_types,
    );
    lower_executables(
        &ast.functions,
        &ast.impls,
        &db_metadata,
        publication_db_metadata,
        &lowered_publication_db_metadata,
        &suspend_index,
        executable_index,
        &const_indices,
        &type_indices,
        external_type_symbols,
        service_dependency_aliases,
        module_path,
        package_aliases,
        package_interface_methods,
        package_operations,
        service_dependency_operations,
        &local_db_objects,
        semantic_context.interface_semantics,
        source_alias_targets,
        type_resolution,
        expression_types,
        &callable_return_types,
        &local_type_fields,
        &executable_signatures,
        &mut unit,
        &mut next_span_id,
    )?;
    unit.required_receiver_builtin_capability_version =
        required_receiver_builtin_capability_version(&unit);
    unit.external_refs = external_refs_for_file_ir_unit(&unit);
    Ok(unit)
}

fn validate_supported_top_level(ast: &SourceFile) -> Result<()> {
    if ast.provider_capability.is_some() {
        return Err(unsupported(
            "legacy provider capability declarations are not supported by the File IR unit emitter",
        ));
    }
    let provider_functions = ast
        .functions
        .iter()
        .filter(|function| function.is_provider)
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    if !provider_functions.is_empty() {
        return Err(unsupported(format!(
            "legacy provider function declarations are not supported by the File IR unit emitter: {}",
            provider_functions.join(", ")
        )));
    }
    let provider_methods = ast
        .impls
        .iter()
        .flat_map(|implementation| {
            implementation
                .method_bodies
                .iter()
                .filter(|method| method.is_provider)
                .map(|method| format!("{}.{}", implementation.target, method.name))
        })
        .collect::<Vec<_>>();
    if !provider_methods.is_empty() {
        return Err(unsupported(format!(
            "legacy provider impl method declarations are not supported by the File IR unit emitter: {}",
            provider_methods.join(", ")
        )));
    }
    if !ast.function_signatures.is_empty() {
        let names = ast
            .function_signatures
            .iter()
            .map(|signature| signature.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(unsupported(format!(
            "bodyless or unparsable function signatures are not supported by the File IR unit emitter: {names}"
        )));
    }
    if !ast.tests.is_empty() || ast.test_default_run.is_some() {
        return Err(unsupported(
            "test declarations are not supported by the File IR unit emitter",
        ));
    }
    Ok(())
}

fn const_indices(constants: &[ConstDecl]) -> BTreeMap<String, u32> {
    constants
        .iter()
        .enumerate()
        .map(|(index, constant)| (constant.name.clone(), index as u32))
        .collect()
}

fn unsupported(message: impl Into<String>) -> CompileError {
    CompileError::Semantic(message.into())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use crate::{
        file_ir::{BoxSourceIr, CallTargetIr, ExecutableIr, ExprIr, PackageRefIr, TypeRefIr},
        source_unit_lowering::symbol,
    };
    use skiff_artifact_model::ReceiverCallAbi;
    use skiff_compiler_source::{
        api::PublicTypeKind, build_from_parsed_sources, parsed_sources::parse_publication_sources,
        source_graph::CompilerSourceFile, CompileParsedPublicationSourcesInput, PackageDependency,
        PublicationApiEntry, PublicationApiSpec, PublicationCompilePolicy,
        ResolvedServiceDependencies, SourceCompilePackageFacts,
    };

    use super::*;

    const MODULE: &str = "internal.any_lowering";
    const PACKAGE_ID: &str = "example.com/reader";
    const PACKAGE_MODULE: &str = "pkg.reader";

    fn any_interface_source() -> &'static str {
        r#"
          interface Provider {
            function name(self: Self) -> string
          }

          type HostProvider implements Provider {
            label: string,
          }

          impl HostProvider {
            function name() -> string {
              return self.label
            }
          }

          function make_box() -> void {
            let provider = HostProvider { label: "host" } as Provider
          }

          function call_box() -> string {
            let provider = HostProvider { label: "host" } as Provider
            return provider.name()
          }
	        "#
    }

    fn package_reader_source() -> &'static str {
        r#"
	          interface Reader<T> {
	            function read(self: Self, fallback: T) -> T
	          }
	        "#
    }

    fn package_interface_box_source() -> &'static str {
        r#"
	          type Host implements pkg.Reader<string> {
	            value: string,
	          }

	          impl Host {
	            function read(fallback: string) -> string {
	              return fallback
	            }
	          }

	          function make_package_box() -> void {
	            let reader = Host { value: "host" } as pkg.Reader<string>
	          }
	        "#
    }

    fn any_interface_signature_source() -> &'static str {
        r#"
          interface Provider {
            function name(self: Self) -> string
          }

          function accept(provider: any Provider) -> void {
          }
        "#
    }

    fn package_any_interface_signature_source() -> &'static str {
        r#"
          function accept_package(reader: any pkg.Reader<string>) -> void {
          }
        "#
    }

    fn lowered_unit(source_text: &str) -> FileIrUnit {
        let root = PathBuf::from("/test");
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/any_lowering.skiff"),
            MODULE.to_string(),
            false,
            false,
            source_text.to_string(),
            "internal/any_lowering.skiff",
        )
        .expect("test source should parse");
        let production_sources = vec![source];
        let parsed_sources = parse_publication_sources(&root, &production_sources)
            .expect("test source facts should build");
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let model = build_from_parsed_sources(CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            service_dependencies: ResolvedServiceDependencies::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package {
                package_id: "example.com/any-lowering",
            },
        })
        .expect("source model should build");
        let lowered = crate::lower(&model).expect("publication should lower");
        lowered
            .file_ir_units()
            .first()
            .expect("one file IR unit should be emitted")
            .clone()
    }

    fn lowered_units(sources: Vec<(&str, &str, &str)>) -> Vec<FileIrUnit> {
        let root = PathBuf::from("/test");
        let production_sources = sources
            .into_iter()
            .map(|(relative_path, module_path, source_text)| {
                CompilerSourceFile::parse(
                    PathBuf::from(relative_path),
                    module_path.to_string(),
                    false,
                    false,
                    source_text.to_string(),
                    relative_path,
                )
                .expect("test source should parse")
            })
            .collect::<Vec<_>>();
        let parsed_sources = parse_publication_sources(&root, &production_sources)
            .expect("test source facts should build");
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let model = build_from_parsed_sources(CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            service_dependencies: ResolvedServiceDependencies::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package {
                package_id: "example.com/publication-local-refs",
            },
        })
        .expect("source model should build");
        crate::lower(&model)
            .expect("publication should lower")
            .file_ir_units()
            .to_vec()
    }

    fn lowered_unit_with_package_facts(source_text: &str) -> FileIrUnit {
        let package_root = PathBuf::from("/package");
        let package_source = CompilerSourceFile::parse(
            PathBuf::from("pkg/reader.skiff"),
            PACKAGE_MODULE.to_string(),
            false,
            false,
            package_reader_source().to_string(),
            "pkg/reader.skiff",
        )
        .expect("package source should parse");
        let package_api = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "Reader",
            PACKAGE_MODULE,
            "Reader",
        )]);
        let package_production_sources = vec![package_source];
        let package_parsed_sources =
            parse_publication_sources(&package_root, &package_production_sources)
                .expect("package source facts should build");
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let package_model = build_from_parsed_sources(CompileParsedPublicationSourcesInput {
            parsed_sources: package_parsed_sources,
            production_sources: package_production_sources,
            diagnostic_root: &package_root,
            publication_api: Some(&package_api),
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            service_dependencies: ResolvedServiceDependencies::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package {
                package_id: PACKAGE_ID,
            },
        })
        .expect("package source model should build");
        assert_eq!(
            package_model
                .export_bindings()
                .public_schema_types()
                .get("Reader")
                .expect("Reader should be exported")
                .kind,
            PublicTypeKind::Interface
        );
        let package_lowered = crate::lower(&package_model).expect("package should lower");
        let package_file_ir_units = package_lowered.file_ir_units().to_vec();
        let package_facts = vec![SourceCompilePackageFacts::new(
            PACKAGE_ID,
            "1.0.0",
            Vec::new(),
            &package_model,
            &package_file_ir_units,
        )];

        let root = PathBuf::from("/test");
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/any_lowering.skiff"),
            MODULE.to_string(),
            false,
            false,
            source_text.to_string(),
            "internal/any_lowering.skiff",
        )
        .expect("test source should parse");
        let production_sources = vec![source];
        let parsed_sources = parse_publication_sources(&root, &production_sources)
            .expect("test source facts should build");
        let package_aliases = BTreeMap::from([("pkg".to_string(), vec![String::new()])]);
        let mut dependency = PackageDependency::id(PACKAGE_ID);
        dependency.alias = Some("pkg".to_string());
        let package_dependencies = vec![dependency];
        let model = build_from_parsed_sources(CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: Some(&package_facts),
            service_dependencies: ResolvedServiceDependencies::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package {
                package_id: "example.com/any-lowering",
            },
        })
        .expect("source model with package facts should build");
        let lowered = crate::lower(&model).expect("publication should lower");
        lowered
            .file_ir_units()
            .first()
            .expect("one file IR unit should be emitted")
            .clone()
    }

    fn executable<'a>(unit: &'a FileIrUnit, name: &str) -> &'a ExecutableIr {
        let expected_symbol = symbol(MODULE, name);
        unit.executables
            .iter()
            .find(|executable| executable.symbol == expected_symbol)
            .unwrap_or_else(|| panic!("missing executable `{expected_symbol}`"))
    }

    fn only_interface_box(executable: &ExecutableIr) -> &ExprIr {
        let boxes = executable
            .body
            .expressions
            .iter()
            .filter(|expr| matches!(expr, ExprIr::InterfaceBox { .. }))
            .collect::<Vec<_>>();
        assert_eq!(
            boxes.len(),
            1,
            "expected exactly one InterfaceBox in {}",
            executable.symbol
        );
        boxes[0]
    }

    #[test]
    fn lowers_cross_module_publication_refs_to_direct_addresses() {
        let units = lowered_units(vec![
            (
                "internal/worker.skiff",
                "internal.worker",
                r#"
                  type DrainResult {
                    value: string,
                  }

                  function drain() -> DrainResult {
                    return DrainResult { value: "ok" }
                  }
                "#,
            ),
            (
                "internal/runner.skiff",
                "internal.runner",
                r#"
                  function run() -> root.internal.worker.DrainResult {
                    return root.internal.worker.drain()
                  }
                "#,
            ),
        ]);
        let worker = units
            .iter()
            .find(|unit| unit.module_path == "internal.worker")
            .expect("worker unit should be emitted");
        let runner = units
            .iter()
            .find(|unit| unit.module_path == "internal.runner")
            .expect("runner unit should be emitted");
        let result_type_index = worker
            .declarations
            .types
            .get("DrainResult")
            .expect("DrainResult declaration should exist")
            .type_index;
        let drain_executable_index = worker
            .declarations
            .executables
            .get("drain")
            .expect("drain declaration should exist")
            .executable_index;
        let run = runner
            .executables
            .iter()
            .find(|executable| executable.symbol == "internal.runner.run")
            .expect("run executable should exist");

        assert!(matches!(
            &run.return_type,
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } if module_path == "internal.worker" && *type_index == result_type_index
        ));
        assert!(
            run.body.expressions.iter().any(|expr| matches!(
                expr,
                ExprIr::Call {
                    call
                } if matches!(
                    &call.target,
                    CallTargetIr::PublicationExecutable {
                        module_path,
                        executable_index,
                    } if module_path == "internal.worker"
                        && *executable_index == drain_executable_index
                )
            )),
            "cross-module function call should lower to PublicationExecutable"
        );
        assert!(
            runner.external_refs.service_symbols.is_empty(),
            "publication-local refs must not remain in external_refs: {:?}",
            runner.external_refs.service_symbols
        );
        assert!(runner.link_targets.types.is_empty());
        assert!(runner.link_targets.executables.is_empty());
        assert!(worker.link_targets.types.is_empty());
        assert!(worker.link_targets.executables.is_empty());
    }

    #[test]
    fn lowers_interface_box_to_local_method_table() {
        let unit = lowered_unit(any_interface_source());
        let make_box = executable(&unit, "make_box");
        let impl_executable_index = unit
            .declarations
            .executables
            .get("HostProvider.name")
            .expect("impl method declaration should exist")
            .executable_index;

        let ExprIr::InterfaceBox {
            interface,
            source:
                BoxSourceIr::Local {
                    concrete_type,
                    method_table,
                },
            ..
        } = only_interface_box(make_box)
        else {
            panic!("expected InterfaceBox Local source");
        };

        assert_eq!(&method_table.interface, interface);
        assert_eq!(&method_table.concrete_type, concrete_type);
        assert!(
            matches!(concrete_type, TypeRefIr::LocalType { .. }),
            "box source concrete type should be a local nominal type"
        );
        assert_eq!(method_table.slots.len(), 1);
        let slot = &method_table.slots[0];
        assert_eq!(slot.slot, 0);
        assert_eq!(slot.method_name, "name");
        assert_eq!(
            slot.target.executable_index, impl_executable_index,
            "method table slot must target the local impl method executable"
        );
        assert_eq!(
            slot.target.receiver_call_abi,
            ReceiverCallAbi::ExplicitSelfFirst
        );
        assert_eq!(slot.signature.params.len(), 1);
        assert_eq!(slot.signature.params[0].name, "self");
        assert_eq!(slot.signature.return_type, TypeRefIr::native("string"));
        assert!(!slot.method_abi_id.is_empty());
    }

    #[test]
    fn lowers_package_interface_box_to_local_method_table() {
        let unit = lowered_unit_with_package_facts(package_interface_box_source());
        let make_box = executable(&unit, "make_package_box");
        let impl_executable_index = unit
            .declarations
            .executables
            .get("Host.read")
            .expect("impl method declaration should exist")
            .executable_index;

        let ExprIr::InterfaceBox {
            interface,
            source:
                BoxSourceIr::Local {
                    concrete_type,
                    method_table,
                },
            ..
        } = only_interface_box(make_box)
        else {
            panic!("expected package InterfaceBox Local source");
        };

        let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .expect("interface ABI id should decode");
        let TypeRefIr::PackageSymbol { symbol } = interface_ty else {
            panic!("package interface box should use PackageSymbol ABI identity");
        };
        assert_eq!(symbol.symbol_path, "Reader");
        assert!(matches!(
            symbol.package,
            PackageRefIr::PackageId { ref package_id } if package_id == PACKAGE_ID
        ));
        assert_eq!(
            interface.canonical_type_args,
            vec![TypeRefIr::native("string")]
        );
        assert_eq!(&method_table.interface, interface);
        assert_eq!(&method_table.concrete_type, concrete_type);
        assert_eq!(method_table.slots.len(), 1);
        let slot = &method_table.slots[0];
        assert_eq!(slot.slot, 0);
        assert_eq!(slot.method_name, "read");
        assert_eq!(slot.target.executable_index, impl_executable_index);
        assert_eq!(
            slot.target.receiver_call_abi,
            ReceiverCallAbi::ExplicitSelfFirst
        );
        assert_eq!(slot.signature.params.len(), 2);
        assert_eq!(slot.signature.params[1].name, "fallback");
        assert_eq!(slot.signature.params[1].ty, TypeRefIr::native("string"));
        assert_eq!(slot.signature.return_type, TypeRefIr::native("string"));
        assert!(!slot.method_abi_id.is_empty());
    }

    #[test]
    fn lowers_any_interface_function_param_to_any_interface_type_ref() {
        let unit = lowered_unit(any_interface_signature_source());
        let accept = executable(&unit, "accept");
        let TypeRefIr::AnyInterface { interface } = &accept.params[0].ty else {
            panic!("any Provider parameter should lower to TypeRefIr::AnyInterface");
        };
        let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .expect("interface ABI id should decode");
        let TypeRefIr::LocalType { type_index } = interface_ty else {
            panic!("local interface selector should use direct local type identity");
        };
        assert_eq!(
            type_index,
            unit.declarations
                .types
                .get("Provider")
                .expect("Provider declaration should exist")
                .type_index
        );
        assert!(interface.canonical_type_args.is_empty());
    }

    #[test]
    fn lowers_package_any_interface_function_param_to_package_symbol_selector() {
        let unit = lowered_unit_with_package_facts(package_any_interface_signature_source());
        let accept = executable(&unit, "accept_package");
        let TypeRefIr::AnyInterface { interface } = &accept.params[0].ty else {
            panic!("any pkg.Reader<string> parameter should lower to TypeRefIr::AnyInterface");
        };
        let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .expect("interface ABI id should decode");
        let TypeRefIr::PackageSymbol { symbol } = interface_ty else {
            panic!("package interface selector should use PackageSymbol ABI identity");
        };
        assert_eq!(symbol.symbol_path, "Reader");
        assert!(matches!(
            symbol.package,
            PackageRefIr::Dependency { ref dependency_ref } if dependency_ref == "pkg"
        ));
        assert_eq!(
            interface.canonical_type_args,
            vec![TypeRefIr::native("string")]
        );
    }

    #[test]
    fn lowers_any_interface_receiver_call_to_interface_method_target() {
        let unit = lowered_unit(any_interface_source());
        let call_box = executable(&unit, "call_box");
        let boxed = only_interface_box(call_box);
        let ExprIr::InterfaceBox {
            interface,
            source: BoxSourceIr::Local { method_table, .. },
            ..
        } = boxed
        else {
            panic!("expected local InterfaceBox before receiver call");
        };
        let slot = &method_table.slots[0];

        let call = call_box
            .body
            .expressions
            .iter()
            .find_map(|expr| {
                let ExprIr::Call { call } = expr else {
                    return None;
                };
                matches!(call.target, CallTargetIr::InterfaceMethod { .. }).then_some(call)
            })
            .expect("provider.name() should lower to InterfaceMethod call");

        let CallTargetIr::InterfaceMethod {
            interface: call_interface,
            method_abi_id,
            slot: call_slot,
        } = &call.target
        else {
            unreachable!("find_map only returns InterfaceMethod calls");
        };
        assert_eq!(call_interface, interface);
        assert_eq!(method_abi_id, &slot.method_abi_id);
        assert_eq!(*call_slot, slot.slot);
        assert_eq!(call.args.len(), 1, "receiver should be the first arg");
        let receiver_arg = &call_box.body.expressions[call.args[0].expression as usize];
        assert!(
            matches!(receiver_arg, ExprIr::LoadSlot { .. }),
            "receiver arg should load the boxed local binding"
        );
    }
}
