use std::{collections::BTreeMap, fmt};

use crate::{
    compile_identity::PackageCallableId,
    executable::{CallTargetIr, ExprIr},
    symbols::{PackageCallableRef, PackageRefIr},
};

use super::{file_ir_expressions, FileIrExpressionOwner, FileIrUnit};

/// Identifies the File IR body that owns a package-call instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIrPackageCallOwner {
    Constant { constant_index: usize },
    Executable { executable_index: usize },
}

/// A canonical package-call instruction and its owner-local coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIrPackageCallSite {
    pub owner: FileIrPackageCallOwner,
    pub expression_index: usize,
    pub package_ref: PackageRefIr,
    pub package_callable_id: PackageCallableId,
}

/// Fail-closed inconsistencies between package-call instructions and their
/// owner-local `packageCallables` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileIrPackageCallValidationError {
    MissingRef {
        site: FileIrPackageCallSite,
    },
    OrphanRef {
        index: usize,
    },
    FieldMismatch {
        site: FileIrPackageCallSite,
        matching_package_ref_index: Option<usize>,
        matching_callable_id_index: Option<usize>,
    },
    DuplicateRef {
        first_index: usize,
        duplicate_index: usize,
    },
}

impl fmt::Display for FileIrPackageCallValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRef { site } => write!(
                formatter,
                "package call at {:?} expression {} has no matching packageCallables entry",
                site.owner, site.expression_index
            ),
            Self::OrphanRef { index } => write!(
                formatter,
                "packageCallables entry {index} is not referenced by any PackageCallable target"
            ),
            Self::FieldMismatch {
                site,
                matching_package_ref_index,
                matching_callable_id_index,
            } => write!(
                formatter,
                "package call at {:?} expression {} matches packageCallables by packageRef at {matching_package_ref_index:?} and by packageCallableId at {matching_callable_id_index:?}, but has no exact entry",
                site.owner, site.expression_index
            ),
            Self::DuplicateRef {
                first_index,
                duplicate_index,
            } => write!(
                formatter,
                "packageCallables entries {first_index} and {duplicate_index} have the same packageRef and packageCallableId"
            ),
        }
    }
}

impl std::error::Error for FileIrPackageCallValidationError {}

/// Enumerates every canonical package-call instruction in a File IR unit.
pub fn file_ir_package_call_sites(
    unit: &FileIrUnit,
) -> impl Iterator<Item = FileIrPackageCallSite> + '_ {
    file_ir_expressions(unit).filter_map(|(owner, expression_index, expression)| {
        let ExprIr::Call { call } = expression else {
            return None;
        };
        let CallTargetIr::PackageCallable {
            package_ref,
            package_callable_id,
        } = &call.target
        else {
            return None;
        };
        Some(FileIrPackageCallSite {
            owner: match owner {
                FileIrExpressionOwner::Constant { constant_index } => {
                    FileIrPackageCallOwner::Constant { constant_index }
                }
                FileIrExpressionOwner::Executable { executable_index } => {
                    FileIrPackageCallOwner::Executable { executable_index }
                }
            },
            expression_index,
            package_ref: package_ref.clone(),
            package_callable_id: package_callable_id.clone(),
        })
    })
}

/// Validates exact set equality between package-call instruction keys and the
/// owner-local `packageCallables` table. Repeated call sites may share one
/// table entry.
pub fn validate_file_ir_package_calls(
    unit: &FileIrUnit,
) -> Result<(), FileIrPackageCallValidationError> {
    let mut index_by_key = BTreeMap::new();
    for (index, reference) in unit.external_refs.package_callables.iter().enumerate() {
        let key = PackageCallKey::new(&reference.package_ref, &reference.package_callable_id);
        if let Some(first_index) = index_by_key.insert(key, index) {
            return Err(FileIrPackageCallValidationError::DuplicateRef {
                first_index,
                duplicate_index: index,
            });
        }
    }

    let mut used = vec![false; unit.external_refs.package_callables.len()];
    for site in file_ir_package_call_sites(unit) {
        let key = PackageCallKey::new(&site.package_ref, &site.package_callable_id);
        if let Some(index) = index_by_key.get(&key).copied() {
            used[index] = true;
            continue;
        }

        let matching_package_ref_index = unit
            .external_refs
            .package_callables
            .iter()
            .position(|reference| reference.package_ref == site.package_ref);
        let matching_callable_id_index = unit
            .external_refs
            .package_callables
            .iter()
            .position(|reference| reference.package_callable_id == site.package_callable_id);
        if matching_package_ref_index.is_some() || matching_callable_id_index.is_some() {
            return Err(FileIrPackageCallValidationError::FieldMismatch {
                site,
                matching_package_ref_index,
                matching_callable_id_index,
            });
        }
        return Err(FileIrPackageCallValidationError::MissingRef { site });
    }

    if let Some(index) = used.iter().position(|referenced| !referenced) {
        return Err(FileIrPackageCallValidationError::OrphanRef { index });
    }
    Ok(())
}

/// Returns the package-call table after validating its complete relationship
/// with all package-call instructions in the File IR unit.
pub fn validated_file_ir_package_callable_refs(
    unit: &FileIrUnit,
) -> Result<&[PackageCallableRef], FileIrPackageCallValidationError> {
    validate_file_ir_package_calls(unit)?;
    Ok(&unit.external_refs.package_callables)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PackageRefKey<'a> {
    PackageId(&'a str),
    Dependency(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PackageCallKey<'a> {
    package_ref: PackageRefKey<'a>,
    package_callable_id: &'a str,
}

impl<'a> PackageCallKey<'a> {
    fn new(package_ref: &'a PackageRefIr, package_callable_id: &'a PackageCallableId) -> Self {
        Self {
            package_ref: match package_ref {
                PackageRefIr::PackageId { package_id } => PackageRefKey::PackageId(package_id),
                PackageRefIr::Dependency { dependency_ref } => {
                    PackageRefKey::Dependency(dependency_ref)
                }
            },
            package_callable_id: package_callable_id.as_str(),
        }
    }
}

#[cfg(test)]
mod tests;
