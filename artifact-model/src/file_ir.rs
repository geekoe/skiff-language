use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    compile_requirements::ServiceCallRef,
    executable::{
        visit_executable_body_type_refs, visit_executable_type_refs, ExecutableBody, ExecutableIr,
        ExprIr,
    },
    refs::SourceSpanRef,
    schema::{FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION},
    symbols::{PackageCallableRef, PackageSymbolRef, ServiceDependencySymbolRef, ServiceSymbolRef},
    targets::NativeTarget,
    types::{
        visit_type_descriptor_type_refs, visit_type_ref, InterfaceDeclIr, NominalTypeRefBaseIr,
        TypeDeclIr, TypeDescriptorIr, TypeRefIr,
    },
    ActorDeclarationIr,
};

mod db_indexes;
mod package_calls;
mod service_calls;

pub use db_indexes::{
    is_db_indexable_scalar_builtin, validate_file_ir_db_indexes, FileIrDbIndexValidationError,
    DB_INDEXABLE_SCALAR_BUILTINS,
};
pub use package_calls::{
    file_ir_package_call_sites, validate_file_ir_package_calls,
    validated_file_ir_package_callable_refs, FileIrPackageCallOwner, FileIrPackageCallSite,
    FileIrPackageCallValidationError,
};

pub use service_calls::{
    file_ir_service_call_sites, validate_file_ir_service_calls,
    validated_file_ir_service_call_refs, FileIrServiceCallOwner, FileIrServiceCallSite,
    FileIrServiceCallValidationError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIrTypeRefValidationError {
    pub location: String,
    pub message: String,
}

impl std::fmt::Display for FileIrTypeRefValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.location, self.message)
    }
}

impl std::error::Error for FileIrTypeRefValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileIrExpressionOwner {
    Constant { constant_index: usize },
    Executable { executable_index: usize },
}

pub(super) fn file_ir_expressions(
    unit: &FileIrUnit,
) -> impl Iterator<Item = (FileIrExpressionOwner, usize, &ExprIr)> {
    let constants =
        unit.constants
            .iter()
            .enumerate()
            .flat_map(|(index, value)| {
                value.body.expressions.iter().enumerate().map(
                    move |(expression_index, expression)| {
                        (
                            FileIrExpressionOwner::Constant {
                                constant_index: index,
                            },
                            expression_index,
                            expression,
                        )
                    },
                )
            });
    let executables = unit
        .executables
        .iter()
        .enumerate()
        .flat_map(|(index, executable)| {
            executable.body.expressions.iter().enumerate().map(
                move |(expression_index, expression)| {
                    (
                        FileIrExpressionOwner::Executable {
                            executable_index: index,
                        },
                        expression_index,
                        expression,
                    )
                },
            )
        });
    constants.chain(executables)
}

