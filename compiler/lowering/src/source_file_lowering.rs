use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::file_ir::{assign_file_ir_identity, FileIrUnit};
use skiff_artifact_model::{
    ActorAbiInput, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorImplementationIdentity, ActorPublicMethodIr, FunctionTypeParamIr,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_compiler_source::{
    parsed_sources::{parse_publication_sources, ParsedCompilerSource},
    publication_db_metadata_index,
    semantic::{
        impl_method_declaration_name, DbAttachmentIndex, PublicationSemanticContext,
        SemanticPublication, SemanticSource, SourceSemanticContext,
    },
    source_graph::CompilerSourceFile,
    type_indices, ExpressionSourceMap, ExpressionTypeModel, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationDbMetadataIndex, PublicationTypeSymbolIndex,
    ResolvedCallTargetFacts, SourceDependencyAnalysisInput, SourceExecutableSignatureFacts,
    SourceExecutionSemantics, SourceInterfaceSignatureFacts, SourceSymbolKey, TypeResolutionModel,
};
use skiff_syntax::{
    ast::{ConstDecl, SourceFile},
    error::{CompileError, Result},
    parser::parse_source,
};

use super::{
    callable_return_types::{extend_callable_return_types_for_source, CallableReturnType},
    db_lowering::{lower_db_declarations, LoweredPackageDbMetadataIndex},
    declaration_lowering::{local_type_field_index, lower_type_declarations},
    executable_declaration_lowering::{
        lower_const_declarations, lower_executables, lowered_executable_signatures,
    },
    external_refs::{
        rebuild_external_refs_for_file_ir_unit, required_receiver_builtin_capability_version,
    },
    service_call_lowering::LoweredServiceCalls,
    source_unit_lowering::{push_source_map_source, source_ast_hash},
    suspend_analysis::suspend_index_for_source,
};

pub(crate) struct PackageSourceLoweringInput<'a, 'context, 'publication> {
    pub source: &'a str,
    pub role: &'a str,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_interface_methods: &'a PackageInterfaceMethodIndex,
    pub resolved_call_targets: &'a ResolvedCallTargetFacts,
    pub external_type_symbols: &'a PublicationTypeSymbolIndex,
    pub publication_db_metadata: &'a PublicationDbMetadataIndex,
    pub semantic_context: &'a SourceSemanticContext<'context, 'publication>,
    pub source_alias_targets: &'a BTreeMap<String, String>,
    pub type_resolution: &'a TypeResolutionModel,
    pub expression_types: Option<&'a ExpressionTypeModel>,
    /// Full package lowering must provide the compiler-validated execution
    /// plan. Standalone helpers keep this absent and fail closed if execution
    /// syntax is encountered.
    pub execution_semantics: Option<&'a SourceExecutionSemantics>,
    pub callable_return_types: &'a BTreeMap<String, CallableReturnType>,
    pub executable_signatures: &'a SourceExecutableSignatureFacts,
    /// `None` is reserved for standalone helpers; an interface then fails
    /// closed instead of rebuilding its signature from syntax.
    pub interface_signatures: Option<&'a SourceInterfaceSignatureFacts>,
    pub service_calls: Option<&'a LoweredServiceCalls>,
}

struct SourceFileLoweringContext<'a> {
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    package_interface_methods: &'a PackageInterfaceMethodIndex,
    resolved_call_targets: &'a ResolvedCallTargetFacts,
    external_type_symbols: &'a PublicationTypeSymbolIndex,
    service_dependency_aliases: &'a BTreeSet<String>,
    publication_db_metadata: &'a PublicationDbMetadataIndex,
    service_calls: Option<&'a LoweredServiceCalls>,
}

static EMPTY_PACKAGE_ALIASES: std::sync::LazyLock<BTreeMap<String, Vec<String>>> =
    std::sync::LazyLock::new(BTreeMap::new);
static EMPTY_PACKAGE_INTERFACE_METHODS: std::sync::LazyLock<PackageInterfaceMethodIndex> =
    std::sync::LazyLock::new(PackageInterfaceMethodIndex::default);
static EMPTY_RESOLVED_CALL_TARGETS: std::sync::LazyLock<ResolvedCallTargetFacts> =
    std::sync::LazyLock::new(ResolvedCallTargetFacts::empty);
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
            resolved_call_targets: &EMPTY_RESOLVED_CALL_TARGETS,
            external_type_symbols: &EMPTY_EXTERNAL_TYPE_SYMBOLS,
            service_dependency_aliases: &EMPTY_SERVICE_DEPENDENCY_ALIASES,
            publication_db_metadata: &EMPTY_PUBLICATION_DB_METADATA,
            service_calls: None,
        }
    }
}

pub(crate) fn compile_package_source_file_ir_unit(
    input: PackageSourceLoweringInput<'_, '_, '_>,
) -> Result<FileIrUnit> {
    validate_file_ir_unit_role(input.role)?;
    let source_ast_hash = source_ast_hash(input.source)?;
    let unit = lower_source_file_ir_unit(
        input.semantic_context,
        source_ast_hash,
        input.package_aliases,
        input.package_interface_methods,
        input.resolved_call_targets,
        input.external_type_symbols,
        input.publication_db_metadata,
        input.source_alias_targets,
        input.type_resolution,
        input.expression_types,
        input.execution_semantics,
        input.callable_return_types,
        input.executable_signatures,
        input.interface_signatures,
        input.service_calls,
    )?;
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
    let mut expression_db_metadata = publication_db_metadata_index(
        parsed_sources
            .iter()
            .map(|source| (source.module_path(), source.ast())),
        ctx.package_aliases,
        ctx.external_type_symbols,
    )?;
    expression_db_metadata.extend(ctx.publication_db_metadata.clone());
    let expression_types = ExpressionTypeModel::build(
        &parsed_sources,
        &expression_sources,
        &type_resolution,
        &expression_db_metadata,
        None,
    )
    .map_err(|error| {
        CompileError::Semantic(format!(
            "single-file expression type model failed:\n- {}",
            error.message()
        ))
    })?;

    let semantic_source = SemanticSource::new(
        parsed.relative_path().display().to_string(),
        &module_path,
        parsed.ast(),
        parsed.alias_targets(),
    );
    let semantic_publication = SemanticPublication::new(vec![semantic_source]);
    let publication_semantic_context = PublicationSemanticContext::build(&semantic_publication)?;
    let source_semantic_context = publication_semantic_context.source_context(&module_path)?;
    let mut callable_return_types = BTreeMap::new();
    extend_callable_return_types_for_source(&mut callable_return_types, &module_path, parsed.ast());
    let executable_signatures = standalone_executable_signatures(
        &parsed_sources,
        &type_resolution,
        parsed.ast(),
        &module_path,
        ctx,
        &expression_types,
    )?;
    let mut unit = compile_package_source_file_ir_unit(PackageSourceLoweringInput {
        source,
        role: &role,
        package_aliases: ctx.package_aliases,
        package_interface_methods: ctx.package_interface_methods,
        resolved_call_targets: ctx.resolved_call_targets,
        external_type_symbols: ctx.external_type_symbols,
        publication_db_metadata: ctx.publication_db_metadata,
        semantic_context: &source_semantic_context,
        source_alias_targets: parsed.alias_targets(),
        type_resolution: &type_resolution,
        expression_types: Some(&expression_types),
        execution_semantics: None,
        callable_return_types: &callable_return_types,
        executable_signatures: &executable_signatures,
        interface_signatures: None,
        service_calls: ctx.service_calls,
    })?;
    finalize_actor_identities(std::slice::from_mut(&mut unit))?;
    assign_file_ir_identity(&mut unit);
    Ok(unit)
}

