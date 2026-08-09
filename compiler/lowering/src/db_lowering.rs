use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{
    BlockIr, CallIr, CallTargetIr, DbBlockModeIr, DbBodyIr, DbChangeIr, DbChangeOpIr,
    DbDeclarationIr, DbFieldStorageIr, DbIndexDirectionIr, DbIndexFieldIr, DbIndexIr,
    DbLeaseClaimIr, DbLeaseIr, DbLeaseReadIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr,
    DbOpKindIr, DbOperationIr, DbOrderEntryIr, DbPredicateCompareOpIr, DbPredicateIr,
    DbProjectionIr, DbQueryIr, DbQueryValueIr, DbRetentionIr, DbRetentionUnitIr, DbSelectorIr,
    DbTargetIr, DbTransactionIr, ExprIr, ExprRefIr, FieldPathIr, FileIrUnit, FunctionTypeParamIr,
    InstructionSourceSite, LiteralIr, MetadataValue, ServiceSymbolRef, SlotKind, StmtIr,
    SyntheticInstructionSiteReason, TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_model::{
    NamedUnionBranchIr, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef,
};
use skiff_compiler_core::db_projection::project_db_read_type;
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref;
use skiff_compiler_source::{
    semantic::DbAttachmentIndex, LocalDbObjectIndex, PublicationDbMetadata,
    PublicationDbMetadataIndex, PublicationTypeSymbolIndex, SourceSymbolKey,
};
use skiff_syntax::{
    ast::{
        BinaryOp, CallArg, DbBlockMode, DbBody, DbChange, DbChangeOp, DbDecl, DbDeclKind,
        DbIndexDirection, DbLeaseClaim, DbLeaseRead, DbOperation, DbOperationKind, DbQuery,
        DbQueryBlock, DbRetentionUnit, DbSelector, DbStorageCodec, DbWhereClause, Expr, FieldPath,
        Stmt, TypeRef, UnaryOp,
    },
    ast_utils::db_collection_name,
    error::{CompileError, Result},
    type_syntax::split_top_level,
};

use super::{
    function_lowering::{block_contains_return_stmt, BindingReadonlyFlags, FunctionLowerer},
    source_unit_lowering::{push_source_span, source_span_ref},
    type_lowering::{
        db_object_type_ref, lower_type_ref, lower_type_text, type_ref_ir_type_text,
        TypeLoweringContext,
    },
};

#[derive(Debug, Clone)]
pub(super) struct DbMetadataIr {
    pub(super) type_ref: TypeRefIr,
    pub(super) type_name: String,
    pub(super) canonical_type_name: String,
    pub(super) kind: DbObjectKindIr,
    pub(super) collection_name: Option<String>,
    pub(super) retention: Option<DbRetentionIr>,
    pub(super) leases: BTreeMap<String, DbLeaseIr>,
    pub(super) key: DbObjectKeyIr,
    pub(super) fields: BTreeSet<String>,
    pub(super) field_types: BTreeMap<String, TypeRefIr>,
    pub(super) field_type_texts: BTreeMap<String, String>,
    pub(super) field_storage: BTreeMap<String, DbFieldStorageIr>,
}

impl DbMetadataIr {
    fn storage_for_top_level_field(&self, name: &str) -> DbFieldStorageIr {
        self.field_storage.get(name).copied().unwrap_or_default()
    }

    fn validate_storage_field_use(
        &self,
        path: &[String],
        use_case: DbFieldUse,
        selector_kind: DbSelectorKind,
    ) -> Result<()> {
        let Some(field) = path.first() else {
            return Ok(());
        };
        match self.storage_for_top_level_field(field) {
            DbFieldStorageIr::Identity => Ok(()),
            DbFieldStorageIr::Encrypted => {
                let top_level = path.len() == 1;
                let allowed = match use_case {
                    DbFieldUse::Projection => top_level,
                    DbFieldUse::WholeSet => {
                        top_level && selector_kind == DbSelectorKind::KnownRecordKey
                    }
                    DbFieldUse::Predicate
                    | DbFieldUse::Order
                    | DbFieldUse::Index
                    | DbFieldUse::PartialChange => false,
                };
                if allowed {
                    return Ok(());
                }
                Err(CompileError::Semantic(format!(
                    "db object {} encrypted storage field `{field}` cannot be used for {}{}",
                    self.type_name,
                    use_case.label(),
                    if use_case == DbFieldUse::WholeSet {
                        " without a key selector"
                    } else {
                        ""
                    }
                )))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct LoweredPackageDbMetadataIndex {
    by_source_key: BTreeMap<SourceSymbolKey, DbMetadataIr>,
    by_bare_name: BTreeMap<String, BTreeSet<SourceSymbolKey>>,
}

impl LoweredPackageDbMetadataIndex {
    pub(super) fn from_source_index(
        index: &PublicationDbMetadataIndex,
        package_aliases: &BTreeMap<String, Vec<String>>,
        external_type_symbols: &PublicationTypeSymbolIndex,
    ) -> Result<Self> {
        let mut lowered = Self::default();
        for (source_key, metadata) in index.entries() {
            lowered.insert(
                source_key.clone(),
                lower_publication_db_metadata(metadata, package_aliases, external_type_symbols)?,
            );
        }
        Ok(lowered)
    }

    fn insert(&mut self, source_key: SourceSymbolKey, metadata: DbMetadataIr) {
        self.by_bare_name
            .entry(source_key.symbol().to_string())
            .or_default()
            .insert(source_key.clone());
        self.by_source_key.insert(source_key, metadata);
    }

    pub fn resolve_qualified(&self, name: &str) -> Option<&DbMetadataIr> {
        source_symbol_key_from_qualified_text(name)
            .and_then(|source_key| self.by_source_key.get(&source_key))
    }

    pub fn resolve_bare(&self, name: &str) -> Result<Option<&DbMetadataIr>> {
        let Some(candidates) = self.by_bare_name.get(name) else {
            return Ok(None);
        };
        let matches = candidates
            .iter()
            .filter_map(|candidate| self.by_source_key.get(candidate))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [metadata] => Ok(Some(metadata)),
            _ => Err(CompileError::Semantic(format!(
                "db operation target `{name}` is ambiguous across publication db objects: {}",
                matches
                    .iter()
                    .map(|metadata| metadata.canonical_type_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DbBodyValidationMode {
    Insert,
    ReplaceByKey,
    ReplaceByQuery,
    UpsertByKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DbSelectorKind {
    KnownRecordKey,
    UnknownRecordKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbFieldUse {
    Projection,
    Predicate,
    Order,
    Index,
    WholeSet,
    PartialChange,
}

impl DbFieldUse {
    fn label(self) -> &'static str {
        match self {
            Self::Projection => "nested projection",
            Self::Predicate => "predicate",
            Self::Order => "order",
            Self::Index => "index",
            Self::WholeSet => "whole-field set",
            Self::PartialChange => "partial change",
        }
    }
}

fn lower_publication_db_metadata(
    metadata: &PublicationDbMetadata,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
) -> Result<DbMetadataIr> {
    let type_ref = metadata.canonical_type_ref.clone().unwrap_or_else(|| {
        db_object_type_ref(ServiceSymbolRef {
            module_path: metadata.module_path.clone(),
            symbol: metadata.type_name.clone(),
        })
    });
    let empty_local_db_objects = LocalDbObjectIndex::default();
    let empty_publication_db_metadata = PublicationDbMetadataIndex::default();
    let source_alias_targets = BTreeMap::new();
    let key = DbObjectKeyIr {
        name: metadata.key.name.clone(),
        ty: match &metadata.canonical_key_type {
            Some(ty) => ty.clone(),
            None => lower_type_ref(
                &metadata.key.ty,
                &BTreeMap::new(),
                &empty_local_db_objects,
                &empty_publication_db_metadata,
                package_aliases,
                external_type_symbols,
                &source_alias_targets,
                TypeLoweringContext::value(),
            )?,
        },
    };
    let mut field_types = BTreeMap::new();
    let mut field_type_texts = BTreeMap::new();
    field_types.insert(key.name.clone(), key.ty.clone());
    field_type_texts.insert(metadata.key.name.clone(), metadata.key.ty.name.clone());
    for (field_name, field_ty) in &metadata.field_types {
        field_types.insert(
            field_name.clone(),
            match metadata.canonical_field_types.get(field_name) {
                Some(ty) => ty.clone(),
                None => lower_type_ref(
                    field_ty,
                    &BTreeMap::new(),
                    &empty_local_db_objects,
                    &empty_publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    &source_alias_targets,
                    TypeLoweringContext::value(),
                )?,
            },
        );
        field_type_texts.insert(field_name.clone(), field_ty.name.clone());
    }
    let retention = metadata.retention.as_ref().map(|retention| DbRetentionIr {
        amount: retention.amount,
        unit: match retention.unit {
            DbRetentionUnit::Days => DbRetentionUnitIr::Days,
            DbRetentionUnit::Hours => DbRetentionUnitIr::Hours,
            DbRetentionUnit::Minutes => DbRetentionUnitIr::Minutes,
            DbRetentionUnit::Seconds => DbRetentionUnitIr::Seconds,
        },
    });
    let leases = metadata
        .leases
        .values()
        .map(|lease| {
            (
                lease.name.clone(),
                DbLeaseIr {
                    name: lease.name.clone(),
                    ttl_ms: lease.ttl_ms,
                    max_ms: lease.max_ms,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(DbMetadataIr {
        type_ref,
        type_name: metadata.type_name.clone(),
        canonical_type_name: metadata.canonical_type_name.clone(),
        kind: metadata.kind,
        collection_name: metadata.collection_name.clone(),
        retention,
        leases,
        key,
        fields: metadata.fields.clone(),
        field_types,
        field_type_texts: metadata.field_type_texts.clone(),
        field_storage: metadata
            .field_storage
            .iter()
            .map(|(field, codec)| (field.clone(), lower_db_storage_codec(*codec)))
            .collect(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_db_declarations(
    db_attachments: &DbAttachmentIndex<'_>,
    type_indices: &BTreeMap<String, u32>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    source_alias_targets: &BTreeMap<String, String>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<BTreeMap<String, DbMetadataIr>> {
    let mut metadata = BTreeMap::new();
    for attachment in db_attachments.iter() {
        let db = attachment.db;
        let key_field = attachment.key;
        let type_ref =
            db_object_type_ref(local_db_objects.resolve(&db.name).unwrap_or_else(|| {
                ServiceSymbolRef {
                    module_path: attachment.module_path.to_string(),
                    symbol: db.name.clone(),
                }
            }));
        let source_span = source_span_ref(db.span);
        let key = DbObjectKeyIr {
            name: key_field.name.clone(),
            ty: db_storage_type_ref(
                lower_type_ref(
                    &key_field.ty,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    TypeLoweringContext::value(),
                )?,
                unit,
            )?,
        };
        let mut type_fields = BTreeMap::new();
        let mut identity_fields = BTreeMap::new();
        let mut field_type_texts = BTreeMap::new();
        debug_assert!(attachment
            .field_map()
            .contains_key(attachment.key.name.as_str()));
        type_fields.insert(key.name.clone(), key.ty.clone());
        identity_fields.insert(
            key.name.clone(),
            lower_type_ref(
                &key_field.ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                TypeLoweringContext::value(),
            )?,
        );
        field_type_texts.insert(key_field.name.clone(), key_field.ty.name.clone());
        for field in attachment.fields() {
            field_type_texts.insert(field.name.clone(), field.ty.name.clone());
            let lowered = lower_type_ref(
                &field.ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                TypeLoweringContext::value(),
            )?;
            identity_fields.insert(field.name.clone(), lowered.clone());
            let field_ty = db_storage_type_ref(lowered, unit)?;
            type_fields.insert(field.name.clone(), field_ty);
        }
        let field_types = type_fields.clone();
        let kind = match db.kind {
            DbDeclKind::Object => DbObjectKindIr::Object,
            DbDeclKind::Contract => DbObjectKindIr::Contract,
        };
        let implements = match (&db.implements, db.kind) {
            (Some(implements), DbDeclKind::Object) => {
                let contract = resolve_implements_contract(
                    implements,
                    package_aliases,
                    publication_db_metadata,
                )?;
                if contract.kind != DbObjectKindIr::Contract {
                    return Err(CompileError::Semantic(format!(
                        "db object {} implements target `{}` which resolves to db object {}, not a db contract; the implementing declaration must reference a `db contract` attached type",
                        db.name, implements.name, contract.canonical_type_name
                    )));
                }
                if contract.canonical_type_ref.is_none() {
                    return Err(CompileError::Semantic(format!(
                        "db object {} implements target `{}` is not a cross-package contract reference; contracts are declared in dependency packages",
                        db.name, implements.name
                    )));
                }
                validate_implements_coverage(
                    db,
                    contract,
                    &identity_fields,
                    attachment.storage_map(),
                )?;
                Some(lower_type_ref(
                    implements,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    TypeLoweringContext::value(),
                )?)
            }
            _ => None,
        };
        let collection_name = if db.kind == DbDeclKind::Object {
            let collection_name = db_collection_name(db);
            validate_db_collection_name(&collection_name, &db.name)?;
            Some(collection_name)
        } else {
            None
        };
        let retention = db.retention.as_ref().map(|retention| DbRetentionIr {
            amount: retention.amount,
            unit: match retention.unit {
                DbRetentionUnit::Days => DbRetentionUnitIr::Days,
                DbRetentionUnit::Hours => DbRetentionUnitIr::Hours,
                DbRetentionUnit::Minutes => DbRetentionUnitIr::Minutes,
                DbRetentionUnit::Seconds => DbRetentionUnitIr::Seconds,
            },
        });
        let mut lease_names = BTreeSet::new();
        let leases = db
            .leases
            .iter()
            .map(|lease| {
                if !lease_names.insert(lease.name.clone()) {
                    return Err(CompileError::Semantic(format!(
                        "db object {} declares lease `{}` more than once",
                        db.name, lease.name
                    )));
                }
                Ok(DbLeaseIr {
                    name: lease.name.clone(),
                    ttl_ms: lease.ttl_ms,
                    max_ms: lease.max_ms,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let lease_map = leases
            .iter()
            .map(|lease| (lease.name.clone(), lease.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut db_field_names = BTreeSet::new();
        db_field_names.insert(key.name.clone());
        let fields = attachment
            .fields()
            .map(|field| {
                db_field_names.insert(field.name.clone());
                Ok(DbObjectFieldIr {
                    name: field.name.clone(),
                    ty: db_storage_type_ref(
                        lower_type_ref(
                            &field.ty,
                            type_indices,
                            local_db_objects,
                            publication_db_metadata,
                            package_aliases,
                            external_type_symbols,
                            source_alias_targets,
                            TypeLoweringContext::value(),
                        )?,
                        unit,
                    )?,
                    storage: attachment
                        .storage_map()
                        .get(&field.name)
                        .copied()
                        .map(lower_db_storage_codec)
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let lowered_metadata = DbMetadataIr {
            type_ref: type_ref.clone(),
            type_name: db.name.clone(),
            canonical_type_name: canonical_db_type_name(attachment.module_path, &db.name),
            kind,
            collection_name: collection_name.clone(),
            retention: retention.clone(),
            leases: lease_map,
            key: key.clone(),
            fields: db_field_names,
            field_types,
            field_type_texts,
            field_storage: attachment
                .storage_map()
                .iter()
                .map(|(field, codec)| (field.clone(), lower_db_storage_codec(*codec)))
                .collect(),
        };
        let indexes = db
            .indexes
            .iter()
            .map(|index| {
                for field in &index.fields {
                    lowered_metadata.validate_storage_field_use(
                        &field.field_path,
                        DbFieldUse::Index,
                        DbSelectorKind::UnknownRecordKey,
                    )?;
                }
                Ok(DbIndexIr {
                    name: index.name.clone(),
                    unique: index.unique,
                    fields: index
                        .fields
                        .iter()
                        .map(|field| DbIndexFieldIr {
                            field: field_path_ir(&field.field_path),
                            direction: match field.direction {
                                DbIndexDirection::Asc => DbIndexDirectionIr::Asc,
                                DbIndexDirection::Desc => DbIndexDirectionIr::Desc,
                            },
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        unit.declarations.db.insert(
            db.name.clone(),
            DbDeclarationIr {
                type_ref: type_ref.clone(),
                type_name: db.name.clone(),
                collection_name: collection_name.clone(),
                implements: implements.clone(),
                identity_fields: if db.kind == DbDeclKind::Contract {
                    identity_fields.clone()
                } else {
                    BTreeMap::new()
                },
                kind,
                key: key.clone(),
                fields,
                retention: retention.clone(),
                leases: leases.clone(),
                indexes,
                source_span: Some(source_span.clone()),
            },
        );
        metadata.insert(db.name.clone(), lowered_metadata);
        push_source_span(
            &mut unit.source_map.spans,
            next_span_id,
            "db",
            &db.name,
            db.span,
        );
    }
    Ok(metadata)
}

/// Resolves a `db object ... implements <contract-ref>` target to the contract
/// declaration facts of a direct dependency package. Contract references are
/// cross-package type references (Phase 0 dual spelling): the alias decides the
/// dependency view, and the remainder is the package symbol path. Same-package
/// references are rejected because the identity comparison below cannot be
/// sound without the contract package's own lowered File IR.
fn resolve_implements_contract<'a>(
    implements: &TypeRef,
    package_aliases: &BTreeMap<String, Vec<String>>,
    publication_db_metadata: &'a PublicationDbMetadataIndex,
) -> Result<&'a PublicationDbMetadata> {
    let name = implements
        .name
        .trim()
        .strip_prefix("root.")
        .unwrap_or_else(|| implements.name.trim());
    let mut lookup_name = String::new();
    let target = if name.contains('/') {
        name
    } else if let Some((alias, rest)) = name.split_once('.') {
        if !package_aliases.contains_key(alias) {
            return Err(CompileError::Semantic(format!(
                "db object implements target `{}` must be a cross-package contract reference (for example `engine.package.Type` or `engine/package.Type`)",
                implements.name
            )));
        }
        lookup_name = format!("{alias}/{rest}");
        lookup_name.as_str()
    } else {
        return Err(CompileError::Semantic(format!(
            "db object implements target `{}` must be a cross-package contract reference (for example `engine.package.Type` or `engine/package.Type`)",
            implements.name
        )));
    };
    publication_db_metadata
        .resolve_qualified(target)
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "db object implements target `{target}` does not resolve to a db contract declaration in a dependency package"
            ))
        })
}

/// Compile-time contract coverage: the implementing db object must declare
/// every contract field with the same schema identity and the same storage
/// mapping, and the same primary key field and type.
fn validate_implements_coverage(
    db: &DbDecl,
    contract: &PublicationDbMetadata,
    type_fields: &BTreeMap<String, TypeRefIr>,
    storage_map: &BTreeMap<String, DbStorageCodec>,
) -> Result<()> {
    let missing = contract
        .fields
        .iter()
        .filter(|field| !type_fields.contains_key(*field))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(CompileError::Semantic(format!(
            "db object {} implements contract {} but its type is missing contract fields: {}",
            db.name,
            contract.canonical_type_name,
            missing.join(", ")
        )));
    }
    for field in &contract.fields {
        let contract_ty = if *field == contract.key.name {
            contract.canonical_key_type.as_ref()
        } else {
            contract.canonical_field_types.get(field)
        }
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "db object {} implements contract {} but contract field {} has no canonical type fact",
                db.name, contract.canonical_type_name, field
            ))
        })?;
        let host_ty = type_fields
            .get(field)
            .expect("covered contract field has a host type");
        if !db_field_type_identity_matches(host_ty, contract_ty) {
            return Err(CompileError::Semantic(format!(
                "db object {} implements contract {} but field {} has a different schema identity: host `{}` vs contract `{}`",
                db.name,
                contract.canonical_type_name,
                field,
                type_ref_ir_type_text(host_ty),
                type_ref_ir_type_text(contract_ty)
            )));
        }
        if contract.field_storage.get(field) != storage_map.get(field) {
            return Err(CompileError::Semantic(format!(
                "db object {} implements contract {} but field {} has a different storage mapping than the contract",
                db.name, contract.canonical_type_name, field
            )));
        }
    }
    Ok(())
}

/// Schema identity equality for one stored field, decided by normalized
/// nominal identity. The contract facts carry the contract's own symbol
/// references resolved into the host's dependency view (same dependency alias
/// + symbol path as the host's cross-package references), so a host reference
/// to a contract-package symbol matches the contract's own symbol while host
/// local nominals (LocalType / PublicationType on either side) never match a
/// contract symbol. Forms without nominal identity (builtin, anonymous record
/// / union / literal, applied nominals whose base resolves nominally) compare
/// structurally; everything else is false.
fn db_field_type_identity_matches(host: &TypeRefIr, contract: &TypeRefIr) -> bool {
    match (host, contract) {
        (
            TypeRefIr::PackageSymbol {
                symbol: host_symbol,
            },
            TypeRefIr::PackageSymbol {
                symbol: contract_symbol,
            },
        ) => package_symbol_identity_matches(host_symbol, contract_symbol),
        (
            TypeRefIr::Builtin { name, args },
            TypeRefIr::Builtin {
                name: other,
                args: other_args,
            },
        ) => {
            name == other
                && args.len() == other_args.len()
                && args
                    .iter()
                    .zip(other_args)
                    .all(|(arg, other)| db_field_type_identity_matches(arg, other))
        }
        (TypeRefIr::Record { fields }, TypeRefIr::Record { fields: other }) => {
            fields.len() == other.len()
                && fields.iter().all(|(name, ty)| {
                    other
                        .get(name)
                        .is_some_and(|other_ty| db_field_type_identity_matches(ty, other_ty))
                })
        }
        (TypeRefIr::Union { items }, TypeRefIr::Union { items: other }) => {
            items.len() == other.len()
                && items
                    .iter()
                    .zip(other)
                    .all(|(item, other)| db_field_type_identity_matches(item, other))
        }
        (TypeRefIr::Nullable { inner }, TypeRefIr::Nullable { inner: other }) => {
            db_field_type_identity_matches(inner, other)
        }
        (TypeRefIr::Literal { value }, TypeRefIr::Literal { value: other }) => value == other,
        (
            TypeRefIr::AppliedNominal { base, arguments },
            TypeRefIr::AppliedNominal {
                base: other_base,
                arguments: other_arguments,
            },
        ) => {
            nominal_base_identity_matches(base, other_base)
                && arguments.len() == other_arguments.len()
                && arguments
                    .iter()
                    .zip(other_arguments)
                    .all(|(argument, other)| db_field_type_identity_matches(argument, other))
        }
        _ => false,
    }
}

fn nominal_base_identity_matches(
    host: &NominalTypeRefBaseIr,
    contract: &NominalTypeRefBaseIr,
) -> bool {
    match (host, contract) {
        (
            NominalTypeRefBaseIr::PackageSymbol {
                symbol: host_symbol,
            },
            NominalTypeRefBaseIr::PackageSymbol {
                symbol: contract_symbol,
            },
        ) => package_symbol_identity_matches(host_symbol, contract_symbol),
        _ => false,
    }
}

/// Cross-package nominal identity: the same dependency view (dependency alias,
/// the link-time ABI expectation is a linker refinement and not part of the
/// identity) and the same public symbol path.
fn package_symbol_identity_matches(host: &PackageSymbolRef, contract: &PackageSymbolRef) -> bool {
    matches!(
        (&host.package, &contract.package),
        (
            PackageRefIr::Dependency {
                dependency_ref: host_dependency
            },
            PackageRefIr::Dependency {
                dependency_ref: contract_dependency
            }
        ) if host_dependency == contract_dependency
    ) && host.symbol_path == contract.symbol_path
}

fn lower_db_storage_codec(codec: DbStorageCodec) -> DbFieldStorageIr {
    match codec {
        DbStorageCodec::Encrypted => DbFieldStorageIr::Encrypted,
    }
}

fn db_storage_type_ref(ty: TypeRefIr, unit: &FileIrUnit) -> Result<TypeRefIr> {
    expand_db_storage_type_ref(&ty, unit, &mut BTreeSet::new())
}

fn expand_db_storage_type_ref(
    ty: &TypeRefIr,
    unit: &FileIrUnit,
    seen_local_types: &mut BTreeSet<u32>,
) -> Result<TypeRefIr> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            if !seen_local_types.insert(*type_index) {
                return Ok(ty.clone());
            }
            let Some(decl) = unit.type_table.get(*type_index as usize) else {
                return Err(CompileError::Semantic(format!(
                    "missing local type index {type_index} while lowering db storage type"
                )));
            };
            let expanded = match &decl.descriptor {
                TypeDescriptorIr::Record { fields } => TypeRefIr::Record {
                    fields: fields
                        .iter()
                        .map(|(name, ty)| {
                            Ok((
                                name.clone(),
                                expand_db_storage_type_ref(ty, unit, seen_local_types)?,
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>>>()?,
                },
                TypeDescriptorIr::Alias { target } => {
                    expand_db_storage_type_ref(target, unit, seen_local_types)?
                }
                TypeDescriptorIr::Representation { representation } => {
                    expand_db_storage_type_ref(representation, unit, seen_local_types)?
                }
                TypeDescriptorIr::Union { branches } => TypeRefIr::Union {
                    items: branches
                        .iter()
                        .map(|branch| {
                            let branch_type = match branch {
                                NamedUnionBranchIr::ConcreteNominal { nominal_type, .. } => {
                                    nominal_type
                                }
                                NamedUnionBranchIr::SyntheticDiscriminator {
                                    payload_type, ..
                                } => payload_type,
                                NamedUnionBranchIr::Literal { value } => {
                                    return Ok(TypeRefIr::Literal {
                                        value: value.clone(),
                                    });
                                }
                            };
                            expand_db_storage_type_ref(branch_type, unit, seen_local_types)
                        })
                        .collect::<Result<Vec<_>>>()?,
                },
                TypeDescriptorIr::Interface => {
                    return Err(CompileError::Semantic(format!(
                        "interface type `{}` cannot be used as db storage",
                        decl.name
                    )));
                }
            };
            seen_local_types.remove(type_index);
            Ok(expanded)
        }
        TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        expand_db_storage_type_ref(ty, unit, seen_local_types)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        }),
        TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| expand_db_storage_type_ref(arg, unit, seen_local_types))
                .collect::<Result<Vec<_>>>()?,
        }),
        TypeRefIr::AppliedNominal { base, arguments } => {
            let NominalTypeRefBaseIr::LocalType { type_index } = base else {
                return Ok(TypeRefIr::AppliedNominal {
                    base: base.clone(),
                    arguments: arguments
                        .iter()
                        .map(|argument| {
                            expand_db_storage_type_ref(argument, unit, seen_local_types)
                        })
                        .collect::<Result<Vec<_>>>()?,
                });
            };
            let Some(decl) = unit.type_table.get(*type_index as usize) else {
                return Err(CompileError::Semantic(format!(
                    "missing local type index {type_index} while lowering applied db storage type"
                )));
            };
            if decl.type_params.len() != arguments.len() {
                return Err(CompileError::Semantic(format!(
                    "db storage type `{}` expects {} type arguments, found {}",
                    decl.name,
                    decl.type_params.len(),
                    arguments.len()
                )));
            }
            if !seen_local_types.insert(*type_index) {
                return Ok(ty.clone());
            }
            let substitutions = decl
                .type_params
                .iter()
                .cloned()
                .zip(arguments.iter().cloned())
                .collect::<BTreeMap<_, _>>();
            let expand = |ty: &TypeRefIr, seen: &mut BTreeSet<u32>| {
                let substituted = substitute_type_params_in_type_ref_ref(ty, &substitutions);
                expand_db_storage_type_ref(&substituted, unit, seen)
            };
            let expanded = match &decl.descriptor {
                TypeDescriptorIr::Record { fields } => TypeRefIr::Record {
                    fields: fields
                        .iter()
                        .map(|(name, ty)| Ok((name.clone(), expand(ty, seen_local_types)?)))
                        .collect::<Result<BTreeMap<_, _>>>()?,
                },
                TypeDescriptorIr::Alias { target } => expand(target, seen_local_types)?,
                TypeDescriptorIr::Representation { representation } => {
                    expand(representation, seen_local_types)?
                }
                TypeDescriptorIr::Union { branches } => TypeRefIr::Union {
                    items: branches
                        .iter()
                        .map(|branch| match branch {
                            NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                                expand(nominal_type, seen_local_types)
                            }
                            NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                                expand(payload_type, seen_local_types)
                            }
                            NamedUnionBranchIr::Literal { value } => Ok(TypeRefIr::Literal {
                                value: value.clone(),
                            }),
                        })
                        .collect::<Result<Vec<_>>>()?,
                },
                TypeDescriptorIr::Interface => {
                    return Err(CompileError::Semantic(format!(
                        "interface type `{}` cannot be used as db storage",
                        decl.name
                    )));
                }
            };
            seen_local_types.remove(type_index);
            Ok(expanded)
        }
        TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(expand_db_storage_type_ref(inner, unit, seen_local_types)?),
        }),
        TypeRefIr::AnyInterface { interface } => Ok(TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| expand_db_storage_type_ref(arg, unit, seen_local_types))
                    .collect::<Result<Vec<_>>>()?,
            },
        }),
        TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| expand_db_storage_type_ref(item, unit, seen_local_types))
                .collect::<Result<Vec<_>>>()?,
        }),
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: expand_db_storage_type_ref(&param.ty, unit, seen_local_types)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(expand_db_storage_type_ref(
                return_type,
                unit,
                seen_local_types,
            )?),
        }),
        TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::Literal { .. } => Ok(ty.clone()),
    }
}

fn validate_db_collection_name(collection_name: &str, db_name: &str) -> Result<()> {
    if collection_name.starts_with("_skiff_") {
        return Err(CompileError::Semantic(format!(
            "db object {db_name} collection name {collection_name:?} uses reserved _skiff_ system namespace"
        )));
    }
    Ok(())
}

pub(super) fn field_path_ir(path: &[String]) -> FieldPathIr {
    FieldPathIr {
        text: path.join("."),
        segments: path.to_vec(),
    }
}

pub(super) fn db_field_path_ir(path: &FieldPath) -> FieldPathIr {
    FieldPathIr {
        text: path.text.clone(),
        segments: path.segments.clone(),
    }
}

fn is_db_read_operation(operation: &DbOperation) -> bool {
    matches!(
        operation.op,
        DbOperationKind::Find | DbOperationKind::Optional | DbOperationKind::Require
    )
}

pub(super) fn is_db_readonly_result_operation(operation: &DbOperation) -> bool {
    is_db_read_operation(operation)
        || matches!(
            operation.op,
            DbOperationKind::Insert
                | DbOperationKind::Update
                | DbOperationKind::Replace
                | DbOperationKind::Upsert
        ) && !operation.many
}

/// Best-effort DB operation typing for callers that do not have DB metadata.
///
/// A projected read has a structural result type derived from DB field metadata,
/// so using the nominal target here would invent a wider type than the expression
/// actually returns.
pub(super) fn db_operation_result_type_text_without_metadata(
    operation: &DbOperation,
) -> Option<String> {
    operation
        .projection
        .is_none()
        .then(|| db_operation_result_type_text(operation, None, None))
}

fn db_operation_result_type_text(
    operation: &DbOperation,
    projection: Option<&DbProjectionIr>,
    db: Option<&DbMetadataIr>,
) -> String {
    let read_target = db
        .map(|db| db_read_result_type_text(db, projection))
        .unwrap_or_else(|| operation.target.name.clone());
    let write_target = db
        .map(db_full_result_type_text)
        .unwrap_or_else(|| operation.target.name.clone());
    match operation.op {
        DbOperationKind::Find if operation.many => format!("Array<{read_target}>"),
        DbOperationKind::Find | DbOperationKind::Optional => format!("{read_target}?"),
        DbOperationKind::Insert if operation.many => "DbInsertManyResult".to_string(),
        DbOperationKind::Update if operation.many => "DbUpdateManyResult".to_string(),
        DbOperationKind::Delete if operation.many => "DbDeleteManyResult".to_string(),
        DbOperationKind::Require => read_target,
        DbOperationKind::Insert => write_target,
        DbOperationKind::Update | DbOperationKind::Replace => format!("{write_target}?"),
        DbOperationKind::Upsert => format!("DbUpsertResult<{write_target}>"),
        DbOperationKind::Delete | DbOperationKind::Exists => "bool".to_string(),
        DbOperationKind::Count => "number".to_string(),
    }
}

pub(super) fn db_operation_result_type_ir(
    operation: &DbOperation,
    target: TypeRefIr,
    projection: Option<&DbProjectionIr>,
    db: Option<&DbMetadataIr>,
) -> Result<TypeRefIr> {
    let read_target = if let Some(db) = db {
        db_read_result_type_ir(db, target.clone(), projection)?
    } else {
        target.clone()
    };
    let write_target = target;
    match operation.op {
        DbOperationKind::Find if operation.many => Ok(TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![read_target],
        }),
        DbOperationKind::Find | DbOperationKind::Optional => Ok(TypeRefIr::Nullable {
            inner: Box::new(read_target),
        }),
        DbOperationKind::Insert if operation.many => Ok(TypeRefIr::builtin("DbInsertManyResult")),
        DbOperationKind::Update if operation.many => Ok(TypeRefIr::builtin("DbUpdateManyResult")),
        DbOperationKind::Delete if operation.many => Ok(TypeRefIr::builtin("DbDeleteManyResult")),
        DbOperationKind::Require => Ok(read_target),
        DbOperationKind::Insert => Ok(write_target),
        DbOperationKind::Update | DbOperationKind::Replace => Ok(TypeRefIr::Nullable {
            inner: Box::new(write_target),
        }),
        DbOperationKind::Upsert => Ok(TypeRefIr::Builtin {
            name: "DbUpsertResult".to_string(),
            args: vec![write_target],
        }),
        DbOperationKind::Delete | DbOperationKind::Exists => Ok(TypeRefIr::Builtin {
            name: "bool".to_string(),
            args: Vec::new(),
        }),
        DbOperationKind::Count => Ok(TypeRefIr::Builtin {
            name: "number".to_string(),
            args: Vec::new(),
        }),
    }
}

fn db_read_result_type_ir(
    db: &DbMetadataIr,
    full_target: TypeRefIr,
    projection: Option<&DbProjectionIr>,
) -> Result<TypeRefIr> {
    let projection_paths = projection.map(|projection| {
        projection
            .fields
            .iter()
            .map(|field| field.segments.clone())
            .collect::<Vec<_>>()
    });
    project_db_read_type(
        &db.type_name,
        &db.key.name,
        full_target,
        &db.field_types,
        projection_paths.as_deref(),
    )
    .map_err(CompileError::Semantic)
}

fn db_read_result_type_text(db: &DbMetadataIr, projection: Option<&DbProjectionIr>) -> String {
    let Some(projection) = projection else {
        return db_full_result_type_text(db);
    };
    type_ref_ir_type_text(
        &db_read_result_type_ir(db, TypeRefIr::builtin(&db.type_name), Some(projection))
            .expect("validated DB projection must have a result type"),
    )
}

fn db_full_result_type_text(db: &DbMetadataIr) -> String {
    db.type_name.clone()
}

pub(super) fn db_query_type_ref(object: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "DbQuery".to_string(),
        args: vec![object],
    }
}

pub(super) fn canonical_db_type_name(module_path: &str, db_name: &str) -> String {
    if db_name.contains('.') {
        db_name.to_string()
    } else {
        format!("{module_path}.{db_name}")
    }
}

fn source_symbol_key_from_qualified_text(name: &str) -> Option<SourceSymbolKey> {
    let name = name.trim();
    let name = name.strip_prefix("root.").unwrap_or(name);
    let (module_path, symbol) = name.rsplit_once('.')?;
    Some(SourceSymbolKey::new(module_path, symbol))
}

impl<'a> FunctionLowerer<'a> {
    pub(super) fn lower_db_transaction_stmt(
        &mut self,
        body: &skiff_syntax::ast::Block,
    ) -> Result<StmtIr> {
        if self.db_transaction_depth > 0 {
            return Err(CompileError::Semantic(
                "nested db transaction blocks are not allowed".to_string(),
            ));
        }
        if block_contains_return_stmt(body) {
            return Err(CompileError::Semantic(
                "return is not allowed inside db.transaction blocks".to_string(),
            ));
        }
        self.db_transaction_depth += 1;
        let block = self.lower_scoped_block("db_transaction", body, |_| Ok(()));
        self.db_transaction_depth -= 1;
        let block = block?;
        let result = self.push_expr(ExprIr::Literal {
            value: LiteralIr::Null,
        });
        let block_arg = self.push_expr(ExprIr::ValueBlock { block, result });
        let call = self.push_expr(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::Builtin {
                    op: "db.transaction".to_string(),
                },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                },
                args: vec![block_arg],
                inout_args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: db_builtin_metadata("transaction", None),
            },
        });
        Ok(StmtIr::Expr { value: call })
    }

    pub(super) fn lower_db_operation(&mut self, operation: &DbOperation) -> Result<ExprIr> {
        let db_metadata = self
            .resolve_db_operation_target(&operation.target.name)?
            .clone();
        self.validate_db_operation_semantics(operation, &db_metadata)?;
        let target_type_ref =
            self.lower_resolved_db_target_type(&operation.target, &db_metadata)?;
        let target = DbTargetIr {
            type_ref: target_type_ref.clone(),
            type_name: db_metadata.canonical_type_name.clone(),
        };
        let mut selector = match operation.selector.as_ref() {
            Some(DbSelector::Query { .. }) => None,
            Some(selector) => Some(self.lower_db_selector(selector, &db_metadata)?),
            None => None,
        };
        let query = operation
            .query
            .as_ref()
            .map(|query| self.lower_db_query(query, &db_metadata))
            .transpose()?;
        if let Some(DbSelector::Query {
            query: selector_query,
        }) = operation.selector.as_ref()
        {
            let query = match query.as_ref() {
                Some(query) => query.clone(),
                None => self.lower_db_query(selector_query, &db_metadata)?,
            };
            selector = Some(DbSelectorIr::Query { query });
        }
        let projection = operation
            .projection
            .as_ref()
            .map(|projection| self.lower_db_projection(&db_metadata, projection))
            .transpose()?;
        let result_type = db_operation_result_type_ir(
            operation,
            target_type_ref,
            projection.as_ref(),
            Some(&db_metadata),
        )?;
        let body = operation
            .body
            .as_ref()
            .map(|body| self.lower_db_body(body))
            .transpose()?;
        let insert_body = operation
            .insert_body
            .as_ref()
            .map(|body| self.lower_db_body(body))
            .transpose()?;
        let change = operation
            .change
            .as_ref()
            .map(|change| self.lower_db_change(change))
            .transpose()?;
        Ok(ExprIr::DbOperation {
            operation: DbOperationIr {
                op: lower_db_op(operation.op),
                many: operation.many,
                target,
                selector,
                query,
                projection,
                body,
                insert_body,
                change,
                result_type,
                source_span: None,
            },
        })
    }

    pub(super) fn lower_db_query_value(&mut self, query: &DbQuery) -> Result<ExprIr> {
        let db_metadata = self
            .resolve_db_operation_target(&query.target.name)?
            .clone();
        let target_type_ref = self.lower_resolved_db_target_type(&query.target, &db_metadata)?;
        let target = DbTargetIr {
            type_ref: target_type_ref.clone(),
            type_name: db_metadata.canonical_type_name.clone(),
        };
        let query_ir = self.lower_db_query(&query.query, &db_metadata)?;
        Ok(ExprIr::DbQuery {
            query: DbQueryValueIr {
                target,
                query: query_ir,
                result_type: db_query_type_ref(target_type_ref),
                source_span: None,
            },
        })
    }

    pub(super) fn lower_db_lease_claim(&mut self, claim: &DbLeaseClaim) -> Result<ExprIr> {
        if self.db_transaction_depth > 0 {
            return Err(CompileError::Semantic(
                "db claim is not allowed inside db transaction blocks".to_string(),
            ));
        }
        if block_contains_return_stmt(&claim.body) {
            return Err(CompileError::Semantic(
                "return is not allowed inside db claim blocks".to_string(),
            ));
        }
        let db_metadata = self
            .resolve_db_operation_target(&claim.target.name)?
            .clone();
        validate_db_lease_slot(&db_metadata, &claim.slot)?;
        let target_type_ref = self.lower_resolved_db_target_type(&claim.target, &db_metadata)?;
        let target_type_text = type_ref_ir_type_text(&target_type_ref);
        let target = DbTargetIr {
            type_ref: target_type_ref,
            type_name: db_metadata.canonical_type_name.clone(),
        };
        let key = self.lower_expr(&claim.key)?;
        let mut binding_slot = None;
        let body = self.lower_scoped_block("db_claim", &claim.body, |lowerer| {
            if let Some(binding) = &claim.binding {
                binding_slot = Some(lowerer.declare_slot_with_type(
                    binding,
                    SlotKind::Local,
                    false,
                    BindingReadonlyFlags {
                        readonly: true,
                        readonly_array_item: false,
                    },
                    Some(target_type_text),
                )?);
            }
            Ok(())
        })?;
        Ok(ExprIr::DbLeaseClaim {
            claim: DbLeaseClaimIr {
                target,
                key,
                slot: claim.slot.clone(),
                binding_slot,
                body,
                result_type: TypeRefIr::builtin("bool"),
                source_span: None,
            },
        })
    }

    pub(super) fn lower_db_lease_read(&mut self, read: &DbLeaseRead) -> Result<ExprIr> {
        let db_metadata = self.resolve_db_operation_target(&read.target.name)?.clone();
        validate_db_lease_slot(&db_metadata, &read.slot)?;
        let target_type_ref = self.lower_resolved_db_target_type(&read.target, &db_metadata)?;
        let target = DbTargetIr {
            type_ref: target_type_ref,
            type_name: db_metadata.canonical_type_name.clone(),
        };
        let key = self.lower_expr(&read.key)?;
        Ok(ExprIr::DbLeaseRead {
            read: DbLeaseReadIr {
                target,
                key,
                slot: read.slot.clone(),
                result_type: db_lease_read_result_type_ir(),
                source_span: None,
            },
        })
    }

    pub(super) fn resolve_db_operation_target(&self, target_name: &str) -> Result<&DbMetadataIr> {
        if !target_name.contains('.') {
            if let Some(metadata) = self.db_metadata.get(target_name) {
                return Ok(metadata);
            }
            if let Some(metadata) = self
                .lowered_publication_db_metadata
                .resolve_bare(target_name)?
            {
                return Ok(metadata);
            }
        } else if let Some(metadata) = self
            .lowered_publication_db_metadata
            .resolve_qualified(target_name)
        {
            return Ok(metadata);
        } else if let Some(metadata) = self
            .db_metadata
            .values()
            .find(|metadata| metadata.canonical_type_name == target_name)
        {
            return Ok(metadata);
        }
        Err(CompileError::Semantic(format!(
            "db operation target `{target_name}` is not a declared db object in File IR unit expression"
        )))
    }

    fn lower_resolved_db_target_type(
        &self,
        target: &skiff_syntax::ast::TypeRef,
        metadata: &DbMetadataIr,
    ) -> Result<TypeRefIr> {
        if matches!(metadata.type_ref, TypeRefIr::PackageSymbol { .. }) {
            return Ok(metadata.type_ref.clone());
        }
        lower_type_ref(
            target,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.db_target_type_context(),
        )
    }

    pub(super) fn validate_db_operation_semantics(
        &self,
        operation: &DbOperation,
        db: &DbMetadataIr,
    ) -> Result<()> {
        validate_contract_write_restriction(operation, db)?;
        match operation.op {
            DbOperationKind::Insert if !operation.many => {
                let Some(body) = &operation.body else {
                    return Err(CompileError::Semantic(format!(
                        "db insert {} requires an object body",
                        operation.target.name
                    )));
                };
                self.validate_db_body_fields(body, db, DbBodyValidationMode::Insert)?;
            }
            DbOperationKind::Replace => {
                let Some(body) = &operation.body else {
                    return Err(CompileError::Semantic(format!(
                        "db replace {} requires an object body",
                        operation.target.name
                    )));
                };
                let mode = if matches!(operation.selector, Some(DbSelector::Key { .. })) {
                    DbBodyValidationMode::ReplaceByKey
                } else {
                    DbBodyValidationMode::ReplaceByQuery
                };
                self.validate_db_body_fields(body, db, mode)?;
            }
            DbOperationKind::Upsert => {
                if let Some(body) = &operation.insert_body {
                    self.validate_db_body_fields(body, db, DbBodyValidationMode::UpsertByKey)?;
                }
            }
            _ => {}
        }
        if let Some(change) = &operation.change {
            let selector_kind =
                if !operation.many && matches!(operation.selector, Some(DbSelector::Key { .. })) {
                    DbSelectorKind::KnownRecordKey
                } else {
                    DbSelectorKind::UnknownRecordKey
                };
            self.validate_db_change(change, db, selector_kind)?;
        }
        Ok(())
    }

    pub(super) fn validate_db_body_fields(
        &self,
        body: &DbBody,
        db: &DbMetadataIr,
        mode: DbBodyValidationMode,
    ) -> Result<()> {
        let DbBody::ObjectFields { fields } = body else {
            return Ok(());
        };

        let mut present = BTreeSet::new();
        for field in fields {
            if !db.fields.contains(&field.field) {
                return Err(CompileError::Semantic(format!(
                    "{} body references unknown field `{}` on {}",
                    db_body_validation_label(mode),
                    field.field,
                    db.type_name
                )));
            }
            if matches!(mode, DbBodyValidationMode::ReplaceByKey) && field.field == db.key.name {
                return Err(CompileError::Semantic(format!(
                    "db replace by key body cannot include key field `{}` on {}; selector preserves the key",
                    db.key.name, db.type_name
                )));
            }
            if matches!(mode, DbBodyValidationMode::UpsertByKey) && field.field == db.key.name {
                return Err(CompileError::Semantic(format!(
                    "db upsert by key insert body cannot include key field `{}` on {}; selector provides the key",
                    db.key.name, db.type_name
                )));
            }
            present.insert(field.field.clone());
        }

        for required in required_db_body_fields(db, mode) {
            if !present.contains(&required) {
                return Err(CompileError::Semantic(format!(
                    "{} body missing required field `{}` on {}",
                    db_body_validation_label(mode),
                    required,
                    db.type_name
                )));
            }
        }
        Ok(())
    }

    pub(super) fn validate_db_change(
        &self,
        change: &DbChange,
        db: &DbMetadataIr,
        selector_kind: DbSelectorKind,
    ) -> Result<()> {
        let mut paths = Vec::new();
        for op in &change.ops {
            let path = db_change_op_path(op);
            self.validate_db_change_path(path, db)?;
            db.validate_storage_field_use(
                &path.segments,
                if matches!(op, DbChangeOp::Set { .. }) {
                    DbFieldUse::WholeSet
                } else {
                    DbFieldUse::PartialChange
                },
                selector_kind,
            )?;
            self.validate_db_change_op_type(op, path, db)?;
            paths.push(path);
        }

        for (index, left) in paths.iter().enumerate() {
            for right in paths.iter().skip(index + 1) {
                if let Some((parent, child)) = parent_child_db_paths(left, right) {
                    return Err(CompileError::Semantic(format!(
                        "db change block cannot modify both `{}` and child path `{}` on {}",
                        parent.text, child.text, db.type_name
                    )));
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_db_change_path(
        &self,
        path: &FieldPath,
        db: &DbMetadataIr,
    ) -> Result<()> {
        let Some(first) = path.segments.first() else {
            return Err(CompileError::Semantic(
                "db change field path cannot be empty".to_string(),
            ));
        };
        if path.segments.len() != 1 {
            return Err(CompileError::Semantic(format!(
                "db change field path `{}` on {} must be a top-level stored field in this Object DB version",
                path.text, db.type_name
            )));
        }
        if *first == db.key.name {
            return Err(CompileError::Semantic(format!(
                "db change block cannot modify key field `{}` on {}",
                first, db.type_name
            )));
        }
        if !db.fields.contains(first) {
            return Err(CompileError::Semantic(format!(
                "db change block references unknown field `{}` on {}",
                first, db.type_name
            )));
        }
        Ok(())
    }

    pub(super) fn validate_db_change_op_type(
        &self,
        op: &DbChangeOp,
        path: &FieldPath,
        db: &DbMetadataIr,
    ) -> Result<()> {
        let Some(field_name) = path.segments.first() else {
            return Ok(());
        };
        let Some(field_type) = db.field_types.get(field_name) else {
            return Ok(());
        };
        match op {
            DbChangeOp::Inc { .. }
                if path.segments.len() == 1 && !is_numeric_db_field(field_type) =>
            {
                Err(CompileError::Semantic(format!(
                    "db change operator +=/-= requires numeric field `{}` on {}",
                    path.text, db.type_name
                )))
            }
            DbChangeOp::AddToSet { .. } | DbChangeOp::Remove { .. }
                if path.segments.len() == 1 && !is_array_db_field(field_type) =>
            {
                Err(CompileError::Semantic(format!(
                    "db change add/remove requires array field `{}` on {}",
                    path.text, db.type_name
                )))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn lower_db_selector(
        &mut self,
        selector: &DbSelector,
        db: &DbMetadataIr,
    ) -> Result<DbSelectorIr> {
        match selector {
            DbSelector::Key { value } => Ok(DbSelectorIr::Key {
                value: self.lower_expr(value)?,
            }),
            DbSelector::Query { query } => Ok(DbSelectorIr::Query {
                query: self.lower_db_query(query, db)?,
            }),
        }
    }

    pub(super) fn lower_db_projection(
        &self,
        db: &DbMetadataIr,
        projection: &skiff_syntax::ast::DbProjection,
    ) -> Result<DbProjectionIr> {
        let mut fields = Vec::new();
        for field in &projection.fields {
            let field = db_field_path_ir(field);
            db.validate_storage_field_use(
                &field.segments,
                DbFieldUse::Projection,
                DbSelectorKind::UnknownRecordKey,
            )?;
            fields.push(field);
        }
        if !fields
            .iter()
            .any(|field| field.segments.first() == Some(&db.key.name))
        {
            fields.insert(
                0,
                FieldPathIr {
                    text: db.key.name.clone(),
                    segments: vec![db.key.name.clone()],
                },
            );
        }
        Ok(DbProjectionIr { fields })
    }

    pub(super) fn lower_db_query(
        &mut self,
        query: &DbQueryBlock,
        db: &DbMetadataIr,
    ) -> Result<DbQueryIr> {
        if query.after.is_some() {
            return Err(CompileError::Semantic(
                "db query after is not supported; use offset".to_string(),
            ));
        }
        Ok(DbQueryIr {
            where_clauses: query
                .where_clauses
                .iter()
                .map(|clause| self.lower_db_where_clause(clause, db))
                .collect::<Result<Vec<_>>>()?,
            order: query
                .order
                .iter()
                .map(|entry| {
                    db.validate_storage_field_use(
                        &entry.field.segments,
                        DbFieldUse::Order,
                        DbSelectorKind::UnknownRecordKey,
                    )?;
                    Ok(DbOrderEntryIr {
                        field: db_field_path_ir(&entry.field),
                        direction: match entry.direction {
                            skiff_syntax::ast::DbIndexDirection::Asc => DbIndexDirectionIr::Asc,
                            skiff_syntax::ast::DbIndexDirection::Desc => DbIndexDirectionIr::Desc,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            limit: query
                .limit
                .as_ref()
                .map(|limit| self.lower_expr(limit))
                .transpose()?,
            offset: query
                .offset
                .as_ref()
                .map(|offset| self.lower_expr(offset))
                .transpose()?,
            after: None,
        })
    }

    pub(super) fn lower_db_where_clause(
        &mut self,
        clause: &DbWhereClause,
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        match clause {
            DbWhereClause::Predicate { predicate } => self.lower_db_query_expr(predicate, db),
            DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                let condition = self.lower_expr(condition)?;
                let predicate = self.lower_db_query_expr(predicate, db)?;
                Ok(DbPredicateIr::Conditional {
                    condition,
                    predicate: Box::new(predicate),
                })
            }
        }
    }

    pub(super) fn lower_db_query_expr(
        &mut self,
        expr: &Expr,
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        self.consume_expression_key();
        match expr {
            Expr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => self.lower_db_query_logical(true, left, right, db),
            Expr::Binary {
                op: BinaryOp::Or,
                left,
                right,
            } => self.lower_db_query_logical(false, left, right, db),
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
            } => {
                let predicate = self.lower_db_query_expr(expr, db)?;
                Ok(DbPredicateIr::Not {
                    predicate: Box::new(predicate),
                })
            }
            Expr::Binary { op, left, right } if db_query_comparison_operator(*op).is_some() => {
                self.lower_db_query_comparison(*op, left, right, db)
            }
            Expr::Call { callee, args }
                if matches!(callee.as_ref(), Expr::Identifier(name) if name == "regex") =>
            {
                self.lower_db_query_regex(callee, args, db)
            }
            _ => Err(CompileError::Semantic(
                "unsupported db query predicate; use field comparisons, regex(field, pattern), joined with && or ||"
                    .to_string(),
            )),
        }
    }

    pub(super) fn lower_db_query_logical(
        &mut self,
        is_and: bool,
        left: &Expr,
        right: &Expr,
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        let left = self.lower_db_query_expr(left, db)?;
        let right = self.lower_db_query_expr(right, db)?;
        let predicates = vec![left, right];
        if is_and {
            Ok(DbPredicateIr::And { predicates })
        } else {
            Ok(DbPredicateIr::Or { predicates })
        }
    }

    pub(super) fn lower_db_query_comparison(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        let Some(operator) = db_query_comparison_operator(op) else {
            unreachable!("caller checks db query comparison operator")
        };
        let Some(path) = ast_field_path(left) else {
            return Err(CompileError::Semantic(
                "db query comparison must use a db field path on the left-hand side".to_string(),
            ));
        };
        self.validate_db_query_field_path(&path, db)?;
        self.consume_db_query_field_path_expression_keys(left)?;
        let value = self.lower_expr(right)?;
        Ok(DbPredicateIr::Compare {
            field: FieldPathIr {
                text: path.join("."),
                segments: path,
            },
            op: operator,
            value,
        })
    }

    pub(super) fn lower_db_query_regex(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        self.consume_db_query_field_path_expression_keys(callee)?;
        if !(2..=3).contains(&args.len()) {
            return Err(CompileError::Semantic(
                "db query regex predicate expects regex(field, pattern) or regex(field, pattern, options)"
                    .to_string(),
            ));
        }
        let Some(path) = ast_field_path(args[0].expr()) else {
            return Err(CompileError::Semantic(
                "db query regex first argument must be a db field path".to_string(),
            ));
        };
        self.validate_db_query_field_path(&path, db)?;
        self.consume_db_query_field_path_expression_keys(args[0].expr())?;
        let pattern = self.lower_expr(args[1].expr())?;
        let options = args
            .get(2)
            .map(|options| self.lower_expr(options.expr()))
            .transpose()?;
        Ok(DbPredicateIr::Regex {
            field: FieldPathIr {
                text: path.join("."),
                segments: path,
            },
            pattern,
            options,
        })
    }

    fn consume_db_query_field_path_expression_keys(&mut self, expr: &Expr) -> Result<()> {
        self.consume_expression_key();
        match expr {
            Expr::Identifier(_) => Ok(()),
            Expr::Field { object, .. } => self.consume_db_query_field_path_expression_keys(object),
            Expr::Generic { callee, .. } => {
                self.consume_db_query_field_path_expression_keys(callee)
            }
            _ => Err(CompileError::Semantic(
                "db query field path must be an identifier or field path".to_string(),
            )),
        }
    }

    pub(super) fn validate_db_query_field_path(
        &self,
        path: &[String],
        db: &DbMetadataIr,
    ) -> Result<()> {
        let Some(first) = path.first() else {
            return Err(CompileError::Semantic(
                "db query field path cannot be empty".to_string(),
            ));
        };
        if !db.fields.contains(first) {
            return Err(CompileError::Semantic(format!(
                "db query predicate references unknown field `{}` on {}",
                first, db.type_name
            )));
        }
        db.validate_storage_field_use(
            path,
            DbFieldUse::Predicate,
            DbSelectorKind::UnknownRecordKey,
        )?;
        Ok(())
    }

    pub(super) fn lower_db_body(&mut self, body: &DbBody) -> Result<DbBodyIr> {
        match body {
            DbBody::ObjectFields { fields } => {
                let mut lowered = BTreeMap::new();
                for field in fields {
                    if lowered.contains_key(&field.field) {
                        return Err(CompileError::Semantic(format!(
                            "duplicate db body field `{}` in File IR unit expression",
                            field.field
                        )));
                    }
                    lowered.insert(field.field.clone(), self.lower_expr(&field.value)?);
                }
                Ok(DbBodyIr::ObjectFields { fields: lowered })
            }
            DbBody::Values { value } => Ok(DbBodyIr::Values {
                value: self.lower_expr(value)?,
            }),
        }
    }

    pub(super) fn lower_db_change(&mut self, change: &DbChange) -> Result<DbChangeIr> {
        let mut ops = Vec::new();
        for op in &change.ops {
            ops.push(match op {
                DbChangeOp::Set { path, value } => DbChangeOpIr::Set {
                    path: db_field_path_ir(path),
                    value: self.lower_expr(value)?,
                },
                DbChangeOp::Inc { path, value } => DbChangeOpIr::Inc {
                    path: db_field_path_ir(path),
                    value: self.lower_expr(value)?,
                },
                DbChangeOp::Unset { path } => DbChangeOpIr::Unset {
                    path: db_field_path_ir(path),
                },
                DbChangeOp::AddToSet { path, value } => DbChangeOpIr::AddToSet {
                    path: db_field_path_ir(path),
                    value: self.lower_expr(value)?,
                },
                DbChangeOp::Remove { path, value } => DbChangeOpIr::Remove {
                    path: db_field_path_ir(path),
                    value: self.lower_expr(value)?,
                },
            });
        }
        Ok(DbChangeIr { ops })
    }

    pub(super) fn lower_db_transaction_expr(
        &mut self,
        transaction: &skiff_syntax::ast::DbTransaction,
    ) -> Result<ExprIr> {
        if self.db_transaction_depth > 0 {
            return Err(CompileError::Semantic(
                "nested db transaction blocks are not allowed".to_string(),
            ));
        }
        if block_contains_return_stmt(&transaction.body) {
            return Err(CompileError::Semantic(
                "return is not allowed inside db transaction blocks".to_string(),
            ));
        }
        self.db_transaction_depth += 1;
        let lowered = self.lower_db_transaction_body(transaction);
        self.db_transaction_depth -= 1;
        let (block, result, result_type) = lowered?;
        Ok(ExprIr::DbTransaction {
            transaction: DbTransactionIr {
                mode: match transaction.mode {
                    DbBlockMode::Effect => DbBlockModeIr::Effect,
                    DbBlockMode::Value => DbBlockModeIr::Value,
                },
                body: block,
                result,
                result_type,
            },
        })
    }

    pub(super) fn lower_db_transaction_body(
        &mut self,
        transaction: &skiff_syntax::ast::DbTransaction,
    ) -> Result<(String, ExprRefIr, TypeRefIr)> {
        let label = self.next_block_label("db_transaction");
        self.push_scope();
        let mut lowered = BlockIr {
            label: label.clone(),
            statements: Vec::new(),
        };

        let (statements, value_result) = match transaction.mode {
            DbBlockMode::Effect => (transaction.body.statements.as_slice(), None),
            DbBlockMode::Value => {
                let Some((last, prefix)) = transaction.body.statements.split_last() else {
                    self.pop_scope();
                    return Err(CompileError::Semantic(
                        "db transaction value requires a final expression".to_string(),
                    ));
                };
                let Stmt::Expr(value) = last else {
                    self.pop_scope();
                    return Err(CompileError::Semantic(
                        "db transaction value final statement must be an expression".to_string(),
                    ));
                };
                (prefix, Some(value))
            }
        };

        for stmt in statements {
            lowered.statements.push(self.lower_stmt(stmt)?);
        }

        let (result, result_type) = if let Some(value) = value_result {
            let result_type = self
                .next_expression_type_ir()
                .or_else(|| {
                    self.expression_types
                        .is_none()
                        .then(|| self.infer_expr_type_ir(value))
                        .flatten()
                })
                .unwrap_or_else(|| TypeRefIr::builtin("Json"));
            (self.lower_expr(value)?, result_type)
        } else {
            (
                self.push_expr(ExprIr::Literal {
                    value: LiteralIr::Null,
                }),
                TypeRefIr::builtin("null"),
            )
        };

        self.pop_scope();
        self.body.blocks.push(lowered);
        Ok((label, result, result_type))
    }

    pub(super) fn lower_db_call_metadata(
        &self,
        op: &str,
        type_args: &[TypeRef],
        first_type_arg_key: Option<&str>,
        args: &[CallArg],
    ) -> Result<BTreeMap<String, MetadataValue>> {
        let operation = op.strip_prefix("db.").unwrap_or(op);
        let call_type = self.db_call_type(operation, type_args, args)?;
        let mut metadata = db_builtin_metadata(operation, first_type_arg_key);
        let Some((type_text, lowered_type)) = call_type else {
            return Ok(metadata);
        };
        metadata.insert(
            "typeName".to_string(),
            MetadataValue::String(type_text.clone()),
        );
        metadata.insert(
            "type".to_string(),
            MetadataValue::from_serializable(&lowered_type),
        );
        if let Ok(db) = self.resolve_db_operation_target(&db_metadata_lookup_key(&type_text)) {
            metadata.insert(
                "declaredTypeName".to_string(),
                MetadataValue::String(db.type_name.clone()),
            );
            metadata.insert(
                "declaredType".to_string(),
                MetadataValue::from_serializable(&db.type_ref),
            );
            if let Some(collection_name) = &db.collection_name {
                metadata.insert(
                    "collectionName".to_string(),
                    MetadataValue::String(collection_name.clone()),
                );
            }
            metadata.insert(
                "kind".to_string(),
                MetadataValue::String(
                    match db.kind {
                        DbObjectKindIr::Object => "object",
                        DbObjectKindIr::Contract => "contract",
                    }
                    .to_string(),
                ),
            );
            if let Some(retention) = &db.retention {
                metadata.insert(
                    "retention".to_string(),
                    MetadataValue::from_serializable(retention),
                );
            }
            metadata.insert("key".to_string(), MetadataValue::from_serializable(&db.key));
        }
        Ok(metadata)
    }

    pub(super) fn db_call_type(
        &self,
        operation: &str,
        type_args: &[TypeRef],
        args: &[CallArg],
    ) -> Result<Option<(String, TypeRefIr)>> {
        let explicit_type = |ty: &TypeRef| {
            Ok(Some((
                ty.name.clone(),
                lower_type_ref(
                    ty,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.value_type_context(),
                )?,
            )))
        };
        let legacy_type = |type_text: String| {
            Ok(Some((
                type_text.clone(),
                lower_type_text(
                    &type_text,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.value_type_context(),
                )?,
            )))
        };
        match operation {
            "get" | "require" | "exists" | "findMany" | "count" | "upsert" => type_args
                .first()
                .map(explicit_type)
                .transpose()
                .map(|ty| ty.flatten()),
            "create" | "append" => {
                if let Some(ty) = type_args.first() {
                    return explicit_type(ty);
                }
                if args.first().is_none() {
                    return Ok(None);
                }
                if let Some(resolved) = self.next_expression_type() {
                    return Ok(Some(resolved));
                }
                if self.expression_types.is_none() {
                    return args
                        .first()
                        .and_then(|arg| self.infer_expr_type_text(arg.expr()))
                        .map(legacy_type)
                        .transpose()
                        .map(|ty| ty.flatten());
                }
                Ok(None)
            }
            "createMany" | "create_many" | "appendMany" | "append_many" => {
                if let Some(ty) = type_args.first() {
                    return explicit_type(ty);
                }
                if args.first().is_none() {
                    return Ok(None);
                }
                if let Some(resolved) = self.next_expression_array_item_type() {
                    return Ok(Some(resolved));
                }
                if self.expression_types.is_none() {
                    return args
                        .first()
                        .and_then(|arg| self.infer_array_item_type_text(arg.expr()))
                        .map(legacy_type)
                        .transpose()
                        .map(|ty| ty.flatten());
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn db_operation_result_type_text(&self, operation: &DbOperation) -> Option<String> {
        let db_metadata = self
            .resolve_db_operation_target(&operation.target.name)
            .ok()?;
        let projection = operation
            .projection
            .as_ref()
            .map(|projection| self.lower_db_projection(db_metadata, projection))
            .transpose()
            .ok()?;
        Some(db_operation_result_type_text(
            operation,
            projection.as_ref(),
            Some(db_metadata),
        ))
    }
}

fn db_builtin_metadata(
    operation: &str,
    type_arg_key: Option<&str>,
) -> BTreeMap<String, MetadataValue> {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "builtinRoot".to_string(),
        MetadataValue::String("db".to_string()),
    );
    metadata.insert(
        "dbOp".to_string(),
        MetadataValue::String(operation.to_string()),
    );
    if let Some(type_arg_key) = type_arg_key {
        metadata.insert(
            "typeArgKey".to_string(),
            MetadataValue::String(type_arg_key.to_string()),
        );
    }
    metadata
}

fn db_body_validation_label(mode: DbBodyValidationMode) -> &'static str {
    match mode {
        DbBodyValidationMode::Insert => "db insert",
        DbBodyValidationMode::ReplaceByKey => "db replace by key",
        DbBodyValidationMode::ReplaceByQuery => "db replace by query",
        DbBodyValidationMode::UpsertByKey => "db upsert by key insert",
    }
}

fn required_db_body_fields(db: &DbMetadataIr, mode: DbBodyValidationMode) -> Vec<String> {
    let include_key = matches!(
        mode,
        DbBodyValidationMode::Insert | DbBodyValidationMode::ReplaceByQuery
    );
    let mut fields = Vec::new();
    if include_key {
        fields.push(db.key.name.clone());
    }
    fields.extend(
        db.field_type_texts
            .iter()
            .filter(|(field, ty)| *field != &db.key.name && is_required_db_field_type_text(ty))
            .map(|(field, _)| field.clone()),
    );
    fields
}

fn is_required_db_field_type_text(ty: &str) -> bool {
    let ty = ty.trim();
    if ty.ends_with('?') {
        return false;
    }
    !split_top_level(ty, '|')
        .iter()
        .any(|part| part.trim() == "null")
}

fn db_change_op_path(op: &DbChangeOp) -> &FieldPath {
    match op {
        DbChangeOp::Set { path, .. }
        | DbChangeOp::Inc { path, .. }
        | DbChangeOp::Unset { path }
        | DbChangeOp::AddToSet { path, .. }
        | DbChangeOp::Remove { path, .. } => path,
    }
}

fn parent_child_db_paths<'a>(
    left: &'a FieldPath,
    right: &'a FieldPath,
) -> Option<(&'a FieldPath, &'a FieldPath)> {
    if is_parent_db_path(left, right) {
        return Some((left, right));
    }
    if is_parent_db_path(right, left) {
        return Some((right, left));
    }
    None
}

fn is_parent_db_path(parent: &FieldPath, child: &FieldPath) -> bool {
    parent.segments.len() < child.segments.len()
        && child.segments.starts_with(parent.segments.as_slice())
}

fn is_numeric_db_field(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if args.is_empty() && matches!(name.as_str(), "number" | "integer")
    )
}

fn is_array_db_field(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, .. } if name == "Array"
    )
}

fn lower_db_op(op: DbOperationKind) -> DbOpKindIr {
    match op {
        DbOperationKind::Find => DbOpKindIr::Find,
        DbOperationKind::Optional => DbOpKindIr::Optional,
        DbOperationKind::Require => DbOpKindIr::Require,
        DbOperationKind::Insert => DbOpKindIr::Insert,
        DbOperationKind::Update => DbOpKindIr::Update,
        DbOperationKind::Upsert => DbOpKindIr::Upsert,
        DbOperationKind::Replace => DbOpKindIr::Replace,
        DbOperationKind::Delete => DbOpKindIr::Delete,
        DbOperationKind::Count => DbOpKindIr::Count,
        DbOperationKind::Exists => DbOpKindIr::Exists,
    }
}

fn db_operation_kind_text(op: DbOperationKind) -> &'static str {
    match op {
        DbOperationKind::Insert => "insert",
        DbOperationKind::Replace => "replace",
        DbOperationKind::Upsert => "upsert",
        _ => "operation",
    }
}

/// Lowering backstop for the shared-collection write restriction: whole-document
/// insert/replace/upsert on a `db contract` target must never reach the artifact.
fn validate_contract_write_restriction(operation: &DbOperation, db: &DbMetadataIr) -> Result<()> {
    if db.kind == skiff_artifact_model::DbObjectKindIr::Contract
        && matches!(
            operation.op,
            DbOperationKind::Insert | DbOperationKind::Replace | DbOperationKind::Upsert
        )
    {
        return Err(CompileError::Semantic(format!(
            "db {} on contract target `{}` is not allowed: the engine contract view cannot insert or replace the whole shared document; the host owns the collection",
            db_operation_kind_text(operation.op),
            operation.target.name
        )));
    }
    Ok(())
}

fn validate_db_lease_slot(db: &DbMetadataIr, slot: &str) -> Result<()> {
    if db.leases.contains_key(slot) {
        return Ok(());
    }
    Err(CompileError::Semantic(format!(
        "db lease slot `{slot}` is not declared on {}",
        db.type_name
    )))
}

pub(super) fn db_lease_read_result_type_ir() -> TypeRefIr {
    TypeRefIr::Nullable {
        inner: Box::new(TypeRefIr::Record {
            fields: BTreeMap::from([
                ("expiresAt".to_string(), TypeRefIr::builtin("string")),
                ("owner".to_string(), TypeRefIr::builtin("string")),
                ("requestId".to_string(), TypeRefIr::builtin("string")),
            ]),
        }),
    }
}

pub(super) fn db_lease_read_result_type_text() -> String {
    "{ expiresAt: string, owner: string, requestId: string }?".to_string()
}

fn db_query_comparison_operator(op: BinaryOp) -> Option<DbPredicateCompareOpIr> {
    Some(match op {
        BinaryOp::Eq => DbPredicateCompareOpIr::Eq,
        BinaryOp::Ne => DbPredicateCompareOpIr::Ne,
        BinaryOp::Lt => DbPredicateCompareOpIr::Lt,
        BinaryOp::Le => DbPredicateCompareOpIr::Lte,
        BinaryOp::Gt => DbPredicateCompareOpIr::Gt,
        BinaryOp::Ge => DbPredicateCompareOpIr::Gte,
        _ => return None,
    })
}

fn ast_field_path(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier(name) => Some(vec![name.clone()]),
        Expr::Field { object, field } => {
            let mut path = ast_field_path(object)?;
            path.push(field.clone());
            Some(path)
        }
        _ => None,
    }
}

fn db_metadata_lookup_key(type_text: &str) -> String {
    use skiff_syntax::type_syntax::generic_parts;
    let ty = type_text.trim().trim_end_matches('?').trim();
    generic_parts(ty)
        .map(|parts| parts.root.trim().to_string())
        .unwrap_or_else(|| ty.to_string())
}

#[cfg(test)]
mod contract_write_restriction_tests {
    use super::*;

    fn contract_metadata() -> DbMetadataIr {
        DbMetadataIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "AgentThread".to_string(),
            canonical_type_name: "internal.any_lowering.AgentThread".to_string(),
            kind: skiff_artifact_model::DbObjectKindIr::Contract,
            collection_name: None,
            retention: None,
            leases: BTreeMap::new(),
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: BTreeSet::from(["id".to_string(), "status".to_string()]),
            field_types: BTreeMap::new(),
            field_type_texts: BTreeMap::new(),
            field_storage: BTreeMap::new(),
        }
    }

    fn operation(op: DbOperationKind) -> DbOperation {
        DbOperation {
            op,
            many: false,
            target: TypeRef {
                name: "AgentThread".to_string(),
            },
            selector: None,
            query: None,
            projection: None,
            body: None,
            insert_body: None,
            change: None,
        }
    }

    #[test]
    fn whole_document_writes_on_contract_target_are_rejected_at_lowering() {
        for op in [
            DbOperationKind::Insert,
            DbOperationKind::Replace,
            DbOperationKind::Upsert,
        ] {
            let error = validate_contract_write_restriction(&operation(op), &contract_metadata())
                .expect_err("contract target whole-document writes must fail lowering");
            assert!(
                error.to_string().contains("contract target"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn reads_and_field_scoped_updates_on_contract_target_pass_lowering() {
        for op in [
            DbOperationKind::Find,
            DbOperationKind::Optional,
            DbOperationKind::Require,
            DbOperationKind::Update,
            DbOperationKind::Delete,
            DbOperationKind::Count,
            DbOperationKind::Exists,
        ] {
            validate_contract_write_restriction(&operation(op), &contract_metadata())
                .expect("contract target reads and field-scoped writes must lower");
        }
    }
}
