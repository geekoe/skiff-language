use std::collections::BTreeMap;

use skiff_artifact_model::{ContractTypeId, ServiceContract};

use crate::{ArtifactIdentityError, Result};

#[derive(Debug)]
pub(super) struct SchemaEdge {
    pub(super) target: ContractTypeId,
    pub(super) path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Complete,
}

pub(super) fn reject_recursive_schema(
    contract: &ServiceContract,
    edges: &BTreeMap<ContractTypeId, Vec<SchemaEdge>>,
) -> Result<()> {
    let mut states = BTreeMap::new();
    for type_id in contract.boundary_schema.keys() {
        visit_type(contract, type_id, edges, &mut states)?;
    }
    Ok(())
}

fn visit_type(
    contract: &ServiceContract,
    type_id: &ContractTypeId,
    edges: &BTreeMap<ContractTypeId, Vec<SchemaEdge>>,
    states: &mut BTreeMap<ContractTypeId, VisitState>,
) -> Result<()> {
    match states.get(type_id) {
        Some(VisitState::Complete) => return Ok(()),
        Some(VisitState::Visiting) => unreachable!("cycle is detected at the incoming edge"),
        None => {}
    }
    states.insert(type_id.clone(), VisitState::Visiting);
    for edge in edges.get(type_id).into_iter().flatten() {
        match states.get(&edge.target) {
            Some(VisitState::Visiting) => {
                let target = contract
                    .boundary_schema
                    .get(&edge.target)
                    .map(|schema| schema.stable_key.as_str())
                    .unwrap_or_else(|| edge.target.as_str());
                return invalid_contract(format!(
                    "{}: recursive contract schema cycle reaches boundarySchema[{target}]",
                    edge.path
                ));
            }
            Some(VisitState::Complete) => {}
            None => visit_type(contract, &edge.target, edges, states)?,
        }
    }
    states.insert(type_id.clone(), VisitState::Complete);
    Ok(())
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}
