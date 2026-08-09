use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    canonical_interface_method_abi_id, interface_instantiation_ref, type_ref_abi_key,
};
use skiff_artifact_model::{InterfaceInstantiationRef, TypeRefIr};
use skiff_compiler_core::{
    json_utils::canonical_json_bytes,
    type_ref::{contains_type_param, substitute_type_params_in_type_ref_ref},
};

use crate::{
    compile_model::ExportPublicInstanceBinding,
    local_interface_conformances::validate_closed_interface_instantiation,
    parsed_sources::ParsedCompilerSource, SourceLocalInterfaceConformanceError, SourceSymbolKey,
    TypeResolutionContext, TypeResolutionModel,
};

use super::{validate_public_root, SourcePublicInstanceOperationFactsError};

pub(crate) struct ResolvedSourcePublicInstance {
    pub(crate) public_root: String,
    pub(crate) interfaces: Vec<ResolvedSourcePublicInstanceInterface>,
}

pub(crate) struct ResolvedSourcePublicInstanceInterface {
    pub(crate) interface: InterfaceInstantiationRef,
    pub(crate) slots: Vec<ResolvedSourcePublicInstanceSlot>,
}

pub(crate) struct ResolvedSourcePublicInstanceSlot {
    pub(crate) method_abi_id: String,
    pub(crate) operation_stable_key: String,
    pub(crate) implementation: SourceSymbolKey,
}

