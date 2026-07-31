use std::{collections::BTreeMap, fmt};

use crate::{
    compile_requirements::ServiceCallRef,
    executable::{CallTargetIr, ExprIr, StmtIr, TestEffectRegisterTargetIr},
};

use super::{
    file_ir_expressions, validate_file_ir_type_refs, FileIrExpressionOwner,
    FileIrTypeRefValidationError, FileIrUnit, ServiceCallRefIndex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIrServiceCallOwner {
    Constant { constant_index: usize },
    Executable { executable_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIrServiceCallSite {
    pub owner: FileIrServiceCallOwner,
    pub expression_index: usize,
    pub service_call_ref_index: ServiceCallRefIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileIrServiceCallValidationError {
    InvalidTypeRef(FileIrTypeRefValidationError),
    DuplicateRef {
        first_index: usize,
        duplicate_index: usize,
    },
    IndexOutOfRange {
        site: FileIrServiceCallSite,
        table_len: usize,
    },
    OrphanRef {
        index: usize,
    },
}

impl fmt::Display for FileIrServiceCallValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTypeRef(error) => write!(formatter, "File IR type ref is invalid: {error}"),
            Self::DuplicateRef {
                first_index,
                duplicate_index,
            } => write!(
                formatter,
                "serviceCallRefs entries {first_index} and {duplicate_index} have the same slot, operation id, and protocol identity"
            ),
            Self::IndexOutOfRange { site, table_len } => write!(
                formatter,
                "service call at {:?} expression {} references index {} but serviceCallRefs has length {table_len}",
                site.owner,
                site.expression_index,
                site.service_call_ref_index.index()
            ),
            Self::OrphanRef { index } => write!(
                formatter,
                "serviceCallRefs entry {index} is not referenced by any ServiceCall target"
            ),
        }
    }
}

impl std::error::Error for FileIrServiceCallValidationError {}

/// Enumerates every canonical service call instruction in a File IR unit.
pub fn file_ir_service_call_sites(
    unit: &FileIrUnit,
) -> impl Iterator<Item = FileIrServiceCallSite> + '_ {
    file_ir_expressions(unit).filter_map(|(owner, expression_index, expression)| {
        let ExprIr::Call { call } = expression else {
            return None;
        };
        let CallTargetIr::ServiceCall {
            service_call_ref_index,
        } = &call.target
        else {
            return None;
        };
        Some(FileIrServiceCallSite {
            owner: match owner {
                FileIrExpressionOwner::Constant { constant_index } => {
                    FileIrServiceCallOwner::Constant { constant_index }
                }
                FileIrExpressionOwner::Executable { executable_index } => {
                    FileIrServiceCallOwner::Executable { executable_index }
                }
            },
            expression_index,
            service_call_ref_index: *service_call_ref_index,
        })
    })
}

/// Validates the complete owner-local table/index relationship once. Identity,
/// lowering, projection and package closure checks must reuse this owner.
pub fn validate_file_ir_service_calls(
    unit: &FileIrUnit,
) -> Result<(), FileIrServiceCallValidationError> {
    validate_file_ir_type_refs(unit).map_err(FileIrServiceCallValidationError::InvalidTypeRef)?;
    validate_unique_refs(&unit.external_refs.service_call_refs)?;

    let mut used = vec![false; unit.external_refs.service_call_refs.len()];
    for site in file_ir_service_call_sites(unit) {
        let index = site.service_call_ref_index.index() as usize;
        let Some(referenced) = used.get_mut(index) else {
            return Err(FileIrServiceCallValidationError::IndexOutOfRange {
                site,
                table_len: used.len(),
            });
        };
        *referenced = true;
    }
    for (executable_index, executable) in unit.executables.iter().enumerate() {
        for statement in &executable.body.statements {
            let StmtIr::TestEffectRegister {
                target:
                    TestEffectRegisterTargetIr::ContractOperation {
                        service_call_ref_index,
                    },
                ..
            } = statement
            else {
                continue;
            };
            let index = service_call_ref_index.index() as usize;
            let Some(referenced) = used.get_mut(index) else {
                return Err(FileIrServiceCallValidationError::IndexOutOfRange {
                    site: FileIrServiceCallSite {
                        owner: FileIrServiceCallOwner::Executable { executable_index },
                        expression_index: 0,
                        service_call_ref_index: *service_call_ref_index,
                    },
                    table_len: used.len(),
                });
            };
            *referenced = true;
        }
    }
    if let Some(index) = used.iter().position(|referenced| !referenced) {
        return Err(FileIrServiceCallValidationError::OrphanRef { index });
    }
    Ok(())
}

/// Returns the validated File IR contribution that downstream PackageArtifact
/// closure checks can union and compare with the package-level aggregate.
pub fn validated_file_ir_service_call_refs(
    unit: &FileIrUnit,
) -> Result<&[ServiceCallRef], FileIrServiceCallValidationError> {
    validate_file_ir_service_calls(unit)?;
    Ok(&unit.external_refs.service_call_refs)
}

fn validate_unique_refs(refs: &[ServiceCallRef]) -> Result<(), FileIrServiceCallValidationError> {
    let mut first_by_tuple = BTreeMap::new();
    for (index, reference) in refs.iter().enumerate() {
        let key = (
            reference.service_requirement_slot,
            reference.contract_operation_id.as_str(),
            reference.expected_protocol_identity.as_str(),
        );
        if let Some(first_index) = first_by_tuple.insert(key, index) {
            return Err(FileIrServiceCallValidationError::DuplicateRef {
                first_index,
                duplicate_index: index,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
