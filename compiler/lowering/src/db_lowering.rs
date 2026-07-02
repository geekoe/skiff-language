use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{
    BlockIr, CallIr, CallTargetIr, DbBlockModeIr, DbBodyIr, DbChangeIr, DbChangeOpIr,
    DbDeclarationIr, DbIndexDirectionIr, DbIndexFieldIr, DbIndexIr, DbLeaseClaimIr, DbLeaseIr,
    DbLeaseReadIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, DbOpKindIr, DbOperationIr,
    DbOrderEntryIr, DbPredicateCompareOpIr, DbPredicateIr, DbProjectionIr, DbQueryIr,
    DbQueryValueIr, DbRetentionIr, DbRetentionUnitIr, DbSelectorIr, DbTargetIr, DbTransactionIr,
    ExprIr, ExprRefIr, FieldPathIr, FileIrUnit, FunctionTypeParamIr, LiteralIr, MetadataValue,
    ServiceSymbolRef, SlotKind, StmtIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_source::{
    semantic::DbAttachmentIndex, LocalDbObjectIndex, PublicationDbMetadata,
    PublicationDbMetadataIndex, PublicationTypeSymbolIndex, SourceSymbolKey,
};
use skiff_syntax::{
    ast::{
        BinaryOp, DbBlockMode, DbBody, DbChange, DbChangeOp, DbIndexDirection, DbLeaseClaim,
        DbLeaseRead, DbOperation, DbOperationKind, DbQuery, DbQueryBlock, DbRetentionUnit,
        DbSelector, DbWhereClause, Expr, FieldPath, Stmt, TypeRef, UnaryOp,
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
    pub(super) collection_name: String,
    pub(super) retention: Option<DbRetentionIr>,
    pub(super) leases: BTreeMap<String, DbLeaseIr>,
    pub(super) key: DbObjectKeyIr,
    pub(super) fields: BTreeSet<String>,
    pub(super) field_types: BTreeMap<String, TypeRefIr>,
    pub(super) field_type_texts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LoweredPublicationDbMetadataIndex {
    by_source_key: BTreeMap<SourceSymbolKey, DbMetadataIr>,
    by_bare_name: BTreeMap<String, BTreeSet<SourceSymbolKey>>,
}

impl LoweredPublicationDbMetadataIndex {
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

fn lower_publication_db_metadata(
    metadata: &PublicationDbMetadata,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
) -> Result<DbMetadataIr> {
    let type_ref = db_object_type_ref(ServiceSymbolRef {
        module_path: metadata.module_path.clone(),
        symbol: metadata.type_name.clone(),
    });
    let empty_local_db_objects = LocalDbObjectIndex::default();
    let empty_publication_db_metadata = PublicationDbMetadataIndex::default();
    let source_alias_targets = BTreeMap::new();
    let key = DbObjectKeyIr {
        name: metadata.key.name.clone(),
        ty: lower_type_ref(
            &metadata.key.ty,
            &BTreeMap::new(),
            &empty_local_db_objects,
            &empty_publication_db_metadata,
            package_aliases,
            external_type_symbols,
            &source_alias_targets,
            TypeLoweringContext::value(),
        )?,
    };
    let mut field_types = BTreeMap::new();
    let mut field_type_texts = BTreeMap::new();
    field_types.insert(key.name.clone(), key.ty.clone());
    field_type_texts.insert(metadata.key.name.clone(), metadata.key.ty.name.clone());
    for (field_name, field_ty) in &metadata.field_types {
        field_types.insert(
            field_name.clone(),
            lower_type_ref(
                field_ty,
                &BTreeMap::new(),
                &empty_local_db_objects,
                &empty_publication_db_metadata,
                package_aliases,
                external_type_symbols,
                &source_alias_targets,
                TypeLoweringContext::value(),
            )?,
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
        collection_name: metadata.collection_name.clone(),
        retention,
        leases,
        key,
        fields: metadata.fields.clone(),
        field_types,
        field_type_texts: metadata.field_type_texts.clone(),
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
        let mut field_type_texts = BTreeMap::new();
        debug_assert!(attachment
            .field_map()
            .contains_key(attachment.key.name.as_str()));
        type_fields.insert(key.name.clone(), key.ty.clone());
        field_type_texts.insert(key_field.name.clone(), key_field.ty.name.clone());
        for field in attachment.fields() {
            field_type_texts.insert(field.name.clone(), field.ty.name.clone());
            let field_ty = db_storage_type_ref(
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
            )?;
            type_fields.insert(field.name.clone(), field_ty);
        }
        let field_types = type_fields.clone();
        let collection_name = db_collection_name(db);
        validate_db_collection_name(&collection_name, &db.name)?;
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
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let indexes = db
            .indexes
            .iter()
            .map(|index| {
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
                    where_expr: index
                        .where_expr
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()
                        .expect("Expr serializes"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        unit.declarations.db.insert(
            db.name.clone(),
            DbDeclarationIr {
                type_ref: type_ref.clone(),
                type_name: db.name.clone(),
                collection_name: collection_name.clone(),
                kind: DbObjectKindIr::Object,
                key: key.clone(),
                fields,
                retention: retention.clone(),
                leases: leases.clone(),
                indexes,
                source_span: Some(source_span.clone()),
            },
        );
        metadata.insert(
            db.name.clone(),
            DbMetadataIr {
                type_ref,
                type_name: db.name.clone(),
                canonical_type_name: canonical_db_type_name(attachment.module_path, &db.name),
                collection_name,
                retention,
                leases: lease_map,
                key,
                fields: db_field_names,
                field_types,
                field_type_texts,
            },
        );
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
                TypeDescriptorIr::Union { variants } => TypeRefIr::Union {
                    items: variants
                        .iter()
                        .map(|variant| expand_db_storage_type_ref(variant, unit, seen_local_types))
                        .collect::<Result<Vec<_>>>()?,
                },
                TypeDescriptorIr::Native { .. } => ty.clone(),
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
        TypeRefIr::Native { name, args } => Ok(TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| expand_db_storage_type_ref(arg, unit, seen_local_types))
                .collect::<Result<Vec<_>>>()?,
        }),
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

/// Variant used from `suspend_analysis` where the db metadata is never available.
pub(super) fn db_operation_result_type_text_no_db(
    operation: &DbOperation,
    projection: Option<&DbProjectionIr>,
) -> String {
    db_operation_result_type_text(operation, projection, None)
}

pub(super) fn db_operation_result_type_text(
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
        DbOperationKind::Find if operation.many => Ok(TypeRefIr::Native {
            name: "Array".to_string(),
            args: vec![read_target],
        }),
        DbOperationKind::Find | DbOperationKind::Optional => Ok(TypeRefIr::Nullable {
            inner: Box::new(read_target),
        }),
        DbOperationKind::Insert if operation.many => Ok(TypeRefIr::native("DbInsertManyResult")),
        DbOperationKind::Update if operation.many => Ok(TypeRefIr::native("DbUpdateManyResult")),
        DbOperationKind::Delete if operation.many => Ok(TypeRefIr::native("DbDeleteManyResult")),
        DbOperationKind::Require => Ok(read_target),
        DbOperationKind::Insert => Ok(write_target),
        DbOperationKind::Update | DbOperationKind::Replace => Ok(TypeRefIr::Nullable {
            inner: Box::new(write_target),
        }),
        DbOperationKind::Upsert => Ok(TypeRefIr::Native {
            name: "DbUpsertResult".to_string(),
            args: vec![write_target],
        }),
        DbOperationKind::Delete | DbOperationKind::Exists => Ok(TypeRefIr::Native {
            name: "bool".to_string(),
            args: Vec::new(),
        }),
        DbOperationKind::Count => Ok(TypeRefIr::Native {
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
    let Some(projection) = projection else {
        return Ok(full_target);
    };
    let mut root = ProjectionTypeNode::default();
    for field in &projection.fields {
        insert_projection_type_path(&mut root, &field.segments, db)?;
    }
    Ok(TypeRefIr::Record {
        fields: root
            .children
            .into_iter()
            .map(|(name, node)| {
                let ty = db
                    .field_types
                    .get(&name)
                    .expect("projection lowering validates top-level DB fields");
                Ok((name.clone(), projection_node_type(db, &name, ty, &node)?))
            })
            .collect::<Result<BTreeMap<_, _>>>()?,
    })
}

#[derive(Debug, Default)]
struct ProjectionTypeNode {
    terminal: bool,
    children: BTreeMap<String, ProjectionTypeNode>,
}

fn insert_projection_type_path(
    root: &mut ProjectionTypeNode,
    segments: &[String],
    db: &DbMetadataIr,
) -> Result<()> {
    let Some(first) = segments.first() else {
        return Err(CompileError::Semantic(format!(
            "db projection field path on {} cannot be empty",
            db.type_name
        )));
    };
    if !db.fields.contains(first) {
        return Err(CompileError::Semantic(format!(
            "db projection references unknown field `{first}` on {}",
            db.type_name
        )));
    }
    let text = segments.join(".");
    let mut node = root;
    for (index, segment) in segments.iter().enumerate() {
        if node.terminal {
            let parent = segments[..index].join(".");
            return Err(CompileError::Semantic(format!(
                "db projection cannot include both `{parent}` and child path `{text}` on {}",
                db.type_name
            )));
        }
        node = node.children.entry(segment.clone()).or_default();
    }
    if node.terminal {
        return Err(CompileError::Semantic(format!(
            "duplicate db projection field `{text}` on {}",
            db.type_name
        )));
    }
    if !node.children.is_empty() {
        let child = first_projection_child_path(segments, node);
        return Err(CompileError::Semantic(format!(
            "db projection cannot include both `{text}` and child path `{child}` on {}",
            db.type_name
        )));
    }
    node.terminal = true;
    Ok(())
}

fn first_projection_child_path(parent: &[String], node: &ProjectionTypeNode) -> String {
    let mut path = parent.to_vec();
    let mut current = node;
    while let Some((name, next)) = current.children.iter().next() {
        path.push(name.clone());
        current = next;
    }
    path.join(".")
}

fn projection_node_type(
    db: &DbMetadataIr,
    path: &str,
    ty: &TypeRefIr,
    node: &ProjectionTypeNode,
) -> Result<TypeRefIr> {
    if node.terminal {
        return Ok(ty.clone());
    }
    let (inner, nullable) = unwrap_nullable_type(ty);
    let TypeRefIr::Record { fields } = inner else {
        let attempted_path = first_projection_child_type_path(path, node);
        return Err(CompileError::Semantic(format!(
            "db projection field `{attempted_path}` on {} cannot traverse non-record type",
            db.type_name
        )));
    };
    let projected = TypeRefIr::Record {
        fields: node
            .children
            .iter()
            .map(|(name, child)| {
                let child_path = format!("{path}.{name}");
                let Some(child_ty) = fields.get(name) else {
                    return Err(CompileError::Semantic(format!(
                        "db projection references unknown field `{child_path}` on {}",
                        db.type_name
                    )));
                };
                Ok((
                    name.clone(),
                    projection_node_type(db, &child_path, child_ty, child)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?,
    };
    if nullable {
        Ok(TypeRefIr::Nullable {
            inner: Box::new(projected),
        })
    } else {
        Ok(projected)
    }
}

fn first_projection_child_type_path(parent: &str, node: &ProjectionTypeNode) -> String {
    let mut path = vec![parent.to_string()];
    let mut current = node;
    while let Some((name, next)) = current.children.iter().next() {
        path.push(name.clone());
        current = next;
    }
    path.join(".")
}

fn unwrap_nullable_type(ty: &TypeRefIr) -> (&TypeRefIr, bool) {
    match ty {
        TypeRefIr::Nullable { inner } => (inner, true),
        _ => (ty, false),
    }
}

fn db_read_result_type_text(db: &DbMetadataIr, projection: Option<&DbProjectionIr>) -> String {
    let Some(projection) = projection else {
        return db_full_result_type_text(db);
    };
    db_read_result_type_ir(db, TypeRefIr::native(&db.type_name), Some(projection))
        .map(|ty| type_ref_ir_type_text(&ty))
        .unwrap_or_else(|_| projection_record_type_text_legacy(db, projection))
}

fn db_full_result_type_text(db: &DbMetadataIr) -> String {
    db.type_name.clone()
}

fn projection_record_type_text_legacy(db: &DbMetadataIr, projection: &DbProjectionIr) -> String {
    let fields = projection
        .fields
        .iter()
        .filter_map(|field| {
            field
                .segments
                .first()
                .and_then(|name| db.field_type_texts.get(name).map(|ty| (name, ty)))
        })
        .map(|(name, ty)| format!("{name}: {ty}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {fields} }}")
}

pub(super) fn db_query_type_ref(object: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Native {
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
                args: vec![block_arg],
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
        let target_type_ref = lower_type_ref(
            &operation.target,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.db_target_type_context(),
        )?;
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
        let target_type_ref = lower_type_ref(
            &query.target,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.db_target_type_context(),
        )?;
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
        let target_type_ref = lower_type_ref(
            &claim.target,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.db_target_type_context(),
        )?;
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
                result_type: TypeRefIr::native("bool"),
                source_span: None,
            },
        })
    }

    pub(super) fn lower_db_lease_read(&mut self, read: &DbLeaseRead) -> Result<ExprIr> {
        let db_metadata = self.resolve_db_operation_target(&read.target.name)?.clone();
        validate_db_lease_slot(&db_metadata, &read.slot)?;
        let target_type_ref = lower_type_ref(
            &read.target,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.db_target_type_context(),
        )?;
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

    pub(super) fn validate_db_operation_semantics(
        &self,
        operation: &DbOperation,
        db: &DbMetadataIr,
    ) -> Result<()> {
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
            self.validate_db_change(change, db)?;
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

    pub(super) fn validate_db_change(&self, change: &DbChange, db: &DbMetadataIr) -> Result<()> {
        let mut paths = Vec::new();
        for op in &change.ops {
            let path = db_change_op_path(op);
            self.validate_db_change_path(path, db)?;
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
        let mut seen = ProjectionPathSet::default();
        for field in &projection.fields {
            let field = db_field_path_ir(field);
            let Some(name) = field.segments.first() else {
                return Err(CompileError::Semantic(format!(
                    "db projection field path on {} cannot be empty",
                    db.type_name
                )));
            };
            if !db.fields.contains(name) {
                return Err(CompileError::Semantic(format!(
                    "db projection references unknown field `{name}` on {}",
                    db.type_name
                )));
            }
            seen.insert(&field, db)?;
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
                .map(|entry| DbOrderEntryIr {
                    field: db_field_path_ir(&entry.field),
                    direction: match entry.direction {
                        skiff_syntax::ast::DbIndexDirection::Asc => DbIndexDirectionIr::Asc,
                        skiff_syntax::ast::DbIndexDirection::Desc => DbIndexDirectionIr::Desc,
                    },
                })
                .collect(),
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
        args: &[Expr],
        db: &DbMetadataIr,
    ) -> Result<DbPredicateIr> {
        self.consume_db_query_field_path_expression_keys(callee)?;
        if !(2..=3).contains(&args.len()) {
            return Err(CompileError::Semantic(
                "db query regex predicate expects regex(field, pattern) or regex(field, pattern, options)"
                    .to_string(),
            ));
        }
        let Some(path) = ast_field_path(&args[0]) else {
            return Err(CompileError::Semantic(
                "db query regex first argument must be a db field path".to_string(),
            ));
        };
        self.validate_db_query_field_path(&path, db)?;
        self.consume_db_query_field_path_expression_keys(&args[0])?;
        let pattern = self.lower_expr(&args[1])?;
        let options = args
            .get(2)
            .map(|options| self.lower_expr(options))
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
                .unwrap_or_else(|| TypeRefIr::native("Json"));
            (self.lower_expr(value)?, result_type)
        } else {
            (
                self.push_expr(ExprIr::Literal {
                    value: LiteralIr::Null,
                }),
                TypeRefIr::native("null"),
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
        args: &[Expr],
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
            metadata.insert(
                "collectionName".to_string(),
                MetadataValue::String(db.collection_name.clone()),
            );
            metadata.insert(
                "kind".to_string(),
                MetadataValue::String("object".to_string()),
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
        args: &[Expr],
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
                        .and_then(|arg| self.infer_expr_type_text(arg))
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
                        .and_then(|arg| self.infer_array_item_type_text(arg))
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

#[derive(Default)]
struct ProjectionPathSet {
    paths: Vec<FieldPathIr>,
}

impl ProjectionPathSet {
    fn insert(&mut self, field: &FieldPathIr, db: &DbMetadataIr) -> Result<()> {
        for existing in &self.paths {
            if existing.segments == field.segments {
                return Err(CompileError::Semantic(format!(
                    "duplicate db projection field `{}` on {}",
                    field.text, db.type_name
                )));
            }
            if existing.segments.len() < field.segments.len()
                && field.segments.starts_with(existing.segments.as_slice())
            {
                return Err(CompileError::Semantic(format!(
                    "db projection cannot include both `{}` and child path `{}` on {}",
                    existing.text, field.text, db.type_name
                )));
            }
            if field.segments.len() < existing.segments.len()
                && existing.segments.starts_with(field.segments.as_slice())
            {
                return Err(CompileError::Semantic(format!(
                    "db projection cannot include both `{}` and child path `{}` on {}",
                    field.text, existing.text, db.type_name
                )));
            }
        }
        self.paths.push(field.clone());
        Ok(())
    }
}

fn is_numeric_db_field(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Native { name, args } if args.is_empty() && matches!(name.as_str(), "number" | "integer")
    )
}

fn is_array_db_field(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Native { name, .. } if name == "Array"
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
                ("expiresAt".to_string(), TypeRefIr::native("string")),
                ("owner".to_string(), TypeRefIr::native("string")),
                ("requestId".to_string(), TypeRefIr::native("string")),
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