pub(crate) fn resolve_public_instance(
    parsed_sources: &[ParsedCompilerSource],
    instance: &ExportPublicInstanceBinding,
    type_resolution: &TypeResolutionModel,
) -> Result<ResolvedSourcePublicInstance, SourcePublicInstanceOperationFactsError> {
    let public_root = instance.public_path.clone();
    validate_public_root(&public_root)?;
    if instance.interfaces.is_empty() {
        return Err(SourcePublicInstanceOperationFactsError::EmptyInterfaces { public_root });
    }
    let source_matches = parsed_sources
        .iter()
        .filter(|parsed| parsed.module_path() == instance.source_module)
        .collect::<Vec<_>>();
    let [source] = source_matches.as_slice() else {
        return Err(
            SourcePublicInstanceOperationFactsError::AmbiguousSourceModule {
                public_root,
                module_path: instance.source_module.clone(),
                count: source_matches.len(),
            },
        );
    };
    let constants = source
        .ast()
        .consts
        .iter()
        .filter(|constant| constant.name == instance.source_symbol)
        .collect::<Vec<_>>();
    let [constant] = constants.as_slice() else {
        return Err(
            SourcePublicInstanceOperationFactsError::AmbiguousSourceConstant {
                public_root,
                source_symbol: instance.source_symbol.clone(),
                count: constants.len(),
            },
        );
    };
    let declared_type = constant.ty.as_ref().ok_or_else(|| {
        SourcePublicInstanceOperationFactsError::MissingReceiverType {
            public_root: public_root.clone(),
            source_symbol: instance.source_symbol.clone(),
        }
    })?;
    let context = TypeResolutionContext::source(&instance.source_module);
    let resolved_type = type_resolution
        .resolve_type_ref(declared_type, &context)
        .map_err(
            |message| SourcePublicInstanceOperationFactsError::ReceiverTypeResolution {
                public_root: public_root.clone(),
                message,
            },
        )?;
    let (receiver, receiver_arguments) = type_resolution
        .public_instance_receiver_instantiation(&resolved_type, &context)
        .map_err(
            |error| SourcePublicInstanceOperationFactsError::ReceiverTypeStability {
                public_root: public_root.clone(),
                message: error.to_string(),
            },
        )?
        .ok_or_else(
            || SourcePublicInstanceOperationFactsError::InvalidReceiverType {
                public_root: public_root.clone(),
            },
        )?;
    if receiver_arguments.iter().any(contains_type_param) {
        return Err(SourcePublicInstanceOperationFactsError::OpenReceiverType { public_root });
    }

    let conformance_facts = type_resolution
        .local_interface_conformance_facts()
        .map_err(
            |error| SourcePublicInstanceOperationFactsError::LocalConformanceFacts {
                public_root: public_root.clone(),
                message: error.to_string(),
            },
        )?;
    let mut stable_keys = BTreeSet::new();
    let mut exact_interfaces = BTreeSet::new();
    let mut interfaces = Vec::new();
    for selector in &instance.interfaces {
        let selector = SourceSymbolKey::new(&selector.source_module, &selector.source_symbol);
        let identities = type_resolution
            .public_instance_interface_selector_identities(&selector)
            .map_err(|message| {
                SourcePublicInstanceOperationFactsError::InterfaceSelectorResolution {
                    public_root: public_root.clone(),
                    selector: selector.clone(),
                    message,
                }
            })?;
        if identities.is_empty() {
            return Err(
                SourcePublicInstanceOperationFactsError::MissingInterfaceSelector {
                    public_root: public_root.clone(),
                    selector,
                },
            );
        }
        if identities.len() != 1 {
            return Err(
                SourcePublicInstanceOperationFactsError::AmbiguousInterfaceSelector {
                    public_root: public_root.clone(),
                    selector,
                },
            );
        }
        let identity_abi_id = type_ref_abi_key(&identities[0]);
        let matches = conformance_facts
            .iter()
            .filter(|row| row.receiver() == &receiver)
            .filter(|row| row.interface().interface_abi_id.as_str() == identity_abi_id.as_str())
            .collect::<Vec<_>>();
        let [conformance] = matches.as_slice() else {
            let error = if matches.is_empty() {
                SourcePublicInstanceOperationFactsError::MissingConformance {
                    public_root: public_root.clone(),
                    receiver: receiver.clone(),
                    selector: selector.clone(),
                }
            } else {
                SourcePublicInstanceOperationFactsError::AmbiguousConformance {
                    public_root: public_root.clone(),
                    receiver: receiver.clone(),
                    selector: selector.clone(),
                }
            };
            return Err(error);
        };
        if conformance.receiver_type_parameters().len() != receiver_arguments.len() {
            return Err(SourcePublicInstanceOperationFactsError::ReceiverArity {
                public_root: public_root.clone(),
                receiver: receiver.clone(),
                expected: conformance.receiver_type_parameters().len(),
                actual: receiver_arguments.len(),
            });
        }
        let substitutions = conformance
            .receiver_type_parameters()
            .iter()
            .cloned()
            .zip(receiver_arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let arguments = conformance
            .interface()
            .canonical_type_args
            .iter()
            .map(|argument| substitute_type_params_in_type_ref_ref(argument, &substitutions))
            .collect::<Vec<_>>();
        if arguments.iter().any(contains_type_param) {
            return Err(
                SourcePublicInstanceOperationFactsError::ResidualInterfaceTypeParameter {
                    public_root: public_root.clone(),
                    interface_abi_id: conformance.interface().interface_abi_id.clone(),
                },
            );
        }
        let identity = serde_json::from_str::<TypeRefIr>(&conformance.interface().interface_abi_id)
            .map_err(
                |error| SourcePublicInstanceOperationFactsError::InvalidInterface {
                    public_root: public_root.clone(),
                    source: SourceLocalInterfaceConformanceError::InvalidInterfaceIdentity {
                        message: error.to_string(),
                    },
                },
            )?;
        let interface = interface_instantiation_ref(identity, arguments);
        validate_closed_interface_instantiation(&interface).map_err(|source| {
            SourcePublicInstanceOperationFactsError::InvalidInterface {
                public_root: public_root.clone(),
                source,
            }
        })?;
        let exact_key = canonical_json_bytes(&interface).map_err(|error| {
            SourcePublicInstanceOperationFactsError::CanonicalKey {
                message: error.to_string(),
            }
        })?;
        if !exact_interfaces.insert(exact_key) {
            return Err(
                SourcePublicInstanceOperationFactsError::DuplicateInterfaceRow {
                    canonical_key: format!("{}:{}", public_root, interface.interface_abi_id),
                },
            );
        }
        let declared_slots = type_resolution
            .public_instance_interface_method_slots(
                &receiver,
                conformance.receiver_type_parameters(),
                &receiver_arguments,
                &interface,
            )
            .map_err(
                |message| SourcePublicInstanceOperationFactsError::InterfaceSlots {
                    public_root: public_root.clone(),
                    message,
                },
            )?;
        if declared_slots.len() != conformance.implementation_methods().len() {
            return Err(SourcePublicInstanceOperationFactsError::SlotCountMismatch {
                public_root: public_root.clone(),
                declared: declared_slots.len(),
                implementations: conformance.implementation_methods().len(),
            });
        }
        let mut method_names = BTreeSet::new();
        let slots = declared_slots
            .into_iter()
            .zip(conformance.implementation_methods())
            .enumerate()
            .map(|(expected, (slot, implementation))| {
                let expected_slot = u32::try_from(expected).unwrap_or(u32::MAX);
                if slot.slot != expected_slot {
                    return Err(SourcePublicInstanceOperationFactsError::NonContiguousSlot {
                        public_root: public_root.clone(),
                        expected: expected_slot,
                        actual: slot.slot,
                    });
                }
                if !method_names.insert(slot.name.clone()) {
                    return Err(SourcePublicInstanceOperationFactsError::DuplicateMethod {
                        public_root: public_root.clone(),
                        method: slot.name,
                    });
                }
                let expected_abi = canonical_interface_method_abi_id(&interface, &slot.name);
                if slot.method_abi_id != expected_abi {
                    return Err(SourcePublicInstanceOperationFactsError::MethodAbiMismatch {
                        public_root: public_root.clone(),
                        slot: expected,
                        expected: expected_abi,
                        actual: slot.method_abi_id,
                    });
                }
                let operation_stable_key = format!("{}.{}", public_root, slot.name);
                if !stable_keys.insert(operation_stable_key.clone()) {
                    return Err(
                        SourcePublicInstanceOperationFactsError::OperationStableKeyCollision {
                            public_root: public_root.clone(),
                            operation_stable_key,
                        },
                    );
                }
                Ok(ResolvedSourcePublicInstanceSlot {
                    method_abi_id: slot.method_abi_id,
                    operation_stable_key,
                    implementation: implementation.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        interfaces.push(ResolvedSourcePublicInstanceInterface { interface, slots });
    }
    Ok(ResolvedSourcePublicInstance {
        public_root,
        interfaces,
    })
}