pub const FILE_IR_SOURCE_MAP_FORMAT: &str = "skiff-file-ir-source-map-v1";
// Retired canonical spellings remain only as admission tombstones. They must
// never be lowered to another builtin/native identity.
const RETIRED_FILE_IR_BUILTIN_TYPES: &[&str] = &["CancelError"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIrUnit {
    pub schema_version: String,
    pub file_ir_identity: String,
    pub source_ast_hash: String,
    pub module_path: String,
    pub ir_format_version: String,
    pub opcode_table_version: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub required_receiver_builtin_capability_version: u32,
    pub source_map: SourceMapDto,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_declarations: Vec<ActorDeclarationIr>,
    pub declarations: FileDeclarations,
    pub link_targets: FileLinkTargets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_table: Vec<TypeDeclIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<ConstIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<ExecutableIr>,
    pub external_refs: ExternalRefTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapDto {
    pub format: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceMapSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<SourceMapSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapSource {
    pub id: u64,
    pub path: String,
    pub module_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ast_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapSpan {
    pub id: u64,
    pub source: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub span: SourceSpanRef,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileDeclarations {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeDeclarationIr>,
    pub interfaces: BTreeMap<String, InterfaceDeclIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub db: BTreeMap<String, DbDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableDeclarationIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstDeclarationIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeDeclarationIr {
    pub type_index: u32,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableDeclarationIr {
    pub executable_index: u32,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstDeclarationIr {
    pub const_index: u32,
    pub symbol: String,
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstIr {
    pub name: String,
    pub ty: TypeRefIr,
    pub body: ExecutableBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbDeclarationIr {
    pub type_ref: TypeRefIr,
    pub type_name: String,
    pub collection_name: String,
    pub kind: DbObjectKindIr,
    pub key: DbObjectKeyIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<DbObjectFieldIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<DbRetentionIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<DbLeaseIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<DbIndexIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbObjectKindIr {
    #[default]
    Object,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbObjectKeyIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbObjectFieldIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "DbFieldStorageIr::is_identity")]
    pub storage: DbFieldStorageIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbFieldStorageIr {
    #[default]
    Identity,
    Encrypted,
}

impl DbFieldStorageIr {
    pub fn is_identity(&self) -> bool {
        *self == Self::Identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbRetentionIr {
    pub amount: u64,
    pub unit: DbRetentionUnitIr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbLeaseIr {
    pub name: String,
    pub ttl_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbRetentionUnitIr {
    Days,
    Hours,
    Minutes,
    Seconds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldPathIr {
    pub text: String,
    pub segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbIndexIr {
    pub name: String,
    pub unique: bool,
    pub fields: Vec<DbIndexFieldIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DbIndexFieldIr {
    pub field: FieldPathIr,
    pub direction: DbIndexDirectionIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DbIndexDirectionIr {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileLinkTargets {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeLinkTargetIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, ExecutableLinkTargetIr>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstLinkTargetIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeLinkTargetIr {
    pub type_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableLinkTargetIr {
    pub executable_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstLinkTargetIr {
    pub const_index: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalRefTable {
    /// Canonical service-call facts. ServiceCall instructions reference this
    /// table by ServiceCallRefIndex and never inline a second copy.
    pub service_call_refs: Vec<ServiceCallRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_symbols: Vec<ServiceSymbolRef>,
    /// Legacy runtime adapter refs. Canonical package lowering uses
    /// service_call_refs plus CallTargetIr::ServiceCall instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_dependency_symbols: Vec<ServiceDependencySymbolRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_symbols: Vec<PackageSymbolRef>,
    /// Owner-local package direct-call identities. Local ABI expectations are
    /// carried by package requirements rather than repeated here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_callables: Vec<PackageCallableRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_targets: Vec<NativeTarget>,
}

/// Owner-local index into `FileIrUnit.externalRefs.serviceCallRefs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServiceCallRefIndex(u32);

impl ServiceCallRefIndex {
    pub fn new(index: u32) -> Self {
        Self(index)
    }

    pub fn index(self) -> u32 {
        self.0
    }
}

impl TryFrom<usize> for ServiceCallRefIndex {
    type Error = std::num::TryFromIntError;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        Ok(Self(u32::try_from(index)?))
    }
}

impl ExternalRefTable {
    pub fn service_call_ref(&self, index: ServiceCallRefIndex) -> Option<&ServiceCallRef> {
        self.service_call_refs.get(index.index() as usize)
    }
}

impl FileIrUnit {
    pub fn empty(module_path: impl Into<String>, source_ast_hash: impl Into<String>) -> Self {
        Self {
            schema_version: FILE_IR_SCHEMA_VERSION.to_string(),
            file_ir_identity: String::new(),
            source_ast_hash: source_ast_hash.into(),
            module_path: module_path.into(),
            ir_format_version: FILE_IR_FORMAT_VERSION.to_string(),
            opcode_table_version: FILE_IR_OPCODE_TABLE_VERSION.to_string(),
            required_receiver_builtin_capability_version: 0,
            source_map: SourceMapDto::empty(),
            actor_declarations: Vec::new(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            type_table: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        }
    }
}

/// Validates every TypeRef-bearing File IR surface against the declaration and
/// generic scope that owns it. External nominal owners retain their exact
/// locator for dependency/link validation; no name or shape inference occurs.
pub fn validate_file_ir_type_refs(unit: &FileIrUnit) -> Result<(), FileIrTypeRefValidationError> {
    for (type_index, declaration) in unit.type_table.iter().enumerate() {
        let location = format!("typeTable[{type_index}]");
        let scope = declaration.type_params.as_slice();
        visit_type_descriptor_type_refs(&declaration.descriptor, &mut |ty| {
            validate_type_ref_node(unit, ty, scope, &location)
        })?;
        for implemented in &declaration.implements {
            visit_type_ref(implemented, &mut |ty| {
                validate_type_ref_node(unit, ty, scope, &location)
            })?;
        }
        if let TypeDescriptorIr::Union { branches } = &declaration.descriptor {
            for (branch_index, branch) in branches.iter().enumerate() {
                if let crate::NamedUnionBranchIr::ConcreteNominal { nominal_type } = branch {
                    if !is_plain_or_applied_nominal(nominal_type) {
                        return type_ref_error(
                            format!("{location}.branches[{branch_index}].nominalType"),
                            "concreteNominal must contain an exact plain or applied nominal",
                        );
                    }
                }
            }
        }
    }

    for (name, interface) in &unit.declarations.interfaces {
        let interface_location = format!("declarations.interfaces[{name}]");
        for operation in &interface.operations {
            let mut scope = interface.type_params.clone();
            scope.extend(operation.type_params.iter().cloned());
            let location = format!("{interface_location}.operations[{}]", operation.name);
            for parameter in &operation.params {
                visit_type_ref(&parameter.ty, &mut |ty| {
                    validate_type_ref_node(unit, ty, &scope, &location)
                })?;
            }
            visit_type_ref(&operation.return_type, &mut |ty| {
                validate_type_ref_node(unit, ty, &scope, &location)
            })?;
            if let Some(implicit_self) = &operation.implicit_self {
                visit_type_ref(implicit_self, &mut |ty| {
                    validate_type_ref_node(unit, ty, &scope, &location)
                })?;
            }
        }
    }

    for (name, declaration) in &unit.declarations.constants {
        let location = format!("declarations.constants[{name}]");
        visit_type_ref(&declaration.ty, &mut |ty| {
            validate_type_ref_node(unit, ty, &[], &location)
        })?;
    }

    for (name, declaration) in &unit.declarations.db {
        let location = format!("declarations.db[{name}]");
        for ty in std::iter::once(&declaration.type_ref)
            .chain(std::iter::once(&declaration.key.ty))
            .chain(declaration.fields.iter().map(|field| &field.ty))
        {
            visit_type_ref(ty, &mut |ty| {
                validate_type_ref_node(unit, ty, &[], &location)
            })?;
        }
    }

    for (index, actor) in unit.actor_declarations.iter().enumerate() {
        let location = format!("actorDeclarations[{index}]");
        for ty in std::iter::once(&actor.abi.actor_id_type)
            .chain(actor.abi.fields.iter().map(|field| &field.ty))
            .chain(
                actor
                    .abi
                    .public_methods
                    .iter()
                    .flat_map(|method| method.parameters.iter().map(|parameter| &parameter.ty)),
            )
            .chain(
                actor
                    .abi
                    .public_methods
                    .iter()
                    .map(|method| &method.return_type),
            )
        {
            visit_type_ref(ty, &mut |ty| {
                validate_type_ref_node(unit, ty, &[], &location)
            })?;
        }
    }

    for (index, constant) in unit.constants.iter().enumerate() {
        let location = format!("constants[{index}]");
        visit_type_ref(&constant.ty, &mut |ty| {
            validate_type_ref_node(unit, ty, &[], &location)
        })?;
        visit_executable_body_type_refs(&constant.body, &mut |ty| {
            validate_type_ref_node(unit, ty, &[], &location)
        })?;
    }

    for (index, executable) in unit.executables.iter().enumerate() {
        let location = format!("executables[{index}]");
        visit_executable_type_refs(executable, &mut |ty| {
            validate_type_ref_node(unit, ty, &executable.type_params, &location)
        })?;
    }

    validate_representation_wraps(unit)?;

    Ok(())
}

fn validate_representation_wraps(unit: &FileIrUnit) -> Result<(), FileIrTypeRefValidationError> {
    for (owner, expression_index, expression) in file_ir_expressions(unit) {
        let ExprIr::RepresentationWrap { value, type_ref } = expression else {
            continue;
        };
        let (location, expression_count) = match owner {
            FileIrExpressionOwner::Constant { constant_index } => (
                format!("constants[{constant_index}].body.expressions[{expression_index}]"),
                unit.constants[constant_index].body.expressions.len(),
            ),
            FileIrExpressionOwner::Executable { executable_index } => (
                format!("executables[{executable_index}].body.expressions[{expression_index}]"),
                unit.executables[executable_index].body.expressions.len(),
            ),
        };
        if value.expression as usize >= expression_count {
            return type_ref_error(
                format!("{location}.value"),
                format!(
                    "representationWrap child expression {} does not exist in owner body with {expression_count} expressions",
                    value.expression
                ),
            );
        }
        validate_representation_wrap_target(unit, type_ref, &format!("{location}.typeRef"))?;
        visit_type_ref(type_ref, &mut |nested| {
            if let TypeRefIr::TypeParam { name } = nested {
                return type_ref_error(
                    format!("{location}.typeRef"),
                    format!("representationWrap target retains unresolved type parameter {name}"),
                );
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn validate_representation_wrap_target(
    unit: &FileIrUnit,
    type_ref: &TypeRefIr,
    location: &str,
) -> Result<(), FileIrTypeRefValidationError> {
    match type_ref {
        TypeRefIr::LocalType { type_index } => {
            validate_representation_declaration(unit, *type_index, None, location)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } if module_path == &unit.module_path => {
            validate_representation_declaration(unit, *type_index, None, location)
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            if arguments.is_empty() {
                return type_ref_error(
                    location,
                    "representationWrap applied target arguments must be non-empty",
                );
            }
            match base {
                NominalTypeRefBaseIr::LocalType { type_index } => {
                    validate_representation_declaration(
                        unit,
                        *type_index,
                        Some(arguments.len()),
                        location,
                    )
                }
                NominalTypeRefBaseIr::PublicationType {
                    module_path,
                    type_index,
                } if module_path == &unit.module_path => {
                    validate_representation_declaration(
                        unit,
                        *type_index,
                        Some(arguments.len()),
                        location,
                    )
                }
                NominalTypeRefBaseIr::PackageSchema { .. } => type_ref_error(
                    location,
                    "applied PackageSchema is not admitted in this artifact generation",
                ),
                NominalTypeRefBaseIr::PublicationType { .. }
                | NominalTypeRefBaseIr::ServiceSymbol { .. }
                | NominalTypeRefBaseIr::PackageSymbol { .. } => type_ref_error(
                    location,
                    "representationWrap applied target base cannot be resolved to an exact File IR representation declaration",
                ),
            }
        }
        TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. } => type_ref_error(
            location,
            "representationWrap target cannot be resolved to an exact File IR representation declaration",
        ),
        TypeRefIr::Builtin { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => type_ref_error(
            location,
            "representationWrap target must be a plain or applied nominal representation",
        ),
    }
}

fn validate_representation_declaration(
    unit: &FileIrUnit,
    type_index: u32,
    applied_arity: Option<usize>,
    location: &str,
) -> Result<(), FileIrTypeRefValidationError> {
    let Some(declaration) = unit.type_table.get(type_index as usize) else {
        return type_ref_error(
            location,
            format!("representationWrap target type index {type_index} does not exist"),
        );
    };
    if !matches!(
        declaration.descriptor,
        TypeDescriptorIr::Representation { .. }
    ) {
        let actual = match declaration.descriptor {
            TypeDescriptorIr::Record { .. } => "record",
            TypeDescriptorIr::Representation { .. } => unreachable!(),
            TypeDescriptorIr::Union { .. } => "union",
            TypeDescriptorIr::Alias { .. } => "alias",
            TypeDescriptorIr::Interface => "interface",
        };
        return type_ref_error(
            location,
            format!(
                "representationWrap target type index {type_index} is {actual}, not representation"
            ),
        );
    }
    match applied_arity {
        Some(actual) => {
            let expected = declaration.type_params.len();
            if expected == 0 || actual != expected {
                return type_ref_error(
                    location,
                    format!(
                        "representationWrap applied target type index {type_index} has arity {actual}, expected {expected}"
                    ),
                );
            }
        }
        None if !declaration.type_params.is_empty() => {
            return type_ref_error(
                location,
                format!(
                    "representationWrap plain target type index {type_index} requires {} type arguments",
                    declaration.type_params.len()
                ),
            );
        }
        None => {}
    }
    Ok(())
}

fn validate_type_ref_node(
    unit: &FileIrUnit,
    ty: &TypeRefIr,
    scope: &[String],
    location: &str,
) -> Result<(), FileIrTypeRefValidationError> {
    match ty {
        TypeRefIr::Builtin { name, .. }
            if RETIRED_FILE_IR_BUILTIN_TYPES.contains(&name.as_str()) =>
        {
            return type_ref_error(
                location,
                format!("retired File IR builtin type {name} is not admitted"),
            );
        }
        TypeRefIr::LocalType { type_index } => {
            validate_local_nominal(unit, *type_index, None, location)?;
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } if module_path == &unit.module_path => {
            validate_local_nominal(unit, *type_index, None, location)?;
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            if arguments.is_empty() {
                return type_ref_error(location, "appliedNominal arguments must be non-empty");
            }
            match base {
                NominalTypeRefBaseIr::LocalType { type_index } => {
                    validate_local_nominal(unit, *type_index, Some(arguments.len()), location)?;
                }
                NominalTypeRefBaseIr::PublicationType {
                    module_path,
                    type_index,
                } if module_path == &unit.module_path => {
                    validate_local_nominal(unit, *type_index, Some(arguments.len()), location)?;
                }
                NominalTypeRefBaseIr::PackageSchema { .. } => {
                    return type_ref_error(
                        location,
                        "applied PackageSchema is not admitted in this artifact generation",
                    );
                }
                NominalTypeRefBaseIr::PublicationType { .. }
                | NominalTypeRefBaseIr::ServiceSymbol { .. }
                | NominalTypeRefBaseIr::PackageSymbol { .. } => {
                    // The descriptor is external to this File IR. Preserve the
                    // exact locator for the dependency/link consumer.
                }
            }
        }
        TypeRefIr::TypeParam { name } if !scope.iter().any(|parameter| parameter == name) => {
            return type_ref_error(
                location,
                format!("type parameter {name} is outside the owning declaration scope"),
            );
        }
        TypeRefIr::Builtin { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => {}
    }
    Ok(())
}

fn validate_local_nominal(
    unit: &FileIrUnit,
    type_index: u32,
    applied_arity: Option<usize>,
    location: &str,
) -> Result<(), FileIrTypeRefValidationError> {
    let Some(declaration) = unit.type_table.get(type_index as usize) else {
        return type_ref_error(
            location,
            format!("local nominal type index {type_index} does not exist"),
        );
    };
    if !matches!(
        declaration.descriptor,
        TypeDescriptorIr::Record { .. }
            | TypeDescriptorIr::Representation { .. }
            | TypeDescriptorIr::Union { .. }
    ) {
        return type_ref_error(
            location,
            format!(
                "local nominal type index {type_index} targets alias/interface instead of record, representation, or named union"
            ),
        );
    }
    match applied_arity {
        Some(actual) => {
            let expected = declaration.type_params.len();
            if expected == 0 || actual != expected {
                return type_ref_error(
                    location,
                    format!(
                        "applied local nominal type index {type_index} has arity {actual}, expected {expected}"
                    ),
                );
            }
        }
        None if !declaration.type_params.is_empty() => {
            return type_ref_error(
                location,
                format!(
                    "plain local nominal type index {type_index} requires {} type arguments",
                    declaration.type_params.len()
                ),
            );
        }
        None => {}
    }
    Ok(())
}

fn is_plain_or_applied_nominal(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::AppliedNominal { .. }
    )
}

fn type_ref_error<T>(
    location: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, FileIrTypeRefValidationError> {
    Err(FileIrTypeRefValidationError {
        location: location.into(),
        message: message.into(),
    })
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[cfg(test)]
mod applied_nominal_tests {
    use super::*;
    use crate::{NamedUnionBranchIr, NominalTypeRefBaseIr};

    fn generic_unit() -> FileIrUnit {
        let mut unit = FileIrUnit::empty("main", "source");
        unit.type_table = vec![
            TypeDeclIr {
                name: "Box".to_string(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                type_params: vec!["T".to_string()],
                implements: Vec::new(),
                source_span: None,
            },
            TypeDeclIr {
                name: "Holder".to_string(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::from([(
                        "boxed".to_string(),
                        TypeRefIr::AppliedNominal {
                            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                            arguments: vec![TypeRefIr::TypeParam {
                                name: "U".to_string(),
                            }],
                        },
                    )]),
                },
                type_params: vec!["U".to_string()],
                implements: Vec::new(),
                source_span: None,
            },
            TypeDeclIr {
                name: "Ready".to_string(),
                descriptor: TypeDescriptorIr::Union {
                    branches: vec![NamedUnionBranchIr::ConcreteNominal {
                        nominal_type: TypeRefIr::AppliedNominal {
                            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                            arguments: vec![TypeRefIr::builtin("string")],
                        },
                    }],
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
            TypeDeclIr {
                name: "Alias".to_string(),
                descriptor: TypeDescriptorIr::Alias {
                    target: TypeRefIr::builtin("string"),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
        ];
        unit
    }

    #[test]
    fn file_ir_admits_exact_local_arity_kind_and_scope() {
        let unit = generic_unit();
        validate_file_ir_type_refs(&unit).unwrap();
    }

    #[test]
    fn file_ir_rejects_empty_wrong_arity_kind_plain_generic_and_unbound_scope() {
        let mut empty = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut empty.type_table[1].descriptor else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { arguments, .. } = fields.get_mut("boxed").unwrap() else {
            unreachable!()
        };
        arguments.clear();
        assert!(validate_file_ir_type_refs(&empty)
            .unwrap_err()
            .message
            .contains("non-empty"));

        let mut wrong_arity = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut wrong_arity.type_table[1].descriptor else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { arguments, .. } = fields.get_mut("boxed").unwrap() else {
            unreachable!()
        };
        arguments.push(TypeRefIr::builtin("number"));
        assert!(validate_file_ir_type_refs(&wrong_arity)
            .unwrap_err()
            .message
            .contains("arity 2"));

        let mut alias = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut alias.type_table[1].descriptor else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { base, .. } = fields.get_mut("boxed").unwrap() else {
            unreachable!()
        };
        *base = NominalTypeRefBaseIr::LocalType { type_index: 3 };
        assert!(validate_file_ir_type_refs(&alias)
            .unwrap_err()
            .message
            .contains("alias/interface"));

        let mut plain_generic = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut plain_generic.type_table[1].descriptor
        else {
            unreachable!()
        };
        *fields.get_mut("boxed").unwrap() = TypeRefIr::LocalType { type_index: 0 };
        assert!(validate_file_ir_type_refs(&plain_generic)
            .unwrap_err()
            .message
            .contains("requires 1 type arguments"));

        let mut unbound = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut unbound.type_table[1].descriptor else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { arguments, .. } = fields.get_mut("boxed").unwrap() else {
            unreachable!()
        };
        arguments[0] = TypeRefIr::TypeParam {
            name: "Missing".to_string(),
        };
        assert!(validate_file_ir_type_refs(&unbound)
            .unwrap_err()
            .message
            .contains("outside"));
    }

    #[test]
    fn file_ir_rejects_applied_package_schema_but_preserves_external_locator() {
        let mut unit = generic_unit();
        let TypeDescriptorIr::Record { fields } = &mut unit.type_table[1].descriptor else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { base, .. } = fields.get_mut("boxed").unwrap() else {
            unreachable!()
        };
        *base = NominalTypeRefBaseIr::PackageSchema {
            package_id: "example.com/pkg".to_string(),
            stable_schema_key: "Box".to_string(),
            package_schema_type_id: crate::PackageSchemaTypeId::new("schema:box"),
        };
        assert!(validate_file_ir_type_refs(&unit)
            .unwrap_err()
            .message
            .contains("not admitted"));
    }

    #[test]
    fn file_ir_execution_traversal_reaches_nested_construct_type_arguments() {
        let mut unit = generic_unit();
        unit.constants.push(ConstIr {
            name: "build".to_string(),
            ty: TypeRefIr::builtin("void"),
            body: ExecutableBody {
                expressions: vec![ExprIr::Construct {
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                        arguments: vec![TypeRefIr::builtin("string")],
                    },
                    fields: BTreeMap::new(),
                }],
                ..ExecutableBody::default()
            },
            source_span: None,
        });
        validate_file_ir_type_refs(&unit).unwrap();

        let ExprIr::Construct { type_ref, .. } = &mut unit.constants[0].body.expressions[0] else {
            unreachable!()
        };
        let TypeRefIr::AppliedNominal { arguments, .. } = type_ref else {
            unreachable!()
        };
        arguments.clear();
        assert!(validate_file_ir_type_refs(&unit)
            .unwrap_err()
            .message
            .contains("non-empty"));
    }
}

#[cfg(test)]
mod legacy_builtin_tests;

#[cfg(test)]
mod representation_wrap_tests;

impl SourceMapDto {
    pub fn empty() -> Self {
        Self {
            format: FILE_IR_SOURCE_MAP_FORMAT.to_string(),
            sources: Vec::new(),
            spans: Vec::new(),
        }
    }
}
