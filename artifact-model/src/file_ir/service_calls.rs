use std::{collections::BTreeMap, fmt};

use crate::{
    compile_requirements::ServiceCallRef,
    executable::{CallTargetIr, ExecutableBody, ExprIr},
};

use super::{FileIrUnit, ServiceCallRefIndex};

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
    let constant_sites = unit
        .constants
        .iter()
        .enumerate()
        .flat_map(|(index, value)| {
            service_call_sites_in_body(
                FileIrServiceCallOwner::Constant {
                    constant_index: index,
                },
                &value.body,
            )
        });
    let executable_sites = unit
        .executables
        .iter()
        .enumerate()
        .flat_map(|(index, executable)| {
            service_call_sites_in_body(
                FileIrServiceCallOwner::Executable {
                    executable_index: index,
                },
                &executable.body,
            )
        });
    constant_sites.chain(executable_sites)
}

/// Validates the complete owner-local table/index relationship once. Identity,
/// lowering, projection and package closure checks must reuse this owner.
pub fn validate_file_ir_service_calls(
    unit: &FileIrUnit,
) -> Result<(), FileIrServiceCallValidationError> {
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

fn service_call_sites_in_body(
    owner: FileIrServiceCallOwner,
    body: &ExecutableBody,
) -> impl Iterator<Item = FileIrServiceCallSite> + '_ {
    body.expressions
        .iter()
        .enumerate()
        .filter_map(move |(expression_index, expression)| {
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
                owner,
                expression_index,
                service_call_ref_index: *service_call_ref_index,
            })
        })
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
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::{
        CallIr, ContractOperationId, ExprIr, ExternalRefTable, FileIrUnit, ServiceProtocolIdentity,
    };

    use super::*;

    #[test]
    fn external_ref_table_and_service_call_target_have_strict_wire_shapes() {
        let table = ExternalRefTable::default();
        assert_eq!(
            serde_json::to_value(&table).unwrap(),
            json!({ "serviceCallRefs": [] })
        );
        assert!(serde_json::from_value::<ExternalRefTable>(json!({})).is_err());

        let target = CallTargetIr::ServiceCall {
            service_call_ref_index: ServiceCallRefIndex::new(3),
        };
        let wire = json!({ "kind": "serviceCall", "serviceCallRefIndex": 3 });
        assert_eq!(serde_json::to_value(&target).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<CallTargetIr>(wire).unwrap(),
            target
        );
        for invalid in [
            json!({ "kind": "serviceCall" }),
            json!({
                "kind": "serviceCall",
                "serviceCallRefIndex": 3,
                "serviceCallRef": service_call_ref(0, "echo", "protocol")
            }),
            json!({
                "kind": "serviceCall",
                "serviceCallRefIndex": 3,
                "serviceDependencySymbol": "legacy"
            }),
        ] {
            assert!(serde_json::from_value::<CallTargetIr>(invalid).is_err());
        }
    }

    #[test]
    fn validator_accepts_typed_sites_and_exposes_validated_refs() {
        let mut unit = FileIrUnit::empty("api", "source");
        unit.external_refs.service_call_refs = vec![service_call_ref(2, "echo", "protocol")];
        push_service_call(&mut unit, 0);

        let sites = file_ir_service_call_sites(&unit).collect::<Vec<_>>();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].service_call_ref_index.index(), 0);
        assert_eq!(
            unit.external_refs
                .service_call_ref(sites[0].service_call_ref_index),
            unit.external_refs.service_call_refs.first()
        );
        assert_eq!(
            validated_file_ir_service_call_refs(&unit).unwrap(),
            unit.external_refs.service_call_refs.as_slice()
        );
    }

    #[test]
    fn validator_rejects_out_of_range_duplicate_and_orphan_refs() {
        let mut out_of_range = FileIrUnit::empty("api", "source");
        push_service_call(&mut out_of_range, 0);
        assert!(matches!(
            validate_file_ir_service_calls(&out_of_range),
            Err(FileIrServiceCallValidationError::IndexOutOfRange { .. })
        ));

        let mut duplicate = FileIrUnit::empty("api", "source");
        duplicate.external_refs.service_call_refs = vec![
            service_call_ref(2, "echo", "protocol"),
            service_call_ref(2, "echo", "protocol"),
        ];
        push_service_call(&mut duplicate, 0);
        push_service_call(&mut duplicate, 1);
        assert!(matches!(
            validate_file_ir_service_calls(&duplicate),
            Err(FileIrServiceCallValidationError::DuplicateRef { .. })
        ));

        let mut orphan = FileIrUnit::empty("api", "source");
        orphan.external_refs.service_call_refs = vec![service_call_ref(2, "echo", "protocol")];
        assert_eq!(
            validate_file_ir_service_calls(&orphan),
            Err(FileIrServiceCallValidationError::OrphanRef { index: 0 })
        );
    }

    fn push_service_call(unit: &mut FileIrUnit, index: u32) {
        if unit.constants.is_empty() {
            unit.constants.push(crate::ConstIr {
                name: "calls".to_string(),
                ty: crate::TypeRefIr::native("void"),
                body: crate::ExecutableBody::default(),
                source_span: None,
            });
        }
        unit.constants[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(index),
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    }

    fn service_call_ref(slot: u32, operation: &str, protocol: &str) -> ServiceCallRef {
        ServiceCallRef {
            service_requirement_slot: slot,
            contract_operation_id: ContractOperationId::new(operation),
            expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
        }
    }
}
