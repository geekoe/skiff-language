use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{compile_identity::PackageSchemaTypeId, types::TypeRefIr};

/// A literal value whose exact payload participates in a ServiceContract type.
///
/// V2 intentionally exposes only string literals. Other literal domains must
/// not become boundary wire protocols until their canonical representation is
/// defined explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractLiteral {
    String { value: String },
}

/// A type reference inside a ServiceContract boundary schema.
///
/// Named boundary references retain their declaring Package owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractTypeRef {
    Builtin {
        name: String,
        arguments: Vec<ContractTypeRef>,
    },
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    AnyInterface {
        interface: Box<ContractTypeRef>,
        arguments: Vec<ContractTypeRef>,
    },
    TypeParam {
        name: String,
    },
    Record {
        fields: BTreeMap<String, ContractTypeRef>,
    },
    StructuralUnion {
        variants: Vec<ContractTypeRef>,
    },
    Nullable {
        inner: Box<ContractTypeRef>,
    },
    Literal {
        value: ContractLiteral,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaTypeRef {
    pub package_id: String,
    pub stable_schema_key: String,
    pub package_schema_type_id: PackageSchemaTypeId,
}

/// Returns every named Package schema reference reachable directly from one
/// canonical descriptor. The caller owns graph traversal across records.
pub fn package_schema_descriptor_refs(
    descriptor: &ContractTypeDescriptor,
) -> BTreeSet<PackageSchemaTypeRef> {
    let mut refs = BTreeSet::new();
    collect_descriptor_package_schema_refs(descriptor, &mut refs);
    refs
}

fn collect_descriptor_package_schema_refs(
    descriptor: &ContractTypeDescriptor,
    refs: &mut BTreeSet<PackageSchemaTypeRef>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields
                .values()
                .for_each(|ty| collect_type_package_schema_refs(ty, refs));
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants
                .iter()
                .for_each(|ty| collect_type_package_schema_refs(ty, refs));
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|branch| collect_type_package_schema_refs(&branch.branch_type, refs)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            collect_type_package_schema_refs(target, refs);
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                operation
                    .parameters
                    .iter()
                    .for_each(|ty| collect_type_package_schema_refs(ty, refs));
                collect_type_package_schema_refs(&operation.return_type, refs);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_package_schema_refs(
    ty: &ContractTypeRef,
    refs: &mut BTreeSet<PackageSchemaTypeRef>,
) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            refs.insert(PackageSchemaTypeRef {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            });
        }
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments
            .iter()
            .for_each(|child| collect_type_package_schema_refs(child, refs)),
        ContractTypeRef::Record { fields } => fields
            .values()
            .for_each(|child| collect_type_package_schema_refs(child, refs)),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_package_schema_refs(interface, refs);
            arguments
                .iter()
                .for_each(|child| collect_type_package_schema_refs(child, refs));
        }
        ContractTypeRef::Nullable { inner } => collect_type_package_schema_refs(inner, refs),
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}

impl ContractTypeRef {
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin {
            name: name.into(),
            arguments: Vec::new(),
        }
    }

    pub fn package_schema(
        package_id: impl Into<String>,
        stable_schema_key: impl Into<String>,
        package_schema_type_id: PackageSchemaTypeId,
    ) -> Self {
        Self::PackageSchema {
            package_id: package_id.into(),
            stable_schema_key: stable_schema_key.into(),
            package_schema_type_id,
        }
    }

    pub fn structural_union(variants: Vec<Self>) -> Self {
        Self::StructuralUnion { variants }
    }

    pub fn string_literal(value: impl Into<String>) -> Self {
        Self::Literal {
            value: ContractLiteral::String {
                value: value.into(),
            },
        }
    }
}

/// Package-local callable signatures have their own type domain. A contract
/// nominal reference is therefore explicit and cannot be confused with an
/// AbiTypeId or hidden inside a legacy TypeRefIr display string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PackageTypeRef {
    Local {
        local_type: TypeRefIr,
    },
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    /// An existential value implementing the exact interface named by
    /// `interface`.
    ///
    /// The interface remains a PackageTypeRef instead of being flattened to a
    /// display name so PackageSchema owner and type identity survive the wire
    /// format and identity hashing.
    AnyInterface {
        interface: Box<PackageTypeRef>,
        arguments: Vec<PackageTypeRef>,
    },
    Container {
        name: String,
        arguments: Vec<PackageTypeRef>,
    },
    Nullable {
        inner: Box<PackageTypeRef>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTypeNameability {
    PublicNameable,
    ClosureOnly,
}

/// One stable tag-to-type entry in a named discriminated union.
///
/// The definition compiler canonicalizes the surrounding list by `tag`; the
/// materialized ServiceContract validator rejects duplicates and non-canonical
/// order instead of silently rewriting loaded artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractDiscriminatedUnionBranch {
    pub tag: String,
    pub branch_type: ContractTypeRef,
}

impl ContractDiscriminatedUnionBranch {
    pub fn new(tag: impl Into<String>, branch_type: ContractTypeRef) -> Self {
        Self {
            tag: tag.into(),
            branch_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractTypeDescriptor {
    Record {
        fields: BTreeMap<String, ContractTypeRef>,
    },
    StructuralUnion {
        variants: Vec<ContractTypeRef>,
    },
    DiscriminatedUnion {
        discriminator_field: String,
        branches: Vec<ContractDiscriminatedUnionBranch>,
    },
    Representation {
        target: ContractTypeRef,
    },
    Alias {
        target: ContractTypeRef,
    },
    Enumeration {
        variants: Vec<String>,
    },
    CallbackInterface {
        operations: BTreeMap<String, BoundaryCallbackOperation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryCallbackOperation {
    pub parameters: Vec<ContractTypeRef>,
    pub return_type: ContractTypeRef,
}

/// Reusable canonical semantic body for a package schema entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractTypeShape {
    pub nameability: ContractTypeNameability,
    pub type_params: Vec<String>,
    pub descriptor: ContractTypeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaCanonicalDescriptor {
    pub type_params: Vec<String>,
    pub descriptor: ContractTypeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaTypeRecord {
    pub package_id: String,
    pub stable_schema_key: String,
    pub package_schema_type_id: PackageSchemaTypeId,
    pub canonical_descriptor: PackageSchemaCanonicalDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaIndexEntry {
    pub package_schema_type_id: PackageSchemaTypeId,
    pub public_path: Option<String>,
    pub nameability: ContractTypeNameability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaIndex {
    pub package_id: String,
    pub package_schema_index_identity: crate::PackageSchemaIndexIdentity,
    pub types: BTreeMap<String, PackageSchemaIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaIndexRef {
    pub package_id: String,
    pub package_schema_index_identity: crate::PackageSchemaIndexIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSchemaTypeRecordRef {
    pub package_id: String,
    pub package_schema_type_id: PackageSchemaTypeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTypeRequirement {
    pub package_id: String,
    pub required_type_ids: Vec<PackageSchemaTypeId>,
}

#[cfg(test)]
mod tests;