fn standalone_executable_signatures(
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
    ast: &SourceFile,
    module_path: &str,
    ctx: &SourceFileLoweringContext<'_>,
    expression_types: &ExpressionTypeModel,
) -> Result<SourceExecutableSignatureFacts> {
    let suspend_index = suspend_index_for_source(
        ast,
        module_path,
        ctx.package_aliases,
        ctx.service_dependency_aliases,
        Some(expression_types),
        ctx.resolved_call_targets,
    );
    let mut may_suspend = BTreeMap::new();
    for function in &ast.functions {
        may_suspend.insert(
            SourceSymbolKey::new(module_path, &function.name),
            suspend_index.function_may_suspend(&function.name),
        );
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            may_suspend.insert(
                SourceSymbolKey::new(
                    module_path,
                    impl_method_declaration_name(&implementation.target, &method.name),
                ),
                suspend_index.method_may_suspend(&implementation.target, &method.name),
            );
        }
    }
    SourceExecutableSignatureFacts::from_exact_may_suspend(
        parsed_sources,
        type_resolution,
        &SourceDependencyAnalysisInput::default(),
        &may_suspend,
    )
    .map_err(|message| {
        CompileError::Semantic(format!(
            "single-file exact executable signature model failed:\n- {message}"
        ))
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
    resolved_call_targets: &ResolvedCallTargetFacts,
    external_type_symbols: &PublicationTypeSymbolIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    execution_semantics: Option<&SourceExecutionSemantics>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    exact_executable_signatures: &SourceExecutableSignatureFacts,
    exact_interface_signatures: Option<&SourceInterfaceSignatureFacts>,
    service_calls: Option<&LoweredServiceCalls>,
) -> Result<FileIrUnit> {
    let source = semantic_context.source;
    let ast = source.ast;
    let source_path = source.source_path.as_ref().to_string();
    let module_path = source.module_path;
    let executable_index = semantic_context.executable_index;
    let empty_service_calls = LoweredServiceCalls::default();
    let service_calls = service_calls.unwrap_or(&empty_service_calls);
    validate_supported_top_level(ast)?;

    let type_indices = type_indices(ast);
    let const_indices = const_indices(&ast.consts);
    let executable_indices = executable_index.indices();
    let mut callable_return_types = callable_return_types.clone();
    extend_callable_return_types_for_source(&mut callable_return_types, module_path, ast);
    let db_attachments = DbAttachmentIndex::build(module_path, ast)?;
    let local_db_objects = LocalDbObjectIndex::from_attachments(&db_attachments);
    let lowered_publication_db_metadata = LoweredPackageDbMetadataIndex::from_source_index(
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
    )?;
    let executable_signatures = lowered_executable_signatures(
        &ast.functions,
        &ast.impls,
        executable_index,
        module_path,
        exact_executable_signatures,
    )?;
    let mut unit = FileIrUnit::empty(module_path.to_string(), source_ast_hash.clone());
    push_source_map_source(&mut unit, source_path, module_path, source_ast_hash);

    let mut next_span_id = 0u64;
    lower_type_declarations(
        &ast.types,
        &ast.aliases,
        &ast.interfaces,
        exact_interface_signatures,
        &type_indices,
        module_path,
        type_resolution,
        &local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        &mut unit,
        &mut next_span_id,
    )?;
    lower_actor_declarations(
        ast,
        module_path,
        executable_index,
        &executable_signatures,
        &type_indices,
        &local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        &mut unit,
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
        resolved_call_targets,
        external_type_symbols,
        module_path,
        &local_db_objects,
        semantic_context.interface_semantics,
        source_alias_targets,
        type_resolution,
        expression_types,
        execution_semantics,
        &callable_return_types,
        &local_type_fields,
        &executable_signatures,
        service_calls,
        &mut unit,
        &mut next_span_id,
    )?;
    lower_executables(
        &ast.functions,
        &ast.impls,
        &db_metadata,
        publication_db_metadata,
        &lowered_publication_db_metadata,
        executable_index,
        &const_indices,
        &type_indices,
        external_type_symbols,
        module_path,
        package_aliases,
        package_interface_methods,
        resolved_call_targets,
        &local_db_objects,
        semantic_context.interface_semantics,
        source_alias_targets,
        type_resolution,
        expression_types,
        execution_semantics,
        &callable_return_types,
        &local_type_fields,
        &executable_signatures,
        service_calls,
        &mut unit,
        &mut next_span_id,
    )?;
    unit.required_receiver_builtin_capability_version =
        required_receiver_builtin_capability_version(&unit);
    unit.external_refs.service_call_refs =
        service_calls.file_service_call_refs(module_path).to_vec();
    rebuild_external_refs_for_file_ir_unit(&mut unit).map_err(|error| {
        CompileError::Semantic(format!("invalid service call File IR: {error}"))
    })?;
    Ok(unit)
}

pub(super) fn finalize_actor_identities(units: &mut [FileIrUnit]) -> Result<()> {
    let actors = units
        .iter()
        .flat_map(|unit| {
            unit.actor_declarations.iter().map(|actor| {
                (
                    unit.module_path.clone(),
                    actor.abi.actor_name.clone(),
                    actor.actor_abi_identity.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    let abi_facts = actors
        .iter()
        .map(|(module, actor, abi)| ((module.clone(), actor.clone()), abi.clone()))
        .collect::<BTreeMap<_, _>>();
    for unit in units.iter_mut() {
        visit_actor_call_targets(unit, |actor, abi, implementation| {
            *abi = abi_facts
                .get(&(actor.module_path.clone(), actor.symbol.clone()))
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "Actor method target {} has no Actor declaration",
                        actor.symbol_path()
                    ))
                })?
                .clone();
            *implementation = ActorImplementationIdentity::new("pending-actor-implementation");
            Ok(())
        })?;
    }

    let mut facts = BTreeMap::new();
    for (module_path, actor_name, actor_abi_identity) in actors {
        let implementation = skiff_artifact_identity::actor_implementation_identity(
            units,
            &module_path,
            &actor_name,
        )
        .map_err(|error| CompileError::Semantic(error.to_string()))?;
        facts.insert(
            (module_path, actor_name),
            (actor_abi_identity, implementation),
        );
    }
    for unit in units.iter_mut() {
        for declaration in &mut unit.actor_declarations {
            declaration.actor_implementation_identity = facts
                .get(&(unit.module_path.clone(), declaration.abi.actor_name.clone()))
                .expect("Actor identity fact was collected from this declaration")
                .1
                .clone();
        }
        visit_actor_call_targets(unit, |actor, abi, implementation| {
            let (canonical_abi, canonical_implementation) = facts
                .get(&(actor.module_path.clone(), actor.symbol.clone()))
                .expect("Actor target was validated before identity computation");
            *abi = canonical_abi.clone();
            *implementation = canonical_implementation.clone();
            Ok(())
        })?;
    }
    Ok(())
}

fn visit_actor_call_targets(
    unit: &mut FileIrUnit,
    mut visit: impl FnMut(
        &skiff_artifact_model::ServiceSymbolRef,
        &mut skiff_artifact_model::ActorAbiIdentity,
        &mut ActorImplementationIdentity,
    ) -> Result<()>,
) -> Result<()> {
    for body in unit
        .constants
        .iter_mut()
        .map(|constant| &mut constant.body)
        .chain(
            unit.executables
                .iter_mut()
                .map(|executable| &mut executable.body),
        )
    {
        for expression in &mut body.expressions {
            let skiff_artifact_model::ExprIr::Call { call } = expression else {
                continue;
            };
            let skiff_artifact_model::CallTargetIr::ActorMethod {
                actor,
                actor_abi_identity,
                actor_implementation_identity,
                ..
            } = &mut call.target
            else {
                continue;
            };
            visit(actor, actor_abi_identity, actor_implementation_identity)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_actor_declarations(
    ast: &SourceFile,
    module_path: &str,
    executable_index: &skiff_compiler_source::semantic::ExecutableIndex,
    executable_signatures: &BTreeMap<u32, super::function_lowering::LoweredExecutableSignature>,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    unit: &mut FileIrUnit,
) -> Result<()> {
    for actor in &ast.actors {
        let actor_id_type = crate::type_lowering::lower_type_ref(
            &actor.id_type,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            crate::type_lowering::TypeLoweringContext::value(),
        )?;
        let fields = actor
            .fields
            .iter()
            .map(|field| {
                Ok(ActorFieldIr {
                    name: field.name.clone(),
                    ty: crate::type_lowering::lower_type_ref(
                        &field.ty,
                        type_indices,
                        local_db_objects,
                        publication_db_metadata,
                        package_aliases,
                        external_type_symbols,
                        source_alias_targets,
                        crate::type_lowering::TypeLoweringContext::value(),
                    )?,
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let actor_methods = ast
            .impls
            .iter()
            .filter(|implementation| implementation.target == actor.name)
            .flat_map(|implementation| implementation.method_bodies.iter())
            .filter(|method| !method.is_static)
            .map(|method| {
                let declaration_name = impl_method_declaration_name(&actor.name, &method.name);
                let executable_index = executable_index
                    .entry(&declaration_name)
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "missing semantic executable index for Actor method `{declaration_name}`"
                        ))
                    })?
                    .executable_index;
                let signature = executable_signatures.get(&executable_index).ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "missing lowered signature for Actor method `{declaration_name}`"
                    ))
                })?;
                let method_identity =
                    skiff_artifact_identity::actor_method_identity(
                        module_path,
                        &actor.name,
                        &method.name,
                    )
                    .map_err(|error| CompileError::Semantic(error.to_string()))?;
                Ok((
                    ActorPublicMethodIr {
                        method_identity: method_identity.clone(),
                        name: method.name.clone(),
                        parameters: signature
                            .params
                            .iter()
                            .skip(usize::from(method.implicit_self.is_none()))
                            .map(|parameter| FunctionTypeParamIr {
                                name: parameter.name.clone(),
                                ty: parameter.ty.clone(),
                            })
                            .collect(),
                        return_type: signature.return_type.clone(),
                        may_suspend: signature.may_suspend,
                    },
                    method_identity,
                    executable_index,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let abi = ActorAbiInput {
            actor_name: actor.name.clone(),
            actor_id_type,
            fields,
            public_methods: actor_methods
                .iter()
                .map(|(method, _, _)| method.clone())
                .collect(),
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        };
        let actor_abi_identity = skiff_artifact_identity::actor_abi_identity(&abi)
            .map_err(|error| CompileError::Semantic(error.to_string()))?;
        unit.actor_declarations.push(ActorDeclarationIr {
            actor_abi_identity,
            // Package lowering replaces this private transient value after all
            // File IR units are available and the reachable graph can be hashed.
            actor_implementation_identity: ActorImplementationIdentity::new(
                "pending-actor-implementation",
            ),
            abi,
            method_implementations: actor_methods
                .into_iter()
                .map(|(_, identity, executable_index)| (identity, executable_index))
                .collect(),
        });
    }
    Ok(())
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
mod interface_execution_tests;

#[cfg(test)]
mod object_materialization_tests;

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::PathBuf,
    };

    use crate::{
        file_ir::{
            BoxSourceIr, CallIr, CallTargetIr, ExecutableIr, ExprIr, PackageRefIr, TypeRefIr,
        },
        source_unit_lowering::symbol,
    };
    use skiff_artifact_model::{
        validate_file_ir_service_calls, ContractOperationId, ContractRequirement,
        InstructionSourceSite, LiteralIr, NamedUnionBranchIr, NominalTypeRefBaseIr,
        PackageCallableId, PackageLocalAbiIdentity, ReceiverCallAbi, ServiceProtocolIdentity,
        SyntheticInstructionSiteReason, TypeDescriptorIr,
    };
    use skiff_compiler_input::CompilerPlatformSources;
    use skiff_compiler_source::{
        api::PublicTypeKind, build_package_from_parsed_sources,
        parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
        source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
        PackageDependency, PublicationApiEntry, PublicationApiSpec, SourceCompilePackageFacts,
    };

    use super::*;

    const MODULE: &str = "internal.any_lowering";
    const PACKAGE_ID: &str = "example.com/reader";
    const PACKAGE_MODULE: &str = "pkg.reader";

    fn initialize_test_prelude() {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve");
        initialize_prelude_registry(
            &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
        )
        .expect("prelude registry should initialize");
    }

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
              type Model {
                value: string,
              }

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
        lowered_unit_result(source_text).expect("publication should lower")
    }

    fn lowered_unit_result(source_text: &str) -> std::result::Result<FileIrUnit, String> {
        initialize_test_prelude();
        let root = PathBuf::from("/test");
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/any_lowering.skiff"),
            MODULE.to_string(),
            false,
            false,
            source_text.to_string(),
            "internal/any_lowering.skiff",
        )
        .map_err(|error| error.to_string())?;
        let production_sources = vec![source];
        let parsed_sources = parse_publication_sources(&root, &production_sources)
            .map_err(|error| error.to_string())?;
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/any-lowering"),
        })
        .map_err(|error| error.to_string())?;
        let lowered = crate::lower(&model).map_err(|error| error.to_string())?;
        lowered
            .file_ir_units()
            .first()
            .cloned()
            .ok_or_else(|| "one File IR unit should be emitted".to_string())
    }

    #[test]
    fn source_declarations_lower_to_exact_mutually_exclusive_descriptors_and_branch_inputs() {
        let unit = lowered_unit(
            r#"
              type ShapeA { value: string }
              type ShapeB { value: string }
              type Box<T> { value: T }
              type PrimitiveFailure = string
              type UnionOne discriminator "kind" =
                ShapeA |
                Box<string> |
                { kind: "same", value: string } |
                "literal"
              type UnionTwo discriminator "kind" =
                ShapeB |
                { kind: "same", value: string } |
                "literal"
              alias TransparentFailure = ShapeA
              interface Marker {
                function label(self: Self) -> string
              }
            "#,
        );

        let declaration = |name: &str| {
            unit.type_table
                .iter()
                .find(|declaration| declaration.name == name)
                .unwrap_or_else(|| panic!("missing declaration `{name}`"))
        };
        let shape_a = declaration("ShapeA");
        let shape_b = declaration("ShapeB");
        assert!(matches!(
            (&shape_a.descriptor, &shape_b.descriptor),
            (
                TypeDescriptorIr::Record { fields: left },
                TypeDescriptorIr::Record { fields: right },
            ) if left == right
        ));
        assert!(matches!(
            declaration("PrimitiveFailure").descriptor,
            TypeDescriptorIr::Representation { ref representation }
                if representation == &TypeRefIr::builtin("string")
        ));
        assert!(matches!(
            declaration("TransparentFailure").descriptor,
            TypeDescriptorIr::Alias {
                target: TypeRefIr::LocalType { type_index: 0 },
            }
        ));
        assert!(matches!(
            declaration("Marker").descriptor,
            TypeDescriptorIr::Interface
        ));

        let TypeDescriptorIr::Union {
            branches: union_one,
        } = &declaration("UnionOne").descriptor
        else {
            panic!("UnionOne must lower as a named union");
        };
        assert_eq!(union_one.len(), 4);
        assert!(matches!(
            &union_one[0],
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::LocalType { type_index: 0 },
            }
        ));
        assert!(matches!(
            &union_one[1],
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index: 2 },
                    arguments,
                },
            } if arguments == &vec![TypeRefIr::builtin("string")]
        ));
        assert!(matches!(
            &union_one[2],
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::Record { fields },
                discriminator_field,
                discriminator_value,
            } if discriminator_field == "kind"
                && discriminator_value == "same"
                && fields["kind"] == TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "same".to_string(),
                    },
                }
        ));
        assert!(matches!(
            &union_one[3],
            NamedUnionBranchIr::Literal {
                value: LiteralIr::String { value },
            } if value == "literal"
        ));

        let TypeDescriptorIr::Union {
            branches: union_two,
        } = &declaration("UnionTwo").descriptor
        else {
            panic!("UnionTwo must lower as a distinct named union");
        };
        assert!(matches!(
            &union_two[0],
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::LocalType { type_index: 1 },
                ..
            }
        ));
        assert_eq!(union_one[2], union_two[1]);
        assert_eq!(union_one[3], union_two[2]);
        assert_ne!(declaration("UnionOne").name, declaration("UnionTwo").name);
    }

    #[test]
    fn applied_nominals_flow_from_source_through_file_ir_signatures_sites_and_calls() {
        let unit = lowered_unit(
            r#"
              type Id = string
              type Box<T> { value: T }
              type Outer<A, B> { first: A, second: B }
              type Token<T> = string
              type Branch<T> { value: T }
              type Choice<T> discriminator "kind" =
                Branch<T> |
                { kind: "inline", value: T } |
                "literal"
              alias StringBox = Box<string>

              function use(
                stringBox: Box<string>,
                numberBox: Box<number>,
                nested: Outer<Box<string>, Array<Id>>,
                token: Token<string>,
                choice: Choice<string>
              ) -> Box<string> {
                const constructed = Box<string> { value: stringBox.value }
                const empty = Array.empty<Box<string>>()
                return constructed
              }

              function fail(value: Box<string>) -> void {
                throw value
              }

              function caught(value: Box<string>) -> void {
                const attempted = catch<Box<string>>(throw value)
              }

              function inspected(boxed: Box<string>) -> void {
                match boxed {
                  Box<string> { value } => {
                  }
                  _ => {
                  }
                }
              }
            "#,
        );
        assert_eq!(unit.schema_version, "skiff-file-ir-v10");
        assert_eq!(unit.ir_format_version, "skiff-file-ir-format-v7");
        assert_eq!(unit.opcode_table_version, "skiff-opcode-table-v2");

        let declaration = |name: &str| {
            unit.type_table
                .iter()
                .find(|declaration| declaration.name == name)
                .unwrap_or_else(|| panic!("missing declaration `{name}`"))
        };
        let index = |name: &str| {
            unit.type_table
                .iter()
                .position(|declaration| declaration.name == name)
                .unwrap_or_else(|| panic!("missing declaration `{name}`")) as u32
        };
        let applied = |name: &str, arguments: Vec<TypeRefIr>| TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: index(name),
            },
            arguments,
        };
        let string_box = applied("Box", vec![TypeRefIr::builtin("string")]);
        let number_box = applied("Box", vec![TypeRefIr::builtin("number")]);

        assert_eq!(declaration("Box").type_params, ["T".to_string()]);
        assert!(matches!(
            &declaration("Box").descriptor,
            TypeDescriptorIr::Record { fields }
                if fields["value"] == TypeRefIr::TypeParam { name: "T".to_string() }
        ));
        assert_eq!(declaration("Token").type_params, ["T".to_string()]);
        assert!(matches!(
            declaration("Token").descriptor,
            TypeDescriptorIr::Representation { ref representation }
                if representation == &TypeRefIr::builtin("string")
        ));
        assert!(matches!(
            &declaration("StringBox").descriptor,
            TypeDescriptorIr::Alias { target } if target == &string_box
        ));

        let TypeDescriptorIr::Union {
            branches: choice_branches,
        } = &declaration("Choice").descriptor
        else {
            panic!("generic Choice must remain a named union");
        };
        assert!(matches!(
            &choice_branches[0],
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type:
                    TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index },
                        arguments,
                    },
            } if *type_index == index("Branch")
                && arguments == &vec![TypeRefIr::TypeParam { name: "T".to_string() }]
        ));
        assert!(matches!(
            &choice_branches[1],
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::Record { fields },
                ..
            } if fields["value"] == TypeRefIr::TypeParam { name: "T".to_string() }
        ));
        assert!(matches!(
            &choice_branches[2],
            NamedUnionBranchIr::Literal { .. }
        ));

        let use_executable = executable(&unit, "use");
        assert_eq!(use_executable.params[0].ty, string_box);
        assert_eq!(use_executable.params[1].ty, number_box);
        assert_ne!(use_executable.params[0].ty, use_executable.params[1].ty);
        assert_eq!(
            use_executable.params[2].ty,
            applied(
                "Outer",
                vec![
                    string_box.clone(),
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::LocalType {
                            type_index: index("Id"),
                        }],
                    },
                ],
            )
        );
        assert_eq!(
            use_executable.params[3].ty,
            applied("Token", vec![TypeRefIr::builtin("string")])
        );
        assert_eq!(
            use_executable.params[4].ty,
            applied("Choice", vec![TypeRefIr::builtin("string")])
        );
        assert_eq!(use_executable.return_type, string_box);
        assert!(use_executable.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Construct { type_ref, .. } if type_ref == &string_box
            )
        }));
        assert!(use_executable.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Call { call }
                    if call.type_args.get("T0") == Some(&string_box)
            )
        }));

        let failed = executable(&unit, "fail");
        assert!(failed.body.statements.iter().any(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::Throw { payload_type, .. }
                    if payload_type == &string_box
            )
        }));
        let caught = executable(&unit, "caught");
        assert!(caught.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Throw { payload_type, .. } if payload_type == &string_box
            )
        }));
        assert!(caught.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Catch { catch_type, .. } if catch_type == &string_box
            )
        }));
        let inspected = executable(&unit, "inspected");
        assert!(inspected.body.statements.iter().any(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::Match { arms, .. }
                    if matches!(
                        &arms[0].pattern,
                        skiff_artifact_model::PatternIr::Type { ty } if ty == &string_box
                    )
            )
        }));

        let wire = serde_json::to_string(&unit).expect("File IR serializes");
        assert!(wire.contains("\"kind\":\"appliedNominal\""));
        assert!(!wire.contains("\"typeArguments\""));
    }

    #[test]
    fn explicit_representation_constructors_preserve_wraps_order_and_throw_site() {
        let unit = lowered_unit(
            r#"
              type Plain = string
              type Generic<A, B> = string
              type Inner = string
              type Outer = Inner

              function payload(value: string) -> string {
                return value
              }

              function plain() -> Plain {
                return Plain("plain")
              }

              function generic() -> Generic<number, string> {
                return Generic<number, string>("generic")
              }

              function passthrough(value: Plain) -> Plain {
                return value
              }

              function nested() -> Outer {
                return Outer(Inner(payload("nested")))
              }

              function fail() -> void {
                throw Plain(payload("failure"))
              }
            "#,
        );
        let type_index = |name: &str| {
            unit.type_table
                .iter()
                .position(|declaration| declaration.name == name)
                .unwrap_or_else(|| panic!("missing representation `{name}`")) as u32
        };
        let plain_type = TypeRefIr::LocalType {
            type_index: type_index("Plain"),
        };
        let generic_type = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType {
                type_index: type_index("Generic"),
            },
            arguments: vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
        };
        let inner_type = TypeRefIr::LocalType {
            type_index: type_index("Inner"),
        };
        let outer_type = TypeRefIr::LocalType {
            type_index: type_index("Outer"),
        };

        let only_wrap = |name: &str| {
            executable(&unit, name)
                .body
                .expressions
                .iter()
                .filter_map(|expression| match expression {
                    ExprIr::RepresentationWrap { value, type_ref } => {
                        Some((*value, type_ref.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(only_wrap("plain").len(), 1);
        assert_eq!(only_wrap("plain")[0].1, plain_type);
        assert_eq!(only_wrap("generic").len(), 1);
        assert_eq!(only_wrap("generic")[0].1, generic_type);
        assert!(
            only_wrap("passthrough").is_empty(),
            "assignability must not synthesize an implicit representation wrap"
        );

        let nested = executable(&unit, "nested");
        let nested_wraps = only_wrap("nested");
        assert_eq!(
            nested_wraps
                .iter()
                .map(|(_, type_ref)| type_ref)
                .collect::<Vec<_>>(),
            vec![&inner_type, &outer_type]
        );
        let inner_wrap_index = nested
            .body
            .expressions
            .iter()
            .position(|expression| {
                matches!(
                    expression,
                    ExprIr::RepresentationWrap { type_ref, .. } if type_ref == &inner_type
                )
            })
            .expect("inner representation wrap");
        let outer_wrap_index = nested
            .body
            .expressions
            .iter()
            .position(|expression| {
                matches!(
                    expression,
                    ExprIr::RepresentationWrap { type_ref, .. } if type_ref == &outer_type
                )
            })
            .expect("outer representation wrap");
        let nested_call_index = nested
            .body
            .expressions
            .iter()
            .position(|expression| matches!(expression, ExprIr::Call { .. }))
            .expect("nested payload call");
        assert!(nested_call_index < inner_wrap_index);
        assert_eq!(
            nested_wraps[0].0.expression as usize, nested_call_index,
            "the inner wrap must reference the once-lowered payload call"
        );
        assert!(inner_wrap_index < outer_wrap_index);
        assert_eq!(
            nested_wraps[1].0.expression as usize, inner_wrap_index,
            "the outer wrap must reference the explicit inner wrap"
        );
        assert_eq!(
            nested
                .body
                .expressions
                .iter()
                .filter(|expression| matches!(expression, ExprIr::Call { .. }))
                .count(),
            1,
            "the payload side effect must lower exactly once"
        );

        let fail = executable(&unit, "fail");
        assert_eq!(
            fail.body
                .expressions
                .iter()
                .filter(|expression| matches!(expression, ExprIr::Call { .. }))
                .count(),
            1,
            "the thrown payload side effect must lower exactly once"
        );
        let fail_call_index = fail
            .body
            .expressions
            .iter()
            .position(|expression| matches!(expression, ExprIr::Call { .. }))
            .expect("throw payload call");
        let (fail_wrap_index, fail_wrap_value) = fail
            .body
            .expressions
            .iter()
            .enumerate()
            .find_map(|(index, expression)| match expression {
                ExprIr::RepresentationWrap { value, type_ref } if type_ref == &plain_type => {
                    Some((index, *value))
                }
                _ => None,
            })
            .expect("direct throw representation wrap");
        assert!(fail_call_index < fail_wrap_index);
        assert_eq!(fail_wrap_value.expression as usize, fail_call_index);
        assert!(fail.body.statements.iter().any(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::Throw {
                    value,
                    payload_type,
                    site: InstructionSourceSite::Source { span },
                } if value.expression as usize == fail_wrap_index
                    && payload_type == &plain_type
                    && span.source_id == 0
                    && span.start.line > 0
            )
        }));
    }

    #[test]
    fn representation_wrap_preserves_external_package_owner_in_ordered_arguments() {
        let unit = lowered_unit_with_package_facts(
            r#"
              type Generic<A, B> = string

              function make() -> Generic<pkg.Model, number> {
                return Generic<pkg.Model, number>("value")
              }
            "#,
        );
        let make = executable(&unit, "make");
        let type_ref = make
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::RepresentationWrap { type_ref, .. } => Some(type_ref),
                _ => None,
            })
            .expect("external package argument representation wrap");

        assert!(
            matches!(
                type_ref,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                arguments,
            } if matches!(
                arguments.as_slice(),
                [
                    TypeRefIr::PackageSymbol { symbol },
                    TypeRefIr::Builtin {
                        name,
                        args,
                    },
                ] if matches!(
                    &symbol.package,
                    PackageRefIr::Dependency { dependency_ref } if dependency_ref == "pkg"
                )
                    && symbol.symbol_path == "Model"
                    && symbol.abi_expectation.is_none()
                    && name == "number"
                    && args.is_empty()
            )
            ),
            "{type_ref:#?}"
        );
        assert!(unit.external_refs.package_symbols.iter().any(|symbol| {
            matches!(
                &symbol.package,
                PackageRefIr::Dependency { dependency_ref } if dependency_ref == "pkg"
            ) && symbol.symbol_path == "Model"
        }));
    }

    #[test]
    fn non_representation_constructor_target_remains_a_source_error() {
        let error = lowered_unit_result(
            r#"
              type Record { value: string }

              function invalid() -> void {
                Record("not a record constructor")
              }
            "#,
        )
        .expect_err("a record call must not become a representation wrap");

        assert!(error.contains("Record"), "{error}");
        assert!(
            error.contains("unresolved")
                || error.contains("not resolved")
                || error.contains("unsupported"),
            "{error}"
        );
    }

    #[test]
    fn source_calls_and_throws_keep_real_sites_and_catch_type_is_required() {
        let unit = lowered_unit(
            r#"
              type Failure { message: string }

              function callee(value: string) -> string {
                return value
              }

              function statement(failure: Failure) -> void {
                callee("call")
                throw failure
              }

              function expression(failure: Failure) -> Failure {
                return throw failure
              }

              function caught(value: string) -> void {
                const attempted = catch<Failure>(callee(value))
              }
            "#,
        );

        let statement = executable(&unit, "statement");
        let call = statement
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .expect("source call lowers");
        assert!(matches!(
            call.site,
            InstructionSourceSite::Source { ref span }
                if span.source_id == 0 && span.start.line > 0
        ));
        let throw_site = statement
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                skiff_artifact_model::StmtIr::Throw {
                    payload_type, site, ..
                } => Some((payload_type, site)),
                _ => None,
            })
            .expect("statement throw lowers");
        assert_eq!(throw_site.0, &TypeRefIr::LocalType { type_index: 0 });
        assert!(matches!(
            throw_site.1,
            InstructionSourceSite::Source { span }
                if span.source_id == 0 && span.start.line > 0
        ));

        let expression = executable(&unit, "expression");
        assert!(expression.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Throw {
                    payload_type: TypeRefIr::LocalType { type_index: 0 },
                    site: InstructionSourceSite::Source { span },
                    ..
                } if span.source_id == 0 && span.start.line > 0
            )
        }));

        let caught = executable(&unit, "caught");
        assert!(caught.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Catch {
                    catch_type: TypeRefIr::LocalType { type_index: 0 },
                    ..
                }
            )
        }));
        assert!(caught.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::Call {
                    call: CallIr {
                        site: InstructionSourceSite::Source { span },
                        ..
                    },
                } if span.source_id == 0 && span.start.line > 0
            )
        }));

        let wire = serde_json::to_value(&unit).expect("File IR serializes");
        assert!(
            !wire.to_string().contains("\"catchType\":null"),
            "typed catch cannot serialize an implicit catch-all"
        );
    }

    #[test]
    fn compiler_generated_native_wrapper_uses_only_the_wrapper_synthetic_reason() {
        let mut units = lowered_units_for_package(
            "skiff.run/std",
            vec![(
                "std/wrapper_fixture.skiff",
                "std.wrapper_fixture",
                "native function passthrough(value: string) -> string",
            )],
        );
        let unit = units.pop().expect("one native wrapper File IR unit");
        let wrapper = unit
            .executables
            .iter()
            .find(|executable| executable.symbol == "std.wrapper_fixture.passthrough")
            .expect("native wrapper executable lowers");
        let call = wrapper
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .expect("native wrapper contains its generated native call");

        assert!(matches!(
            call.site,
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            }
        ));
    }

    #[test]
    fn validated_package_db_schema_lowers_to_typed_file_ir() {
        let unit = lowered_unit(
            r#"
                type Owner { id: string }
                type Thread { id: string, owner: Owner }
                db object Thread {
                  primary key(id)
                  unique index byOwner(owner.id desc) where owner.id != ""
                }
            "#,
        );

        let db = unit
            .declarations
            .db
            .get("Thread")
            .expect("validated package DB declaration should lower");
        assert_eq!(db.key.name, "id");
        assert!(db.fields.iter().any(|field| field.name == "owner"));
        assert_eq!(db.indexes.len(), 1);
        assert_eq!(db.indexes[0].name, "byOwner");
        assert!(db.indexes[0].unique);
        assert_eq!(db.indexes[0].fields[0].field.text, "owner.id");
        assert_eq!(
            db.indexes[0].fields[0].field.segments,
            ["owner".to_string(), "id".to_string()]
        );
        assert_eq!(
            db.indexes[0].fields[0].direction,
            skiff_artifact_model::DbIndexDirectionIr::Desc
        );
        assert!(db.indexes[0].where_expr.is_some());
    }

    fn lowered_units(sources: Vec<(&str, &str, &str)>) -> Vec<FileIrUnit> {
        lowered_units_for_package("example.com/publication-local-refs", sources)
    }

    fn lowered_units_for_package(
        package_id: &str,
        sources: Vec<(&str, &str, &str)>,
    ) -> Vec<FileIrUnit> {
        lowered_units_result(package_id, sources).expect("publication should lower")
    }

    fn lowered_units_result(
        package_id: &str,
        sources: Vec<(&str, &str, &str)>,
    ) -> std::result::Result<Vec<FileIrUnit>, String> {
        initialize_test_prelude();
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
                .map_err(|error| error.to_string())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let parsed_sources = parse_publication_sources(&root, &production_sources)
            .map_err(|error| error.to_string())?;
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(package_id),
        })
        .map_err(|error| error.to_string())?;
        crate::lower(&model)
            .map_err(|error| error.to_string())
            .map(|lowered| lowered.file_ir_units().to_vec())
    }

    fn lowered_unit_with_package_facts(source_text: &str) -> FileIrUnit {
        initialize_test_prelude();
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
        let package_api = PublicationApiSpec::from_entries(vec![
            PublicationApiEntry::for_source("Reader", PACKAGE_MODULE, "Reader"),
            PublicationApiEntry::for_source("Model", PACKAGE_MODULE, "Model"),
        ]);
        let package_production_sources = vec![package_source];
        let package_parsed_sources =
            parse_publication_sources(&package_root, &package_production_sources)
                .expect("package source facts should build");
        let package_aliases = BTreeMap::new();
        let package_dependencies = Vec::<PackageDependency>::new();
        let package_model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
            parsed_sources: package_parsed_sources,
            production_sources: package_production_sources,
            diagnostic_root: &package_root,
            publication_api: Some(&package_api),
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(PACKAGE_ID),
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
        let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: Some(&package_facts),
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/any-lowering"),
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
    fn emits_actor_declaration_and_exact_registry_type_arguments() {
        initialize_test_prelude();
        let unit = lowered_unit(
            r#"
              actor UserActor id string {
                displayName: string,
                loginCount: number,
              }

              impl UserActor {
                function rename(self: UserActor, value: string) -> string {
                  self.displayName = value
                  return self.displayName
                }

                function increment(delta: number) -> number {
                  self.loginCount = self.loginCount + delta
                  return self.loginCount
                }
              }

              function load(id: string) -> UserActor {
                const actor = std.actor.getOrCreate<UserActor>(
                  id,
                  { displayName: "Ada", loginCount: 1 }
                )
                const found = std.actor.find<UserActor>(id)
                return actor
              }

              function invoke(actor: UserActor) -> string {
                return actor.rename("Grace")
              }
            "#,
        );

        let declaration = unit
            .actor_declarations
            .first()
            .expect("actor declaration should be emitted in its owner file");
        assert_eq!(declaration.abi.actor_name, "UserActor");
        assert_eq!(declaration.abi.actor_id_type, TypeRefIr::builtin("string"));
        assert_eq!(
            declaration
                .abi
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            ["displayName", "loginCount"]
        );
        assert_eq!(declaration.abi.public_methods.len(), 2);
        let rename = declaration
            .abi
            .public_methods
            .iter()
            .find(|method| method.name == "rename")
            .unwrap();
        assert_eq!(rename.name, "rename");
        assert_eq!(rename.parameters.len(), 1);
        assert_eq!(rename.parameters[0].name, "value");
        assert_eq!(rename.return_type, TypeRefIr::builtin("string"));
        assert!(!rename.may_suspend);
        let increment = declaration
            .abi
            .public_methods
            .iter()
            .find(|method| method.name == "increment")
            .unwrap();
        assert_eq!(
            increment
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["delta"]
        );
        assert_eq!(increment.return_type, TypeRefIr::builtin("number"));
        assert_eq!(
            declaration
                .method_implementations
                .get(&rename.method_identity),
            unit.declarations
                .executables
                .get("UserActor.rename")
                .map(|entry| &entry.executable_index)
        );
        assert!(!declaration
            .actor_implementation_identity
            .as_str()
            .contains("pending"));

        let rename_executable = executable(&unit, "UserActor.rename");
        assert!(rename_executable.body.expressions.iter().any(|expression| {
            matches!(
                expression,
                ExprIr::ActorSelfField { field, field_type }
                    if field == "displayName" && field_type == &TypeRefIr::builtin("string")
            )
        }));
        assert!(rename_executable.body.statements.iter().any(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::Assign {
                    target: skiff_artifact_model::AssignTargetIr::ActorSelfField {
                        field,
                        field_type,
                    },
                    ..
                } if field == "displayName" && field_type == &TypeRefIr::builtin("string")
            )
        }));

        let load = executable(&unit, "load");
        let calls = load
            .body
            .expressions
            .iter()
            .filter_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .filter(|call| {
                matches!(
                    &call.target,
                    CallTargetIr::Native { target }
                        if target.binding_key.as_deref().is_some_and(|key| key.starts_with("std.actor."))
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);

        let invoke = executable(&unit, "invoke");
        let actor_call = invoke
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call {
                    call:
                        call @ skiff_artifact_model::CallIr {
                            target: CallTargetIr::ActorMethod { .. },
                            ..
                        },
                } => Some(call),
                _ => None,
            })
            .expect("Actor receiver call should keep its dedicated target");
        let CallTargetIr::ActorMethod {
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        } = &actor_call.target
        else {
            unreachable!("find_map only returns ActorMethod");
        };
        assert_eq!(actor.module_path, MODULE);
        assert_eq!(actor.symbol, "UserActor");
        assert_eq!(actor_abi_identity, &declaration.actor_abi_identity);
        assert_eq!(
            actor_implementation_identity,
            &declaration.actor_implementation_identity
        );
        assert_eq!(method_identity, &rename.method_identity);

        let get_or_create = calls
            .iter()
            .find(|call| {
                matches!(
                    &call.target,
                    CallTargetIr::Native { target }
                        if target.binding_key.as_deref() == Some("std.actor.getOrCreate")
                )
            })
            .expect("getOrCreate call should be lowered");
        assert_eq!(get_or_create.type_args["T1"], TypeRefIr::builtin("string"));
        assert!(matches!(
            &get_or_create.type_args["T2"],
            TypeRefIr::Record { fields }
                if fields["displayName"] == TypeRefIr::builtin("string")
                    && fields["loginCount"] == TypeRefIr::builtin("number")
        ));

        let find = calls
            .iter()
            .find(|call| {
                matches!(
                    &call.target,
                    CallTargetIr::Native { target }
                        if target.binding_key.as_deref() == Some("std.actor.find")
                )
            })
            .expect("find call should be lowered");
        assert_eq!(find.type_args["T1"], TypeRefIr::builtin("string"));
        assert!(!find.type_args.contains_key("T2"));
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
    fn lowers_cross_module_generic_function_to_exact_publication_executable() {
        let units = lowered_units(vec![
            (
                "internal/worker.skiff",
                "internal.worker",
                r#"
                  function identity<T>(value: T) -> T {
                    return value
                  }
                "#,
            ),
            (
                "internal/runner.skiff",
                "internal.runner",
                r#"
                  function run() -> string {
                    return root.internal.worker.identity<string>("ok")
                  }
                "#,
            ),
        ]);
        let worker = units
            .iter()
            .find(|unit| unit.module_path == "internal.worker")
            .unwrap();
        let runner = units
            .iter()
            .find(|unit| unit.module_path == "internal.runner")
            .unwrap();
        let expected_index = worker.declarations.executables["identity"].executable_index;
        let call = runner
            .executables
            .iter()
            .find(|executable| executable.symbol == "internal.runner.run")
            .expect("runner executable")
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .expect("generic call should lower");
        assert!(matches!(
            call.target,
            CallTargetIr::PublicationExecutable {
                ref module_path,
                executable_index,
            } if module_path == "internal.worker" && executable_index == expected_index
        ));
        assert_eq!(call.type_args["T0"], TypeRefIr::builtin("string"));
    }

    #[test]
    fn lowers_cross_module_generic_impl_receiver_to_exact_publication_executable() {
        let units = lowered_units(vec![
            (
                "internal/worker.skiff",
                "internal.worker",
                r#"
                  type Box<T> { value: T }

                  impl Box<T> {
                    function unwrap() -> T {
                      return self.value
                    }
                  }
                "#,
            ),
            (
                "internal/runner.skiff",
                "internal.runner",
                r#"
                  function run(box: root.internal.worker.Box<string>) -> string {
                    return box.unwrap()
                  }
                "#,
            ),
        ]);
        let worker = units
            .iter()
            .find(|unit| unit.module_path == "internal.worker")
            .unwrap();
        let runner = units
            .iter()
            .find(|unit| unit.module_path == "internal.runner")
            .unwrap();
        let expected_index = worker.declarations.executables["Box<T>.unwrap"].executable_index;
        let call = runner
            .executables
            .iter()
            .find(|executable| executable.symbol == "internal.runner.run")
            .expect("runner executable")
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .expect("generic receiver call should lower");
        assert!(matches!(
            call.target,
            CallTargetIr::PublicationExecutable {
                ref module_path,
                executable_index,
            } if module_path == "internal.worker" && executable_index == expected_index
        ));
        assert_eq!(call.type_args["T0"], TypeRefIr::builtin("string"));
    }

    #[test]
    fn lowers_cross_module_const_initializer_call_to_exact_publication_executable() {
        let units = lowered_units(vec![
            (
                "internal/worker.skiff",
                "internal.worker",
                r#"
                  function label() -> string {
                    return "worker"
                  }
                "#,
            ),
            (
                "internal/runner.skiff",
                "internal.runner",
                r#"
                  const LABEL: string = root.internal.worker.label()

                  function run() -> string {
                    return LABEL
                  }
                "#,
            ),
        ]);
        let worker = units
            .iter()
            .find(|unit| unit.module_path == "internal.worker")
            .unwrap();
        let runner = units
            .iter()
            .find(|unit| unit.module_path == "internal.runner")
            .unwrap();
        let expected_index = worker.declarations.executables["label"].executable_index;
        let call = runner.constants[0]
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .expect("const initializer call should lower");
        assert!(matches!(
            call.target,
            CallTargetIr::PublicationExecutable {
                ref module_path,
                executable_index,
            } if module_path == "internal.worker" && executable_index == expected_index
        ));
    }

    #[test]
    fn rethrow_statement_keeps_following_local_recursive_call_target_aligned() {
        let unit = lowered_unit(
            r#"
              function attempt() -> void {
                return null
              }

              function retry(remainingAttempts: integer) -> void {
                const result = catch<std.db.ConflictError>(attempt())
                if result.tag == "ok" {
                  return null
                }
                if remainingAttempts == 0 {
                  const exception = result.exception
                  rethrow exception
                }
                return retry(remainingAttempts)
              }
            "#,
        );
        let retry_index = unit.declarations.executables["retry"].executable_index;
        let retry = unit
            .executables
            .iter()
            .find(|executable| executable.symbol == format!("{MODULE}.retry"))
            .expect("retry executable");

        assert!(
            retry.body.expressions.iter().any(|expression| matches!(
                expression,
                ExprIr::Call { call }
                    if matches!(
                        call.target,
                        CallTargetIr::LocalExecutable { executable_index }
                            if executable_index == retry_index
                    )
            )),
            "the self-recursive call after rethrow must retain its exact local target"
        );
    }

    #[test]
    fn generic_function_and_impl_self_recursion_use_exact_local_targets() {
        let unit = lowered_unit(
            r#"
              type Box<T> {
                value: T,
              }

              function retryValue<T>(value: T, remainingAttempts: integer) -> T {
                if remainingAttempts == 0 {
                  return value
                }
                return retryValue<T>(value, remainingAttempts)
              }

              impl Box<T> {
                function retry(remainingAttempts: integer) -> T {
                  if remainingAttempts == 0 {
                    return self.value
                  }
                  return self.retry(remainingAttempts)
                }
              }
            "#,
        );
        for declaration_name in ["retryValue", "Box<T>.retry"] {
            let expected_index = unit.declarations.executables[declaration_name].executable_index;
            let executable = &unit.executables[expected_index as usize];
            assert!(
                executable
                    .body
                    .expressions
                    .iter()
                    .any(|expression| matches!(
                        expression,
                        ExprIr::Call { call }
                            if matches!(
                                call.target,
                                CallTargetIr::LocalExecutable { executable_index }
                                    if executable_index == expected_index
                            )
                    )),
                "`{declaration_name}` must resolve its self-edge to its canonical executable index"
            );
        }
    }

    #[test]
    fn ambiguous_generic_impl_receiver_fails_before_file_ir() {
        let error = lowered_units_result(
            "example.com/ambiguous-generic-impl",
            vec![
                (
                    "internal/worker.skiff",
                    "internal.worker",
                    r#"
                      type Box<T> { value: T }

                      impl Box<T> {
                        function unwrap() -> T { return self.value }
                      }

                      impl Box<U> {
                        function unwrap() -> U { return self.value }
                      }
                    "#,
                ),
                (
                    "internal/runner.skiff",
                    "internal.runner",
                    r#"
                      function run(box: root.internal.worker.Box<string>) -> string {
                        return box.unwrap()
                      }
                    "#,
                ),
            ],
        )
        .expect_err("ambiguous generic impl receiver must fail closed");
        assert!(
            error.contains("duplicate")
                || error.contains("ambiguous")
                || error.contains("more than once")
                || error.contains("no exact typed source target"),
            "unexpected ambiguity diagnostic: {error}"
        );
    }

    #[test]
    fn lowers_current_package_symbol_types_to_direct_publication_addresses() {
        let units = lowered_units_for_package(
            skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID,
            vec![
                (
                    "std/time.skiff",
                    "std.time",
                    r#"
                      type Duration = integer

                      function identity(duration: Duration) -> Duration {
                        return duration
                      }
                    "#,
                ),
                (
                    "std/consumer.skiff",
                    "std.consumer",
                    r#"
                      function passthrough(duration: std.time.Duration) -> std.time.Duration {
                        return duration
                      }
                    "#,
                ),
            ],
        );
        let time = units
            .iter()
            .find(|unit| unit.module_path == "std.time")
            .expect("std.time unit should be emitted");
        let consumer = units
            .iter()
            .find(|unit| unit.module_path == "std.consumer")
            .expect("std.consumer unit should be emitted");
        let duration_type_index = time
            .declarations
            .types
            .get("Duration")
            .expect("Duration declaration should exist")
            .type_index;
        let identity = time
            .executables
            .iter()
            .find(|executable| executable.symbol == "std.time.identity")
            .expect("identity executable should exist");
        let passthrough = consumer
            .executables
            .iter()
            .find(|executable| executable.symbol == "std.consumer.passthrough")
            .expect("passthrough executable should exist");

        assert_eq!(
            identity.params[0].ty,
            TypeRefIr::LocalType {
                type_index: duration_type_index,
            }
        );
        assert_eq!(
            identity.return_type,
            TypeRefIr::LocalType {
                type_index: duration_type_index,
            }
        );
        let expected_cross_module = TypeRefIr::PublicationType {
            module_path: "std.time".to_string(),
            type_index: duration_type_index,
        };
        assert_eq!(passthrough.params[0].ty, expected_cross_module);
        assert_eq!(passthrough.return_type, expected_cross_module);
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
        assert_eq!(slot.signature.return_type, TypeRefIr::builtin("string"));
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
            vec![TypeRefIr::builtin("string")]
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
        assert_eq!(slot.signature.params[1].ty, TypeRefIr::builtin("string"));
        assert_eq!(slot.signature.return_type, TypeRefIr::builtin("string"));
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
        let provider_type_index = unit
            .declarations
            .types
            .get("Provider")
            .expect("Provider declaration should exist")
            .type_index;
        assert_eq!(
            interface_ty,
            TypeRefIr::PublicationType {
                module_path: MODULE.to_string(),
                type_index: provider_type_index,
            }
        );
        assert!(interface.canonical_type_args.is_empty());
    }

    #[test]
    fn exact_package_any_interface_function_param_preserves_package_owner() {
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
            PackageRefIr::PackageId { ref package_id } if package_id == PACKAGE_ID
        ));
        assert_eq!(
            interface.canonical_type_args,
            vec![TypeRefIr::builtin("string")]
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

    #[test]
    fn exact_receiver_builtin_targets_are_consumed_from_source_facts() {
        let unit = lowered_unit(
            r#"
              function isBefore(left: Date, right: Date) -> bool {
                return left.isBefore(right)
              }

              function epoch(value: Date) -> integer {
                return value.toEpochMilliseconds()
              }

              function millis(value: Duration) -> integer {
                return value.toMilliseconds()
              }

              function now() -> Date {
                return Date.now()
              }

              function sleep() -> void {
                return std.time.sleep(Duration.milliseconds(0))
              }
            "#,
        );
        let targets = unit
            .executables
            .iter()
            .flat_map(|executable| executable.body.expressions.iter())
            .filter_map(|expression| match expression {
                ExprIr::Call {
                    call:
                        CallIr {
                            target: CallTargetIr::ReceiverBuiltin { op },
                            ..
                        },
                } => Some(op.canonical_key),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            targets,
            BTreeSet::from([
                "receiver:Date.isBefore@1",
                "receiver:Date.toEpochMilliseconds@1",
                "receiver:Duration.toMilliseconds@1",
            ])
        );
        for name in ["isBefore", "epoch", "millis", "now"] {
            assert!(
                !executable(&unit, name).may_suspend,
                "{name} should consume exact non-suspending callable semantics"
            );
        }
        assert!(
            executable(&unit, "sleep").may_suspend,
            "sleep should consume its exact may-suspend descriptor"
        );
    }

    #[test]
    fn typed_contract_call_site_lowers_to_canonical_service_call_without_legacy_operation_abi() {
        let source = r#"
          function run() -> void {
            echo/ping()
          }
        "#;
        let operation_id = ContractOperationId::new("operation:ping");
        let protocol = ServiceProtocolIdentity::new("protocol:echo");
        let contract_requirement = ContractRequirement {
            alias: "echo".to_string(),
            service_id: "example.echo".to_string(),
            contract_version: "1.0.0".to_string(),
            expected_protocol_identity: protocol.clone(),
        };
        let expression = skiff_compiler_source::ExpressionKey::new(
            MODULE,
            skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
            0,
        );
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                expression,
                skiff_compiler_source::ResolvedCallTarget::ContractOperation {
                    contract_requirement,
                    contract_operation_id: operation_id.clone(),
                },
            )]));
        let service_calls = crate::lower_service_calls(&targets).unwrap();
        let service_aliases = BTreeSet::from(["echo".to_string()]);
        let ast = parse_source(source).unwrap();
        let unit = compile_parsed_source_file_ir_unit_with_lowering_context(
            ast,
            source,
            "internal/any_lowering.skiff",
            MODULE,
            "package",
            &SourceFileLoweringContext {
                service_dependency_aliases: &service_aliases,
                service_calls: Some(&service_calls),
                ..SourceFileLoweringContext::none()
            },
        )
        .unwrap();

        validate_file_ir_service_calls(&unit).unwrap();
        assert_eq!(unit.external_refs.service_call_refs.len(), 1);
        assert_eq!(
            unit.external_refs.service_call_refs[0].contract_operation_id,
            operation_id
        );
        assert_eq!(
            unit.external_refs.service_call_refs[0].expected_protocol_identity,
            protocol
        );
        let run = executable(&unit, "run");
        assert!(run.body.expressions.iter().any(|expression| matches!(
            expression,
            ExprIr::Call { call }
                if matches!(call.target, CallTargetIr::ServiceCall { .. })
        )));
        assert!(!run.body.expressions.iter().any(|expression| matches!(
            expression,
            ExprIr::Call { call }
                if matches!(call.target, CallTargetIr::ServiceDependencySymbol { .. })
        )));
        let wire = serde_json::to_string(&unit).unwrap();
        assert!(!wire.contains("operationAbiId"));
        assert!(!wire.contains("serviceDependencySymbols"));
    }

    fn package_call_source() -> &'static str {
        r#"
          function run() -> void {
            utils/format()
          }
        "#
    }

    fn package_call_expression() -> skiff_compiler_source::ExpressionKey {
        skiff_compiler_source::ExpressionKey::new(
            MODULE,
            skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
            0,
        )
    }

    fn lower_package_call(
        package_aliases: &BTreeMap<String, Vec<String>>,
        targets: &skiff_compiler_source::ResolvedCallTargetFacts,
    ) -> skiff_syntax::error::Result<FileIrUnit> {
        initialize_test_prelude();
        let source = package_call_source();
        let ast = parse_source(source)?;
        compile_parsed_source_file_ir_unit_with_lowering_context(
            ast,
            source,
            "internal/any_lowering.skiff",
            MODULE,
            "package",
            &SourceFileLoweringContext {
                package_aliases,
                resolved_call_targets: targets,
                ..SourceFileLoweringContext::none()
            },
        )
    }

    fn lower_local_function_call(
        targets: &skiff_compiler_source::ResolvedCallTargetFacts,
    ) -> skiff_syntax::error::Result<FileIrUnit> {
        initialize_test_prelude();
        let source = r#"
          function helper() -> string {
            return "ok"
          }

          function run() -> string {
            return helper()
          }
        "#;
        compile_parsed_source_file_ir_unit_with_lowering_context(
            parse_source(source)?,
            source,
            "internal/any_lowering.skiff",
            MODULE,
            "package",
            &SourceFileLoweringContext {
                resolved_call_targets: targets,
                ..SourceFileLoweringContext::none()
            },
        )
    }

    fn local_run_call_expression() -> skiff_compiler_source::ExpressionKey {
        skiff_compiler_source::ExpressionKey::new(
            MODULE,
            skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
            0,
        )
    }

    #[test]
    fn unresolved_local_function_does_not_fall_back_to_name_lookup() {
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                local_run_call_expression(),
                skiff_compiler_source::ResolvedCallTarget::Unknown {
                    reason: skiff_compiler_source::UnknownCallTargetReason::UnresolvedName,
                },
            )]));
        let error = lower_local_function_call(&targets)
            .expect_err("an unresolved local target must fail before File IR")
            .to_string();
        assert!(
            error.contains("callee `helper` is not resolved"),
            "unexpected unresolved target diagnostic: {error}"
        );
    }

    #[test]
    fn typed_local_function_index_mismatch_fails_before_file_ir() {
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                local_run_call_expression(),
                skiff_compiler_source::ResolvedCallTarget::LocalFunction {
                    source_callable: skiff_compiler_source::SourceSymbolKey::new(MODULE, "helper"),
                    executable_index: 99,
                },
            )]));
        let error = lower_local_function_call(&targets)
            .expect_err("a mutated executable index must fail before File IR")
            .to_string();
        assert!(
            error.contains("canonical local index is 0"),
            "unexpected executable index diagnostic: {error}"
        );
    }

    #[test]
    fn typed_package_call_site_lowers_by_expression_key_without_local_abi_witness() {
        let expression = package_call_expression();
        let expected_local_abi = PackageLocalAbiIdentity::new("local-abi:must-not-enter-call-site");
        let package_callable_id = PackageCallableId::new("callable:utils.format");
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                expression,
                skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
                    package_requirement_alias: "utils".to_string(),
                    compiler_owned: false,
                    package_callable_id: package_callable_id.clone(),
                    expected_local_abi: expected_local_abi.clone(),
                    exact_signature: None,
                },
            )]));
        let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
        let unit = lower_package_call(&package_aliases, &targets).unwrap();

        let run = executable(&unit, "run");
        assert!(
            run.may_suspend,
            "missing exact dependency signature must fail closed"
        );
        assert!(run.body.expressions.iter().any(|expression| matches!(
            expression,
            ExprIr::Call { call }
                if matches!(
                    &call.target,
                    CallTargetIr::PackageCallable {
                        package_ref: PackageRefIr::Dependency { dependency_ref },
                        package_callable_id: target_callable_id,
                    } if dependency_ref == "utils" && target_callable_id == &package_callable_id
                )
        )));
        assert_eq!(unit.external_refs.package_callables.len(), 1);
        assert_eq!(
            unit.external_refs.package_callables[0].package_callable_id,
            package_callable_id
        );
        assert_eq!(
            unit.external_refs.package_callables[0].package_ref,
            PackageRefIr::Dependency {
                dependency_ref: "utils".to_string(),
            }
        );
        let wire = serde_json::to_string(&unit).unwrap();
        assert!(wire.contains("packageCallableId"));
        assert!(!wire.contains(expected_local_abi.as_str()));
        assert!(!wire.contains("operationAbiId"));
        assert!(unit
            .file_ir_identity
            .starts_with("skiff-file-ir-v10:sha256:"));
    }

    #[test]
    fn dependency_exact_signature_controls_lowered_suspend_flag_without_synthetic_calls() {
        let lower = |may_suspend| {
            let targets =
                skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                    package_call_expression(),
                    skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
                        package_requirement_alias: "utils".to_string(),
                        compiler_owned: false,
                        package_callable_id: PackageCallableId::new("callable:utils.format"),
                        expected_local_abi: PackageLocalAbiIdentity::new("local-abi:utils"),
                        exact_signature: Some(skiff_artifact_model::PackageCallableSignature {
                            type_params: Vec::new(),
                            parameters: Vec::new(),
                            return_type: skiff_artifact_model::PackageTypeRef::Local {
                                local_type: TypeRefIr::builtin("void"),
                            },
                            may_suspend,
                        }),
                    },
                )]));
            lower_package_call(
                &BTreeMap::from([("utils".to_string(), vec![String::new()])]),
                &targets,
            )
            .unwrap()
        };
        let non_suspending = lower(false);
        let suspending = lower(true);
        let non_suspending_run = executable(&non_suspending, "run");
        let suspending_run = executable(&suspending, "run");

        assert!(!non_suspending_run.may_suspend);
        assert!(suspending_run.may_suspend);
        assert_eq!(
            non_suspending_run.body, suspending_run.body,
            "conservative suspension changes only the executable summary, not the call body"
        );
        assert_eq!(
            suspending_run
                .body
                .expressions
                .iter()
                .filter(|expression| matches!(expression, ExprIr::Call { .. }))
                .count(),
            1,
            "suspension inference must not inject a synthetic runtime call"
        );
    }

    #[test]
    fn known_package_call_without_typed_target_fails_closed() {
        let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
        let error = lower_package_call(
            &package_aliases,
            &skiff_compiler_source::ResolvedCallTargetFacts::empty(),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("package dependency call `utils/format`"));
        assert!(message.contains("missing ResolvedCallTargetFacts entry"));
    }

    #[test]
    fn known_package_call_with_unknown_typed_target_fails_closed() {
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                package_call_expression(),
                skiff_compiler_source::ResolvedCallTarget::Unknown {
                    reason: skiff_compiler_source::UnknownCallTargetReason::UnresolvedName,
                },
            )]));
        let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
        let error = lower_package_call(&package_aliases, &targets).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("package dependency call `utils/format`"));
        assert!(message.contains("Unknown(UnresolvedName)"));
    }

    #[test]
    fn package_call_target_alias_must_match_callee_root() {
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                package_call_expression(),
                skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
                    package_requirement_alias: "other".to_string(),
                    compiler_owned: false,
                    package_callable_id: PackageCallableId::new("callable:other.format"),
                    expected_local_abi: PackageLocalAbiIdentity::new("local-abi:other"),
                    exact_signature: None,
                },
            )]));
        let package_aliases = BTreeMap::from([
            ("other".to_string(), vec![String::new()]),
            ("utils".to_string(), vec![String::new()]),
        ]);
        let error = lower_package_call(&package_aliases, &targets).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("typed package target names dependency `other`"));
        assert!(message.contains("callee root is `utils`"));
    }
}
