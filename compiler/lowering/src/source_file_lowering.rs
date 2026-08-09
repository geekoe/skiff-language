use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::file_ir::{assign_file_ir_identity, FileIrUnit};
use skiff_artifact_model::{
    ActorAbiInput, ActorCreateImplementationIr, ActorCreateSignatureIr, ActorDeclarationIr,
    ActorFieldEncodingIr, ActorFieldIr, ActorImplementationIdentity, ActorPublicMethodIr,
    FunctionTypeParamIr, TypeRefIr, ACTOR_RUNTIME_ABI_VERSION_V1,
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
    ast::{ActorCreateDecl, ConstDecl, FunctionDecl, SourceFile},
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
    super::actor_method_validation::validate_actor_source_rules(ast)?;

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
    super::actor_method_validation::validate_actor_method_ir_rules(&unit)?;
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
        let attached_type = ast
            .types
            .iter()
            .find(|ty| ty.name == actor.name)
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "actor {} requires a same-file type declaration",
                    actor.name
                ))
            })?;
        let fields = attached_type
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
        if actor.create.is_none() && fields.iter().any(|field| field.name != actor.key_field) {
            return Err(CompileError::Semantic(format!(
                "actor {} must declare create(...) because attached type has non-key fields",
                actor.name
            )));
        }
        let actor_id_type = fields
            .iter()
            .find(|field| field.name == actor.key_field)
            .map(|field| field.ty.clone())
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "actor {} key({}) must name a field of the attached type",
                    actor.name, actor.key_field
                ))
            })?;
        validate_actor_key_type(&actor_id_type, &actor.name)?;

        let create_candidates = ast
            .impls
            .iter()
            .filter(|implementation| implementation.target == actor.name)
            .flat_map(|implementation| implementation.method_bodies.iter())
            .filter(|method| !method.is_static && method.name == "create")
            .collect::<Vec<_>>();
        if let Some(create_decl) = actor.create.as_ref() {
            if create_candidates.is_empty() {
                return Err(CompileError::Semantic(format!(
                    "actor {} declares create(...) but its impl has no create method",
                    actor.name
                )));
            }
            if create_candidates.len() > 1 {
                return Err(CompileError::Semantic(format!(
                    "actor {} impl declares create more than once",
                    actor.name
                )));
            }
            validate_actor_create_signature(actor, create_decl, create_candidates[0])?;
        } else if !create_candidates.is_empty() {
            return Err(CompileError::Semantic(format!(
                "actor {} omits create(...) but its impl declares a create method; \
                 create is required when the attached type has non-key fields",
                actor.name
            )));
        }
        let create_implementation = create_candidates
            .first()
            .map(|method| {
                let declaration_name = impl_method_declaration_name(&actor.name, &method.name);
                let executable_index = executable_index
                    .entry(&declaration_name)
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "missing semantic executable index for Actor create `{declaration_name}`"
                        ))
                    })?
                    .executable_index;
                let identity = skiff_artifact_identity::actor_method_identity(
                    module_path,
                    &actor.name,
                    &method.name,
                )
                .map_err(|error| CompileError::Semantic(error.to_string()))?;
                Ok(ActorCreateImplementationIr {
                    identity,
                    executable_index,
                })
            })
            .transpose()?;

        let actor_methods = ast
            .impls
            .iter()
            .filter(|implementation| implementation.target == actor.name)
            .flat_map(|implementation| implementation.method_bodies.iter())
            .filter(|method| !method.is_static && method.name != "create")
            .map(|method| {
                if method
                    .params
                    .iter()
                    .any(|parameter| parameter.mode == skiff_syntax::ast::ParamMode::InOut)
                {
                    return Err(CompileError::Semantic(format!(
                        "inout is not allowed on actor external method `{}.{}`; actor ABI boundaries materialize values",
                        actor.name, method.name
                    )));
                }
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
            key_field: actor.key_field.clone(),
            fields,
            create: actor
                .create
                .as_ref()
                .map(|create| {
                    Ok(ActorCreateSignatureIr {
                        parameters: create
                            .params
                            .iter()
                            .map(|param| {
                                Ok(FunctionTypeParamIr {
                                    name: param.name.clone(),
                                    ty: crate::type_lowering::lower_type_ref(
                                        &param.ty,
                                        type_indices,
                                        local_db_objects,
                                        publication_db_metadata,
                                        package_aliases,
                                        external_type_symbols,
                                        source_alias_targets,
                                        crate::type_lowering::TypeLoweringContext::value(),
                                    )?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                    })
                })
                .transpose()?,
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
            create_implementation,
        });
    }
    Ok(())
}

fn validate_actor_create_signature(
    actor: &skiff_syntax::ast::ActorDecl,
    declaration: &ActorCreateDecl,
    method: &FunctionDecl,
) -> Result<()> {
    let mut impl_params = method.params.iter();
    if method.implicit_self.is_none() {
        impl_params.next();
    }
    let impl_params = impl_params.collect::<Vec<_>>();
    if impl_params.len() != declaration.params.len() {
        return Err(CompileError::Semantic(format!(
            "actor {} create(...) declaration has {} parameter(s) but impl create has {}",
            actor.name,
            declaration.params.len(),
            impl_params.len()
        )));
    }
    for (impl_param, declared) in impl_params.iter().zip(&declaration.params) {
        if impl_param.name != declared.name || impl_param.ty.name != declared.ty.name {
            return Err(CompileError::Semantic(format!(
                "actor {} create(...) declaration parameter `{}: {}` does not match impl parameter `{}: {}`",
                actor.name,
                declared.name,
                declared.ty.name,
                impl_param.name,
                impl_param.ty.name
            )));
        }
    }
    if method.return_type.name != "void" {
        return Err(CompileError::Semantic(format!(
            "actor {} create must return void",
            actor.name
        )));
    }
    Ok(())
}

fn validate_actor_key_type(ty: &TypeRefIr, actor_name: &str) -> Result<()> {
    let unsupported = match ty {
        TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
            Some("interface or function")
        }
        TypeRefIr::Builtin { name, .. }
            if matches!(name.as_str(), "unknown" | "void" | "never") =>
        {
            Some(name.as_str())
        }
        _ => None,
    };
    if let Some(reason) = unsupported {
        return Err(CompileError::Semantic(format!(
            "actor {} key field type must support stable canonical encoding; {reason} does not",
            actor_name
        )));
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
mod tests;
