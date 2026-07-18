use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryErrorContract, BoundaryOperationContract,
    BoundaryStreamContract, ContractTypeDescriptor, ContractTypeId, ContractTypeRef,
    ContractTypeShape,
};

use crate::{ArtifactIdentityError, Result};

use super::{
    contract_type_id,
    normalization::{normalize_contract_operation_contract, normalize_contract_type_shape},
};

/// Canonicalizes a code-free definition surface and erases transparent aliases.
///
/// Alias stable keys are authoring-only: every usage is expanded to its target
/// before identity derivation, and alias entries are removed from the
/// materialized boundary schema.
pub fn normalize_contract_definition_surface(
    service_id: &str,
    contract_version: &str,
    operations: &mut BTreeMap<String, BoundaryOperationContract>,
    boundary_schema: &mut BTreeMap<String, ContractTypeShape>,
) -> Result<()> {
    for (stable_key, operation) in operations.iter_mut() {
        *operation = normalize_contract_operation_contract(
            operation.clone(),
            &format!("operations[{stable_key}].contract"),
        )?;
    }
    for (stable_key, shape) in boundary_schema.iter_mut() {
        *shape = normalize_contract_type_shape(
            shape.clone(),
            &format!("boundarySchema[{stable_key}].shape"),
        )?;
    }

    let type_ids = boundary_schema
        .keys()
        .map(|stable_key| {
            Ok((
                stable_key.clone(),
                contract_type_id(service_id, contract_version, stable_key)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let known_type_ids = type_ids.values().cloned().collect::<BTreeSet<_>>();
    let alias_targets = boundary_schema
        .iter()
        .filter_map(|(stable_key, shape)| {
            let ContractTypeDescriptor::Alias { target } = &shape.descriptor else {
                return None;
            };
            Some((
                type_ids[stable_key].clone(),
                (stable_key.clone(), target.clone()),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let alias_stable_keys = alias_targets
        .values()
        .map(|(stable_key, _)| stable_key.clone())
        .collect::<BTreeSet<_>>();
    let mut expander = AliasExpander::new(alias_targets, known_type_ids);
    expander.validate_all_aliases()?;

    for (stable_key, operation) in operations.iter_mut() {
        *operation = expander.expand_operation(
            operation.clone(),
            &format!("operations[{stable_key}].contract"),
        )?;
    }
    for (stable_key, shape) in boundary_schema.iter_mut() {
        if alias_stable_keys.contains(stable_key) {
            continue;
        }
        *shape = expander.expand_shape(
            shape.clone(),
            &format!("boundarySchema[{stable_key}].shape"),
        )?;
    }
    boundary_schema.retain(|stable_key, _| !alias_stable_keys.contains(stable_key));
    Ok(())
}

struct AliasExpander {
    alias_targets: BTreeMap<ContractTypeId, (String, ContractTypeRef)>,
    known_type_ids: BTreeSet<ContractTypeId>,
    expanded: BTreeMap<ContractTypeId, ContractTypeRef>,
    visiting: BTreeSet<ContractTypeId>,
}

impl AliasExpander {
    fn new(
        alias_targets: BTreeMap<ContractTypeId, (String, ContractTypeRef)>,
        known_type_ids: BTreeSet<ContractTypeId>,
    ) -> Self {
        Self {
            alias_targets,
            known_type_ids,
            expanded: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    fn validate_all_aliases(&mut self) -> Result<()> {
        let alias_ids = self.alias_targets.keys().cloned().collect::<Vec<_>>();
        for alias_id in alias_ids {
            self.expand_alias(&alias_id)?;
        }
        Ok(())
    }

    fn expand_alias(&mut self, alias_id: &ContractTypeId) -> Result<ContractTypeRef> {
        if let Some(expanded) = self.expanded.get(alias_id) {
            return Ok(expanded.clone());
        }
        let Some((stable_key, target)) = self.alias_targets.get(alias_id).cloned() else {
            return invalid_contract(format!(
                "transparent alias ContractTypeId {alias_id} is absent from the definition"
            ));
        };
        if !self.visiting.insert(alias_id.clone()) {
            return invalid_contract(format!(
                "boundarySchema[{stable_key}].shape.descriptor.target: transparent alias cycle reaches {alias_id}"
            ));
        }
        let path = format!("boundarySchema[{stable_key}].shape.descriptor.target");
        let expanded = self.expand_type_ref(target, &path)?;
        self.visiting.remove(alias_id);
        self.expanded.insert(alias_id.clone(), expanded.clone());
        Ok(expanded)
    }

    fn expand_operation(
        &mut self,
        mut operation: BoundaryOperationContract,
        path: &str,
    ) -> Result<BoundaryOperationContract> {
        for (index, parameter) in operation.parameters.iter_mut().enumerate() {
            parameter.ty = self.expand_type_ref(
                parameter.ty.clone(),
                &format!("{path}.parameters[{index}].ty"),
            )?;
        }
        operation.return_value.ty =
            self.expand_type_ref(operation.return_value.ty, &format!("{path}.returnValue.ty"))?;
        if let BoundaryErrorContract::Typed { payload_type, .. } = &mut operation.errors {
            *payload_type =
                self.expand_type_ref(payload_type.clone(), &format!("{path}.errors.payloadType"))?;
        }
        if let BoundaryStreamContract::ServerStream { item_type, .. } = &mut operation.stream {
            *item_type =
                self.expand_type_ref(item_type.clone(), &format!("{path}.stream.itemType"))?;
        }
        if let BoundaryCallbackContract::RequestScoped {
            interface_type_ids, ..
        } = &mut operation.callbacks
        {
            for (index, type_id) in interface_type_ids.iter_mut().enumerate() {
                *type_id = self.expand_nominal_type_id(
                    type_id.clone(),
                    &format!("{path}.callbacks.interfaceTypeIds[{index}]"),
                )?;
            }
        }
        normalize_contract_operation_contract(operation, path)
    }

    fn expand_shape(
        &mut self,
        mut shape: ContractTypeShape,
        path: &str,
    ) -> Result<ContractTypeShape> {
        shape.descriptor = match shape.descriptor {
            ContractTypeDescriptor::Record { fields } => ContractTypeDescriptor::Record {
                fields: fields
                    .into_iter()
                    .map(|(name, field)| {
                        Ok((
                            name.clone(),
                            self.expand_type_ref(
                                field,
                                &format!("{path}.descriptor.fields[{name}]"),
                            )?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            },
            ContractTypeDescriptor::StructuralUnion { variants } => {
                ContractTypeDescriptor::StructuralUnion {
                    variants: variants
                        .into_iter()
                        .enumerate()
                        .map(|(index, variant)| {
                            self.expand_type_ref(
                                variant,
                                &format!("{path}.descriptor.variants[{index}]"),
                            )
                        })
                        .collect::<Result<_>>()?,
                }
            }
            ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field,
                branches,
            } => ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field,
                branches: branches
                    .into_iter()
                    .map(|mut branch| {
                        branch.branch_type = self.expand_type_ref(
                            branch.branch_type,
                            &format!("{path}.descriptor.branches[{}].branchType", branch.tag),
                        )?;
                        Ok(branch)
                    })
                    .collect::<Result<_>>()?,
            },
            ContractTypeDescriptor::Representation { target } => {
                ContractTypeDescriptor::Representation {
                    target: self.expand_type_ref(target, &format!("{path}.descriptor.target"))?,
                }
            }
            ContractTypeDescriptor::Alias { .. } => {
                return invalid_contract(format!(
                    "{path}.descriptor: transparent alias must be erased before shape expansion"
                ))
            }
            ContractTypeDescriptor::Enumeration { variants } => {
                ContractTypeDescriptor::Enumeration { variants }
            }
            ContractTypeDescriptor::CallbackInterface { operations } => {
                ContractTypeDescriptor::CallbackInterface {
                    operations: operations
                        .into_iter()
                        .map(|(name, mut operation)| {
                            for (index, parameter) in operation.parameters.iter_mut().enumerate() {
                                *parameter = self.expand_type_ref(
                                    parameter.clone(),
                                    &format!(
                                        "{path}.descriptor.operations[{name}].parameters[{index}]"
                                    ),
                                )?;
                            }
                            operation.return_type = self.expand_type_ref(
                                operation.return_type,
                                &format!("{path}.descriptor.operations[{name}].returnType"),
                            )?;
                            Ok((name, operation))
                        })
                        .collect::<Result<_>>()?,
                }
            }
        };
        normalize_contract_type_shape(shape, path)
    }

    fn expand_type_ref(&mut self, ty: ContractTypeRef, path: &str) -> Result<ContractTypeRef> {
        let expanded = match ty {
            ContractTypeRef::Builtin { name, arguments } => ContractTypeRef::Builtin {
                name,
                arguments: arguments
                    .into_iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        self.expand_type_ref(argument, &format!("{path}.arguments[{index}]"))
                    })
                    .collect::<Result<_>>()?,
            },
            ContractTypeRef::Contract { contract_type_id } => {
                if self.alias_targets.contains_key(&contract_type_id) {
                    return self.expand_alias(&contract_type_id);
                }
                if !self.known_type_ids.contains(&contract_type_id) {
                    return invalid_contract(format!(
                        "{path}: boundary schema is not closed; referenced ContractTypeId {contract_type_id} is absent"
                    ));
                }
                ContractTypeRef::contract(contract_type_id)
            }
            ContractTypeRef::Record { fields } => ContractTypeRef::Record {
                fields: fields
                    .into_iter()
                    .map(|(name, field)| {
                        Ok((
                            name.clone(),
                            self.expand_type_ref(field, &format!("{path}.fields[{name}]"))?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            },
            ContractTypeRef::StructuralUnion { variants } => ContractTypeRef::StructuralUnion {
                variants: variants
                    .into_iter()
                    .enumerate()
                    .map(|(index, variant)| {
                        self.expand_type_ref(variant, &format!("{path}.variants[{index}]"))
                    })
                    .collect::<Result<_>>()?,
            },
            ContractTypeRef::Nullable { inner } => ContractTypeRef::Nullable {
                inner: Box::new(self.expand_type_ref(*inner, &format!("{path}.inner"))?),
            },
            ContractTypeRef::Literal { .. } => ty,
        };
        super::normalization::normalize_contract_type_ref(expanded, path)
    }

    fn expand_nominal_type_id(
        &mut self,
        type_id: ContractTypeId,
        path: &str,
    ) -> Result<ContractTypeId> {
        if self.alias_targets.contains_key(&type_id) {
            let expanded = self.expand_alias(&type_id)?;
            let ContractTypeRef::Contract { contract_type_id } = expanded else {
                return invalid_contract(format!(
                    "{path}: callback interface alias must expand to a nominal contract type"
                ));
            };
            return Ok(contract_type_id);
        }
        if !self.known_type_ids.contains(&type_id) {
            return invalid_contract(format!(
                "{path}: boundary schema is not closed; referenced ContractTypeId {type_id} is absent"
            ));
        }
        Ok(type_id)
    }
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}
