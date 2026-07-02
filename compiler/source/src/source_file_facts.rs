use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageRefIr, ServiceSymbolRef, TypeRefIr};

use crate::{
    semantic::DbAttachmentIndex,
    shared::{
        ast::{DbRetentionUnit, SourceFile, TypeRef},
        ast_utils::db_collection_name,
        error::{CompileError, Result},
    },
};

use super::{PublicationTypeSymbolIndex, SourceSymbolKey};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageInterfaceMethodIndex {
    methods: BTreeMap<PackageInterfaceMethodKey, BTreeSet<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PackageInterfaceMethodKey {
    package_ref: String,
    symbol_path: String,
}

impl PackageInterfaceMethodIndex {
    pub fn insert_method_names(
        &mut self,
        package_ref: impl Into<String>,
        symbol_path: impl Into<String>,
        method_names: impl IntoIterator<Item = String>,
    ) {
        let key = PackageInterfaceMethodKey {
            package_ref: package_ref.into(),
            symbol_path: symbol_path.into(),
        };
        self.methods.entry(key).or_default().extend(method_names);
    }

    pub fn is_interface_method(&self, receiver_ty: &TypeRefIr, method_name: &str) -> bool {
        match receiver_ty {
            TypeRefIr::PackageSymbol { symbol } => {
                let key = PackageInterfaceMethodKey {
                    package_ref: package_ref_identity(&symbol.package).to_string(),
                    symbol_path: symbol.symbol_path.clone(),
                };
                self.methods
                    .get(&key)
                    .is_some_and(|methods| methods.contains(method_name))
            }
            TypeRefIr::Nullable { inner } => self.is_interface_method(inner, method_name),
            _ => false,
        }
    }

    pub fn is_interface_type(&self, receiver_ty: &TypeRefIr) -> bool {
        match receiver_ty {
            TypeRefIr::PackageSymbol { symbol } => {
                let key = PackageInterfaceMethodKey {
                    package_ref: package_ref_identity(&symbol.package).to_string(),
                    symbol_path: symbol.symbol_path.clone(),
                };
                self.methods.contains_key(&key)
            }
            TypeRefIr::Nullable { inner } => self.is_interface_type(inner),
            _ => false,
        }
    }
}

fn package_ref_identity(package: &PackageRefIr) -> &str {
    match package {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref,
        PackageRefIr::PackageId { package_id } => package_id,
    }
}

#[derive(Debug, Clone, Default)]
pub struct LocalDbObjectIndex {
    by_name: BTreeMap<String, ServiceSymbolRef>,
    by_source_key: BTreeMap<SourceSymbolKey, ServiceSymbolRef>,
}

impl LocalDbObjectIndex {
    pub fn from_attachments(attachments: &DbAttachmentIndex<'_>) -> Self {
        let mut index = Self::default();
        for attachment in attachments.iter() {
            let db = attachment.db;
            let source_key = db_source_key(attachment.module_path, &db.name);
            let symbol = ServiceSymbolRef {
                module_path: source_key.module_path().to_string(),
                symbol: source_key.symbol().to_string(),
            };
            index
                .by_name
                .insert(source_key.symbol().to_string(), symbol.clone());
            index.by_source_key.insert(source_key, symbol);
        }
        index
    }

    pub fn from_declarations(module_path: &str, ast: &SourceFile) -> Result<Self> {
        let attachments = DbAttachmentIndex::build(module_path, ast)?;
        Ok(Self::from_attachments(&attachments))
    }

    pub fn resolve(&self, name: &str) -> Option<ServiceSymbolRef> {
        if name.contains('.') {
            source_symbol_key_from_qualified_text(name)
                .and_then(|source_key| self.by_source_key.get(&source_key))
                .cloned()
        } else {
            self.by_name.get(name).cloned()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PublicationDbMetadataIndex {
    by_source_key: BTreeMap<SourceSymbolKey, PublicationDbMetadata>,
    by_bare_name: BTreeMap<String, BTreeSet<SourceSymbolKey>>,
}

impl PublicationDbMetadataIndex {
    fn insert(&mut self, module_path: &str, source_name: &str, metadata: PublicationDbMetadata) {
        let source_key = db_source_key(module_path, source_name);
        self.insert_source_key(source_key, metadata);
    }

    pub fn insert_alias(
        &mut self,
        module_path: &str,
        source_name: &str,
        metadata: PublicationDbMetadata,
    ) {
        self.insert(module_path, source_name, metadata);
    }

    pub fn extend(&mut self, other: PublicationDbMetadataIndex) {
        for (source_key, metadata) in other.by_source_key {
            self.insert_source_key(source_key, metadata);
        }
    }

    fn insert_source_key(&mut self, source_key: SourceSymbolKey, metadata: PublicationDbMetadata) {
        self.by_bare_name
            .entry(source_key.symbol().to_string())
            .or_default()
            .insert(source_key.clone());
        self.by_source_key.insert(source_key, metadata);
    }

    pub fn resolve_qualified(&self, name: &str) -> Option<&PublicationDbMetadata> {
        source_symbol_key_from_qualified_text(name)
            .and_then(|source_key| self.by_source_key.get(&source_key))
    }

    pub fn resolve_bare(&self, name: &str) -> Result<Option<&PublicationDbMetadata>> {
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

    pub fn entries(&self) -> impl Iterator<Item = (&SourceSymbolKey, &PublicationDbMetadata)> {
        self.by_source_key.iter()
    }
}

#[derive(Debug, Clone)]
pub struct PublicationDbMetadata {
    pub module_path: String,
    pub type_name: String,
    pub canonical_type_name: String,
    pub collection_name: String,
    pub retention: Option<PublicationDbRetention>,
    pub leases: BTreeMap<String, PublicationDbLease>,
    pub key: PublicationDbObjectKey,
    pub fields: BTreeSet<String>,
    pub field_types: BTreeMap<String, TypeRef>,
    pub field_type_texts: BTreeMap<String, String>,
}

impl PublicationDbMetadata {
    pub fn object_symbol(&self) -> ServiceSymbolRef {
        ServiceSymbolRef {
            module_path: self.module_path.clone(),
            symbol: self.type_name.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PublicationDbObjectKey {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone)]
pub struct PublicationDbRetention {
    pub amount: u64,
    pub unit: DbRetentionUnit,
}

#[derive(Debug, Clone)]
pub struct PublicationDbLease {
    pub name: String,
    pub ttl_ms: u64,
    pub max_ms: Option<u64>,
}

pub fn publication_db_metadata_index<'a>(
    sources: impl IntoIterator<Item = (&'a str, &'a SourceFile)>,
    _package_aliases: &BTreeMap<String, Vec<String>>,
    _external_type_symbols: &PublicationTypeSymbolIndex,
) -> Result<PublicationDbMetadataIndex> {
    let mut index = PublicationDbMetadataIndex::default();
    for (module_path, ast) in sources {
        let attachments = DbAttachmentIndex::build(module_path, ast)?;
        for attachment in attachments.iter() {
            let metadata =
                publication_db_metadata(attachment.module_path, ast, attachment.db.name.as_str())?;
            index.insert(module_path, &attachment.db.name, metadata);
        }
    }
    Ok(index)
}

fn publication_db_metadata(
    module_path: &str,
    ast: &SourceFile,
    db_name: &str,
) -> Result<PublicationDbMetadata> {
    let attachments = DbAttachmentIndex::build(module_path, ast)?;
    let attachment = attachments
        .iter()
        .find(|attachment| attachment.db.name == db_name)
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "missing db attachment metadata for {module_path}.{db_name}"
            ))
        })?;
    let db = attachment.db;
    let key_field = attachment.key;
    let key = PublicationDbObjectKey {
        name: key_field.name.clone(),
        ty: key_field.ty.clone(),
    };
    let mut db_field_names = BTreeSet::new();
    let mut field_types = BTreeMap::new();
    let mut field_type_texts = BTreeMap::new();
    db_field_names.insert(key.name.clone());
    field_types.insert(key.name.clone(), key.ty.clone());
    field_type_texts.insert(key_field.name.clone(), key_field.ty.name.clone());
    for field in attachment.fields() {
        db_field_names.insert(field.name.clone());
        field_types.insert(field.name.clone(), field.ty.clone());
        field_type_texts.insert(field.name.clone(), field.ty.name.clone());
    }
    let collection_name = db_collection_name(db);
    validate_db_collection_name(&collection_name, &db.name)?;
    let retention = db
        .retention
        .as_ref()
        .map(|retention| PublicationDbRetention {
            amount: retention.amount,
            unit: retention.unit,
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
            Ok((
                lease.name.clone(),
                PublicationDbLease {
                    name: lease.name.clone(),
                    ttl_ms: lease.ttl_ms,
                    max_ms: lease.max_ms,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(PublicationDbMetadata {
        module_path: module_path.to_string(),
        type_name: db.name.clone(),
        canonical_type_name: canonical_db_type_name(module_path, &db.name),
        collection_name,
        retention,
        leases,
        key,
        fields: db_field_names,
        field_types,
        field_type_texts,
    })
}

fn validate_db_collection_name(collection_name: &str, db_name: &str) -> Result<()> {
    if collection_name.starts_with("_skiff_") {
        return Err(CompileError::Semantic(format!(
            "db object {db_name} collection name {collection_name:?} uses reserved _skiff_ system namespace"
        )));
    }
    Ok(())
}

fn canonical_db_type_name(module_path: &str, db_name: &str) -> String {
    if db_name.contains('.') {
        db_name.to_string()
    } else {
        format!("{module_path}.{db_name}")
    }
}

fn db_source_key(module_path: &str, db_name: &str) -> SourceSymbolKey {
    source_symbol_key_from_qualified_text(db_name)
        .unwrap_or_else(|| SourceSymbolKey::new(module_path, db_name))
}

fn source_symbol_key_from_qualified_text(name: &str) -> Option<SourceSymbolKey> {
    let name = name.trim();
    let name = name.strip_prefix("root.").unwrap_or(name);
    let (module_path, symbol) = name.rsplit_once('.')?;
    Some(SourceSymbolKey::new(module_path, symbol))
}

pub fn type_text_with_args(type_name: &str, type_args: &[TypeRef]) -> String {
    if type_args.is_empty() {
        return type_name.to_string();
    }
    let args = type_args
        .iter()
        .map(|ty| ty.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{type_name}<{args}>")
}

pub fn type_indices(ast: &SourceFile) -> BTreeMap<String, u32> {
    let mut indices = BTreeMap::new();
    for ty in &ast.types {
        indices.insert(ty.name.clone(), indices.len() as u32);
    }
    for alias in &ast.aliases {
        indices.insert(alias.name.clone(), indices.len() as u32);
    }
    for interface in &ast.interfaces {
        indices.insert(interface.name.clone(), indices.len() as u32);
    }
    indices
}
