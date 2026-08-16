//! Typed VM heap materialization for the F6 service boundary plans.
//!
//! This module intentionally consumes `ValueSlot`/`VmHeap` and the linked
//! boundary facts. It does not route through the legacy `RuntimeValue` /
//! `RequestHeap` service adapter and never re-derives a plan from a runtime
//! shape or type name.

use std::collections::BTreeMap;

use serde_json::Value;
use skiff_artifact_model::{
    BoundaryTransfer, BoundaryValueCarrier, BoundaryValueEncoding, ContractLiteral,
    ContractTypeRef, InterfaceInstantiationRef, PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    LinkedContainerLayoutKind, LinkedServiceBoundaryValue, LinkedShapeEntry, LinkedTypeEntry,
    TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    service_error::{
        CatchIdentity, FileAddr, LocalExecutionTypeIdentity, NominalTypeIdentity,
        PackageSchemaTypeIdentity, TypeAddr, UnitAddr,
    },
    vm_heap::{VmContainerShape, VmHeap, VmHeapError, VmMapEntry, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VmMaterializeError {
    #[error("service boundary plan is unsupported: {reason}")]
    UnsupportedPlan { reason: String },
    #[error("service boundary value type is unsupported: {message}")]
    UnsupportedType { message: String },
    #[error("service boundary value type row is absent from image {image}")]
    MissingType { image: String, type_index: u32 },
    #[error("service boundary value type row {type_index} does not match image index")]
    TypeIndexMismatch { type_index: u32 },
    #[error("service boundary source value has no runtime kind")]
    MissingKind,
    #[error("service boundary source value kind {kind:?} is not a detached graph value")]
    UnsupportedKind {
        kind: Option<skiff_runtime_model::vm_value::ValueKind>,
    },
    #[error("service boundary plan source owner/lifetime/carrier mismatch")]
    PlanMismatch,
    #[error("service boundary value heap operation failed: {0}")]
    Heap(#[from] VmHeapError),
    #[error("service boundary catch identity is unavailable for type {type_index}")]
    MissingCatchIdentity { type_index: u32 },
}

/// Materializes one source value into a destination VM heap from the exact
/// caller-side boundary value plan.
pub fn materialize_linked_value(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_type: TypeIndex,
    plan: &LinkedServiceBoundaryValue,
) -> Result<ValueSlot, VmMaterializeError> {
    validate_plan(plan)?;
    let mut session = MaterializeSession::default();
    let result = materialize_value_inner(
        source_heap,
        source,
        destination_heap,
        destination_image,
        destination_type,
        &mut session,
    );
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            session.release_all(destination_heap);
            Err(error)
        }
    }
}

#[derive(Default)]
struct MaterializeSession {
    roots: Vec<ValueSlot>,
}

impl MaterializeSession {
    fn commit(&mut self, root: ValueSlot) {
        self.roots.push(root);
    }

    fn release_all(&mut self, heap: &mut dyn VmHeap) {
        while let Some(root) = self.roots.pop() {
            let _ = heap.release_snapshot(&root);
        }
    }
}

fn validate_plan(plan: &LinkedServiceBoundaryValue) -> Result<(), VmMaterializeError> {
    let value_plan = plan.value_plan();
    let skiff_artifact_model::BoundaryValuePlan::Linkable {
        carrier, encoding, ..
    } = value_plan
    else {
        return Err(VmMaterializeError::UnsupportedPlan {
            reason: format!("{value_plan:?}"),
        });
    };
    if *carrier != BoundaryValueCarrier::DetachedValueGraph
        || *encoding != BoundaryValueEncoding::CanonicalValue
    {
        return Err(VmMaterializeError::PlanMismatch);
    }
    Ok(())
}

fn materialize_value_inner(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_type: TypeIndex,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, VmMaterializeError> {
    let destination_entry = checked_type_entry(destination_image, destination_type)?;
    let Some(kind) = source.kind() else {
        return Err(VmMaterializeError::MissingKind);
    };
    match kind {
        skiff_runtime_model::vm_value::ValueKind::Null
        | skiff_runtime_model::vm_value::ValueKind::Bool
        | skiff_runtime_model::vm_value::ValueKind::Number
        | skiff_runtime_model::vm_value::ValueKind::Integer
        | skiff_runtime_model::vm_value::ValueKind::Date => {
            validate_immediate_type(destination_entry, kind)?;
            Ok(*source)
        }
        skiff_runtime_model::vm_value::ValueKind::RequestHeapRef => {
            source_heap.validate_live(source)?;
            materialize_request_heap_ref(
                source_heap,
                source,
                destination_heap,
                destination_image,
                destination_entry,
                session,
            )
        }
        other => Err(VmMaterializeError::UnsupportedKind { kind: Some(other) }),
    }
}

fn checked_type_entry<'a>(
    image: &'a DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<&'a LinkedTypeEntry, VmMaterializeError> {
    let position =
        usize::try_from(type_index.get()).map_err(|_| VmMaterializeError::TypeIndexMismatch {
            type_index: type_index.get(),
        })?;
    let entry = image
        .types()
        .get(position)
        .filter(|entry| entry.index() == type_index)
        .ok_or_else(|| VmMaterializeError::MissingType {
            image: image.owner().build_id().as_str().to_string(),
            type_index: type_index.get(),
        })?;
    Ok(entry)
}

fn validate_immediate_type(
    entry: &LinkedTypeEntry,
    actual: skiff_runtime_model::vm_value::ValueKind,
) -> Result<(), VmMaterializeError> {
    let expected = match entry.type_ref() {
        TypeRefIr::Builtin { name, args } if args.is_empty() => match name.as_str() {
            "null" => skiff_runtime_model::vm_value::ValueKind::Null,
            "bool" => skiff_runtime_model::vm_value::ValueKind::Bool,
            "number" => skiff_runtime_model::vm_value::ValueKind::Number,
            "integer" => skiff_runtime_model::vm_value::ValueKind::Integer,
            "Date" => skiff_runtime_model::vm_value::ValueKind::Date,
            _ => return Err(unsupported_type(entry.type_ref())),
        },
        _ => return Err(unsupported_type(entry.type_ref())),
    };
    if expected == actual {
        Ok(())
    } else {
        Err(VmMaterializeError::UnsupportedKind { kind: Some(actual) })
    }
}

fn materialize_request_heap_ref(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, VmMaterializeError> {
    let tag =
        CompactTypeTag::try_from_type_index(destination_entry.index().get()).ok_or_else(|| {
            VmMaterializeError::TypeIndexMismatch {
                type_index: destination_entry.index().get(),
            }
        })?;
    let flags = ValueFlags::new(0);
    if let Some(carrier) = destination_entry.representation_carrier() {
        let payload = source_heap.representation_payload(source)?;
        let materialized_payload = materialize_value_inner(
            source_heap,
            &payload,
            destination_heap,
            destination_image,
            carrier.physical_carrier_type(),
            session,
        )?;
        session.commit(materialized_payload);
        let identity = catch_identity_for_type(destination_image, destination_entry.index())
            .ok_or_else(|| VmMaterializeError::MissingCatchIdentity {
                type_index: destination_entry.index().get(),
            })?;
        let representation = destination_heap.allocate_representation(
            &materialized_payload,
            identity,
            tag,
            flags,
        )?;
        session.commit(representation);
        return Ok(representation);
    }

    match destination_entry.type_ref() {
        TypeRefIr::Builtin { name, args } if args.is_empty() && name == "string" => {
            let value = source_heap.string_value(source)?;
            let materialized = destination_heap.alloc_typed_string(value, tag, flags)?;
            session.commit(materialized);
            Ok(materialized)
        }
        TypeRefIr::Builtin { name, args } if args.is_empty() && name == "bytes" => {
            let value = source_heap.bytes_value(source)?;
            let materialized = destination_heap.alloc_typed_bytes(value, tag, flags)?;
            session.commit(materialized);
            Ok(materialized)
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
            materialize_array(
                source_heap,
                source,
                destination_heap,
                destination_image,
                destination_entry,
                tag,
                session,
            )
        }
        TypeRefIr::Builtin { name, args }
            if matches!(name.as_str(), "Map" | "JsonObject") && args.len() == 2 =>
        {
            materialize_map(
                source_heap,
                source,
                destination_heap,
                destination_image,
                destination_entry,
                tag,
                session,
            )
        }
        TypeRefIr::Record { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::PackageSymbol { .. } => materialize_record(
            source_heap,
            source,
            destination_heap,
            destination_image,
            destination_entry,
            tag,
            session,
        ),
        _ => Err(unsupported_type(destination_entry.type_ref())),
    }
}

fn materialize_array(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, VmMaterializeError> {
    let element_type = destination_entry
        .container_layout()
        .and_then(|layout| {
            (layout.kind() == LinkedContainerLayoutKind::Array)
                .then(|| layout.element().map(|element| element.ty()))
                .flatten()
        })
        .or_else(|| {
            let TypeRefIr::Builtin { args, .. } = destination_entry.type_ref() else {
                return None;
            };
            find_type_index_by_ref(destination_image, &args[0])
        })
        .ok_or_else(|| unsupported_type(destination_entry.type_ref()))?;
    let elements = source_heap.container_elements(source)?;
    if elements.shape != VmContainerShape::Array {
        return Err(VmMaterializeError::UnsupportedKind {
            kind: source.kind(),
        });
    }
    let mut materialized = Vec::with_capacity(elements.elements.len());
    for element in &elements.elements {
        let value = materialize_value_inner(
            source_heap,
            &element.value,
            destination_heap,
            destination_image,
            element_type,
            session,
        )?;
        materialized.push(value);
    }
    let array = destination_heap.allocate_array(&materialized, tag, ValueFlags::new(0))?;
    // The array now owns its child slots; they must not be released a second
    // time through the session on the normal success path.
    let committed = session.roots.len().saturating_sub(materialized.len());
    session.roots.truncate(committed);
    session.commit(array);
    Ok(array)
}

fn materialize_map(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, VmMaterializeError> {
    let layout = destination_entry
        .container_layout()
        .ok_or_else(|| unsupported_type(destination_entry.type_ref()))?;
    let key_type = layout
        .key()
        .ok_or_else(|| unsupported_type(destination_entry.type_ref()))?
        .ty();
    let value_type = layout
        .value()
        .ok_or_else(|| unsupported_type(destination_entry.type_ref()))?
        .ty();
    let len = source_heap.map_len(source)?;
    let mut entries = Vec::with_capacity(len);
    for ordinal in 0..len {
        let entry = source_heap.map_entry_at(source, ordinal)?;
        let key = materialize_value_inner(
            source_heap,
            &entry.key,
            destination_heap,
            destination_image,
            key_type,
            session,
        )?;
        let value = materialize_value_inner(
            source_heap,
            &entry.value,
            destination_heap,
            destination_image,
            value_type,
            session,
        )?;
        entries.push(VmMapEntry { key, value });
    }
    let map = destination_heap.allocate_map(&entries, tag, ValueFlags::new(0))?;
    // Map allocation consumes both child slots.
    let consumed = entries.len().saturating_mul(2);
    let committed = session.roots.len().saturating_sub(consumed);
    session.roots.truncate(committed);
    session.commit(map);
    Ok(map)
}

fn materialize_record(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    destination_image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, VmMaterializeError> {
    let fields = record_fields(destination_image, destination_entry)?;
    let mut materialized = Vec::with_capacity(fields.len());
    for (name, field_type) in fields {
        let field = source_heap.record_field(source, &name)?;
        let value = materialize_value_inner(
            source_heap,
            &field,
            destination_heap,
            destination_image,
            field_type,
            session,
        )?;
        materialized.push(VmRecordField { name, value });
    }
    let record = destination_heap.allocate_record(&materialized, tag, ValueFlags::new(0))?;
    let consumed = materialized.len();
    let committed = session.roots.len().saturating_sub(consumed);
    session.roots.truncate(committed);
    session.commit(record);
    Ok(record)
}

fn record_fields(
    image: &DeploymentExecutionImage,
    entry: &LinkedTypeEntry,
) -> Result<Vec<(String, TypeIndex)>, VmMaterializeError> {
    if let TypeRefIr::Record { fields } = entry.type_ref() {
        return fields
            .iter()
            .map(|(name, ty)| {
                find_type_index_by_ref(image, ty)
                    .map(|index| (name.clone(), index))
                    .ok_or_else(|| unsupported_type(ty))
            })
            .collect();
    }
    let shape = image
        .shapes()
        .iter()
        .find(|shape: &&LinkedShapeEntry| shape.nominal_type() == entry.index())
        .ok_or_else(|| unsupported_type(entry.type_ref()))?;
    Ok(shape
        .fields()
        .iter()
        .map(|field| (field.name().to_string(), field.ty()))
        .collect())
}

fn find_type_index_by_ref(image: &DeploymentExecutionImage, ty: &TypeRefIr) -> Option<TypeIndex> {
    image
        .types()
        .iter()
        .find(|entry| entry.type_ref() == ty)
        .map(LinkedTypeEntry::index)
}

fn unsupported_type(ty: &TypeRefIr) -> VmMaterializeError {
    VmMaterializeError::UnsupportedType {
        message: format!("{ty:?}"),
    }
}

fn catch_identity_for_type(
    image: &DeploymentExecutionImage,
    leaf: TypeIndex,
) -> Option<CatchIdentity> {
    let entry = checked_type_entry(image, leaf).ok()?;
    match entry.type_ref() {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let identity = PackageSchemaTypeIdentity::new(
                package_id.clone(),
                stable_schema_key.clone(),
                package_schema_type_id.clone(),
            )
            .ok()?;
            Some(CatchIdentity::Nominal(NominalTypeIdentity::PackageSchema(
                identity,
            )))
        }
        TypeRefIr::PackageSymbol { symbol } => {
            let PackageRefIr::PackageId { package_id } = &symbol.package else {
                return None;
            };
            let package_slot = image
                .packages()
                .iter()
                .find(|package| package.package_build_id() == entry.origin().package_build_id())
                .map(|package| package.index().get() as usize)?;
            Some(CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
                LocalExecutionTypeIdentity {
                    addr: TypeAddr {
                        unit: UnitAddr::Package(package_slot),
                        file: FileAddr::FileIrIdentity(package_id.clone()),
                        type_index: leaf.get() as usize,
                    },
                    type_arguments: Vec::new(),
                },
            )))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::PackageSchemaTypeId;

    #[test]
    fn callback_any_interface_maps_to_linked_interface_identity() {
        let interface = ContractTypeRef::package_schema(
            "example.com/phase-6-callback-provider",
            "Handler",
            PackageSchemaTypeId::new("contract:reader"),
        );
        let ty = ContractTypeRef::AnyInterface {
            interface: Box::new(interface),
            arguments: Vec::new(),
        };

        let linked = contract_type_to_type_ref(&ty)
            .expect("callback AnyInterface must map to a linked type ref");
        let TypeRefIr::AnyInterface { interface } = linked else {
            panic!("callback AnyInterface must remain an AnyInterface row");
        };
        assert!(interface
            .interface_abi_id
            .contains("example.com/phase-6-callback-provider"));
        assert!(interface.interface_abi_id.contains("Handler"));
        assert!(interface.canonical_type_args.is_empty());
    }
}

/// Compares the compiler-emitted transfer fact without consuming it.
pub fn transfer_is_move(plan: &LinkedServiceBoundaryValue) -> bool {
    plan.transfer() == BoundaryTransfer::Move
}

/// Releases a caller-side boundary source using the boundary drop plan.
///
/// The concrete VM heap performs recursive snapshot release for detached
/// graphs; no runtime-kind fallback is used.
pub fn release_boundary_source(
    heap: &mut dyn VmHeap,
    value: &ValueSlot,
) -> Result<(), VmMaterializeError> {
    heap.release_snapshot(value)
        .map_err(VmMaterializeError::from)
}

/// Checks the provider signature's exact type row against the linker-carried
/// boundary value TypeRefIr. Same-type rows may appear more than once in an
/// image, so the provider index is authoritative rather than a first match.
pub fn boundary_value_matches_linked_type(
    image: &DeploymentExecutionImage,
    provider_type: TypeIndex,
    value: &LinkedServiceBoundaryValue,
) -> bool {
    let position = usize::try_from(provider_type.get()).ok();
    let Some(entry) = position
        .and_then(|position| image.types().get(position))
        .filter(|entry| entry.index() == provider_type)
    else {
        return false;
    };
    same_boundary_type(entry.type_ref(), value.linked_type_ref())
}

/// Resolves the exact provider-side callback carrier type when the contract's
/// package-schema key does not carry the provider's package-symbol/ABI facts.
///
/// The provider signature is the linked authority for the operation; this
/// lookup only accepts an `AnyInterface` row from the same package whose
/// symbol path matches the contract interface. It never fabricates a type row.
pub fn linked_callback_type_for_contract(
    image: &DeploymentExecutionImage,
    contract_type: &ContractTypeRef,
) -> Option<TypeIndex> {
    let ContractTypeRef::AnyInterface {
        interface,
        arguments,
    } = contract_type
    else {
        return None;
    };
    let ContractTypeRef::PackageSchema {
        package_id,
        stable_schema_key,
        ..
    } = interface.as_ref()
    else {
        return None;
    };
    let canonical_type_args = arguments
        .iter()
        .map(contract_type_to_type_ref)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let mut matches = image.types().iter().filter_map(|entry| {
        let TypeRefIr::AnyInterface { interface } = entry.type_ref() else {
            return None;
        };
        if interface.canonical_type_args != canonical_type_args {
            return None;
        }
        let Ok(value) = serde_json::from_str::<Value>(&interface.interface_abi_id) else {
            return None;
        };
        let Some(symbol) = value.get("symbol") else {
            return None;
        };
        let Some(actual_package_id) = symbol
            .get("package")
            .and_then(|package| package.get("packageId"))
            .and_then(Value::as_str)
        else {
            return None;
        };
        let Some(actual_symbol_path) = symbol.get("symbolPath").and_then(Value::as_str) else {
            return None;
        };
        let suffix = format!(".{stable_schema_key}");
        (actual_package_id == package_id
            && (actual_symbol_path == stable_schema_key || actual_symbol_path.ends_with(&suffix)))
        .then_some(entry.index())
    });
    let resolved = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(resolved)
}

fn contract_type_to_type_ref(ty: &ContractTypeRef) -> Result<TypeRefIr, VmMaterializeError> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => Ok(TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        }),
        ContractTypeRef::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), contract_type_to_type_ref(ty)?)))
                .collect::<Result<BTreeMap<_, _>, VmMaterializeError>>()?,
        }),
        ContractTypeRef::StructuralUnion { variants } => Ok(TypeRefIr::Union {
            items: variants
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        ContractTypeRef::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(contract_type_to_type_ref(inner)?),
        }),
        ContractTypeRef::Literal { value } => match value {
            ContractLiteral::String { value } => Ok(TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::String {
                    value: value.clone(),
                },
            }),
        },
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let interface_ir = match interface.as_ref() {
                ContractTypeRef::PackageSchema {
                    package_id,
                    stable_schema_key,
                    ..
                } => TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: package_id.clone(),
                        },
                        symbol_path: stable_schema_key.clone(),
                        abi_expectation: None,
                    },
                },
                other => contract_type_to_type_ref(other)?,
            };
            let interface_abi_id =
                skiff_canonical_json::canonical_json_bytes(&interface_ir).map_err(|error| {
                    VmMaterializeError::UnsupportedType {
                        message: format!(
                            "service boundary callback interface identity cannot be canonicalized: {error}"
                        ),
                    }
                })?;
            let interface_abi_id = String::from_utf8(interface_abi_id).map_err(|error| {
                VmMaterializeError::UnsupportedType {
                    message: format!(
                        "service boundary callback interface identity is not UTF-8: {error}"
                    ),
                }
            })?;
            let canonical_type_args = arguments
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args,
                },
            })
        }
        ContractTypeRef::TypeParam { name } => Err(VmMaterializeError::UnsupportedType {
            message: format!("{name:?}"),
        }),
    }
}

fn same_boundary_type(provider: &TypeRefIr, linked: &TypeRefIr) -> bool {
    if provider == linked {
        return true;
    }
    let (
        TypeRefIr::AnyInterface {
            interface: provider_interface,
        },
        TypeRefIr::AnyInterface {
            interface: linked_interface,
        },
    ) = (provider, linked)
    else {
        return false;
    };
    interface_stable_key(provider_interface) == interface_stable_key(linked_interface)
}

fn interface_stable_key(interface: &InterfaceInstantiationRef) -> Option<(String, String)> {
    let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
    match identity {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => Some((package_id, stable_schema_key)),
        TypeRefIr::PackageSymbol { symbol } => {
            let PackageRefIr::PackageId { package_id } = symbol.package else {
                return None;
            };
            let stable_schema_key = symbol
                .symbol_path
                .rsplit_once('.')
                .map(|(_, symbol)| symbol.to_string())
                .unwrap_or(symbol.symbol_path);
            Some((package_id, stable_schema_key))
        }
        _ => None,
    }
}
