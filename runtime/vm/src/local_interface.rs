//! Exact VM-local value materialization for local interface children.
//!
//! Local interface methods are executed as flat scheduler children in their
//! own heap domain. Arguments and results therefore cross the same typed
//! `ValueSlot` boundary as service children, but they are described by the
//! method's `LinkedCallableSignature` instead of a service boundary plan.
//! This module copies only canonical detached value graphs from the exact
//! linked type/plan facts; it never guesses a plan from a type name or shape.

use std::fmt;

use skiff_artifact_model::{PackageRefIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    LinkedContainerLayoutKind, LinkedResourceDropPlan, LinkedShapeEntry, LinkedTypeEntry,
    LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    service_error::{
        CatchIdentity, FileAddr, LocalExecutionTypeIdentity, NominalTypeIdentity,
        PackageSchemaTypeIdentity, TypeAddr, UnitAddr,
    },
    vm_heap::{VmContainerShape, VmHeap, VmHeapError, VmMapEntry, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

use crate::lifecycle::LifecycleExecutor;

/// Failure mode for one exact local-interface value transfer.
#[derive(Debug)]
pub enum LocalInterfaceMaterializeError {
    PlanMismatch { type_index: u32, detail: String },
    MissingType { image: String, type_index: u32 },
    TypeIndexMismatch { type_index: u32 },
    SourceTypeMismatch { type_index: u32 },
    UnsupportedKind { kind: Option<ValueKind> },
    UnsupportedType { type_ref: String },
    MissingShape { type_index: u32 },
    MissingCatchIdentity { type_index: u32 },
    Heap(VmHeapError),
}

impl fmt::Display for LocalInterfaceMaterializeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlanMismatch { type_index, detail } => write!(
                formatter,
                "local interface type {type_index} has no exact linked plan: {detail}"
            ),
            Self::MissingType { image, type_index } => write!(
                formatter,
                "local interface type {type_index} is absent from image {image}"
            ),
            Self::TypeIndexMismatch { type_index } => write!(
                formatter,
                "local interface type index {type_index} does not match the image row"
            ),
            Self::SourceTypeMismatch { type_index } => write!(
                formatter,
                "local interface source value does not carry linked type {type_index}"
            ),
            Self::UnsupportedKind { kind } => {
                write!(
                    formatter,
                    "local interface source kind {kind:?} is not copyable"
                )
            }
            Self::UnsupportedType { type_ref } => {
                write!(
                    formatter,
                    "local interface destination type is unsupported: {type_ref}"
                )
            }
            Self::MissingShape { type_index } => write!(
                formatter,
                "local interface type {type_index} has no exact linked shape"
            ),
            Self::MissingCatchIdentity { type_index } => write!(
                formatter,
                "local interface type {type_index} has no exact catch identity"
            ),
            Self::Heap(error) => {
                write!(formatter, "local interface heap operation failed: {error}")
            }
        }
    }
}

impl std::error::Error for LocalInterfaceMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Heap(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VmHeapError> for LocalInterfaceMaterializeError {
    fn from(error: VmHeapError) -> Self {
        Self::Heap(error)
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

/// Copies one canonical detached value into a new heap domain using the exact
/// linked method type/plan. The source owner remains live until the caller
/// explicitly releases it.
pub fn materialize_local_interface_value(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_type: TypeIndex,
    plan: &LinkedValueTransferPlan,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    let mut session = MaterializeSession::default();
    match copy_value_inner(
        source_heap,
        source,
        destination_heap,
        image,
        destination_type,
        plan,
        &mut session,
    ) {
        Ok(value) => Ok(value),
        Err(error) => {
            session.release_all(destination_heap);
            Err(error)
        }
    }
}

/// Releases one caller-side local interface argument according to its exact
/// linked plan after the destination child graph has been published.
pub fn release_local_interface_source(
    heap: &mut dyn VmHeap,
    source: &ValueSlot,
    plan: &LinkedValueTransferPlan,
) -> Result<(), LocalInterfaceMaterializeError> {
    LifecycleExecutor::new(heap)
        .release(source, plan)
        .map_err(|error| match error {
            crate::lifecycle::LifecycleError::Heap(error) => {
                LocalInterfaceMaterializeError::Heap(error)
            }
            crate::lifecycle::LifecycleError::PlanUnavailable => {
                LocalInterfaceMaterializeError::PlanMismatch {
                    type_index: source
                        .compact_type_tag()
                        .map(|tag| tag.type_index())
                        .unwrap_or_default(),
                    detail: "source plan is not supported for local interface release".to_string(),
                }
            }
        })
}

fn copy_value_inner(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_type: TypeIndex,
    plan: &LinkedValueTransferPlan,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    validate_plan(image, destination_type, plan)?;
    let entry = checked_type_entry(image, destination_type)?;
    match source.kind() {
        Some(
            ValueKind::Null
            | ValueKind::Bool
            | ValueKind::Number
            | ValueKind::Integer
            | ValueKind::Date,
        ) => {
            source_heap.validate_live(source)?;
            validate_immediate_type(entry, source.kind())?;
            Ok(*source)
        }
        Some(ValueKind::RequestHeapRef) => {
            let source_type = source
                .compact_type_tag()
                .map(|tag| tag.type_index())
                .map(TypeIndex::new);
            if !source_type.is_some_and(|source_type| {
                equivalent_linked_type(image, source_type, destination_type)
            }) {
                return Err(LocalInterfaceMaterializeError::SourceTypeMismatch {
                    type_index: destination_type.get(),
                });
            }
            source_heap.validate_live(source)?;
            copy_request_heap_ref(
                source_heap,
                source,
                destination_heap,
                image,
                destination_type,
                plan,
                entry,
                session,
            )
        }
        other => Err(LocalInterfaceMaterializeError::UnsupportedKind { kind: other }),
    }
}

fn equivalent_linked_type(
    image: &DeploymentExecutionImage,
    source: TypeIndex,
    destination: TypeIndex,
) -> bool {
    if source == destination {
        return true;
    }
    let source_entry = image
        .types()
        .get(usize::try_from(source.get()).unwrap_or(usize::MAX))
        .filter(|entry| entry.index() == source);
    let destination_entry = image
        .types()
        .get(usize::try_from(destination.get()).unwrap_or(usize::MAX))
        .filter(|entry| entry.index() == destination);
    matches!(
        (source_entry, destination_entry),
        (Some(source_entry), Some(destination_entry))
            if source_entry.type_ref() == destination_entry.type_ref()
                && source_entry.plan() == destination_entry.plan()
    )
}

fn validate_plan(
    image: &DeploymentExecutionImage,
    type_index: TypeIndex,
    plan: &LinkedValueTransferPlan,
) -> Result<(), LocalInterfaceMaterializeError> {
    let exact =
        image
            .type_plan(type_index)
            .ok_or_else(|| LocalInterfaceMaterializeError::MissingType {
                image: image.owner().build_id().as_str().to_string(),
                type_index: type_index.get(),
            })?;
    if exact != plan {
        return Err(LocalInterfaceMaterializeError::PlanMismatch {
            type_index: type_index.get(),
            detail: "linked type plan differs from the method signature plan".to_string(),
        });
    }
    let supported = match plan {
        LinkedValueTransferPlan::SnapshotShare { drop }
        | LinkedValueTransferPlan::MoveOnly { drop } => matches!(
            drop,
            LinkedValueDropPlan::Trivial
                | LinkedValueDropPlan::SnapshotRelease
                | LinkedValueDropPlan::RecursiveShape { .. }
        ),
        LinkedValueTransferPlan::AffineResource { drop } => {
            matches!(drop, LinkedResourceDropPlan::ResourceTableRelease)
        }
        LinkedValueTransferPlan::ExplicitCloneLease { .. } => false,
    };
    if !supported {
        return Err(LocalInterfaceMaterializeError::PlanMismatch {
            type_index: type_index.get(),
            detail: "local interface copy/release requires a supported linked plan".to_string(),
        });
    }
    Ok(())
}

fn checked_type_entry<'a>(
    image: &'a DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<&'a LinkedTypeEntry, LocalInterfaceMaterializeError> {
    let position = usize::try_from(type_index.get()).map_err(|_| {
        LocalInterfaceMaterializeError::TypeIndexMismatch {
            type_index: type_index.get(),
        }
    })?;
    image
        .types()
        .get(position)
        .filter(|entry| entry.index() == type_index)
        .ok_or_else(|| LocalInterfaceMaterializeError::MissingType {
            image: image.owner().build_id().as_str().to_string(),
            type_index: type_index.get(),
        })
}

fn validate_immediate_type(
    entry: &LinkedTypeEntry,
    actual: Option<ValueKind>,
) -> Result<(), LocalInterfaceMaterializeError> {
    let expected = match entry.type_ref() {
        TypeRefIr::Builtin { name, args } if args.is_empty() => match name.as_str() {
            "null" => ValueKind::Null,
            "bool" => ValueKind::Bool,
            "number" => ValueKind::Number,
            "integer" => ValueKind::Integer,
            "Date" => ValueKind::Date,
            _ => {
                return Err(LocalInterfaceMaterializeError::UnsupportedType {
                    type_ref: format!("{:?}", entry.type_ref()),
                })
            }
        },
        _ => {
            return Err(LocalInterfaceMaterializeError::UnsupportedType {
                type_ref: format!("{:?}", entry.type_ref()),
            })
        }
    };
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(LocalInterfaceMaterializeError::UnsupportedKind { kind: actual })
    }
}

fn copy_request_heap_ref(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_type: TypeIndex,
    _plan: &LinkedValueTransferPlan,
    destination_entry: &LinkedTypeEntry,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    let tag = CompactTypeTag::try_from_type_index(destination_type.get()).ok_or_else(|| {
        LocalInterfaceMaterializeError::TypeIndexMismatch {
            type_index: destination_type.get(),
        }
    })?;
    if let Some(carrier) = destination_entry.representation_carrier() {
        let payload = source_heap.representation_payload(source)?;
        let payload_type = carrier.physical_carrier_type();
        let payload_plan = image.type_plan(payload_type).cloned().ok_or_else(|| {
            LocalInterfaceMaterializeError::MissingType {
                image: image.owner().build_id().as_str().to_string(),
                type_index: payload_type.get(),
            }
        })?;
        let materialized_payload = copy_value_inner(
            source_heap,
            &payload,
            destination_heap,
            image,
            payload_type,
            &payload_plan,
            session,
        )?;
        session.commit(materialized_payload);
        let identity = catch_identity_for_type(image, destination_type).ok_or_else(|| {
            LocalInterfaceMaterializeError::MissingCatchIdentity {
                type_index: destination_type.get(),
            }
        })?;
        let representation = destination_heap.allocate_representation(
            &materialized_payload,
            identity,
            tag,
            ValueFlags::new(0),
        )?;
        session.commit(representation);
        return Ok(representation);
    }

    match destination_entry.type_ref() {
        TypeRefIr::Builtin { name, args } if args.is_empty() && name == "string" => {
            let value = source_heap.string_value(source)?;
            let materialized =
                destination_heap.alloc_typed_string(value, tag, ValueFlags::new(0))?;
            session.commit(materialized);
            Ok(materialized)
        }
        TypeRefIr::Builtin { name, args } if args.is_empty() && name == "bytes" => {
            let value = source_heap.bytes_value(source)?;
            let materialized =
                destination_heap.alloc_typed_bytes(value, tag, ValueFlags::new(0))?;
            session.commit(materialized);
            Ok(materialized)
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => copy_array(
            source_heap,
            source,
            destination_heap,
            image,
            destination_entry,
            tag,
            session,
        ),
        TypeRefIr::Builtin { name, args }
            if matches!(name.as_str(), "Map" | "JsonObject") && args.len() == 2 =>
        {
            copy_map(
                source_heap,
                source,
                destination_heap,
                image,
                destination_entry,
                tag,
                session,
            )
        }
        TypeRefIr::Record { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::AppliedNominal { .. } => copy_record(
            source_heap,
            source,
            destination_heap,
            image,
            destination_entry,
            tag,
            session,
        ),
        _ => Err(LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        }),
    }
}

fn copy_array(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    let layout = destination_entry.container_layout().ok_or_else(|| {
        LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        }
    })?;
    if layout.kind() != LinkedContainerLayoutKind::Array {
        return Err(LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        });
    }
    let element =
        layout
            .element()
            .ok_or_else(|| LocalInterfaceMaterializeError::UnsupportedType {
                type_ref: format!("{:?}", destination_entry.type_ref()),
            })?;
    let elements = source_heap.container_elements(source)?;
    if elements.shape != VmContainerShape::Array {
        return Err(LocalInterfaceMaterializeError::UnsupportedKind {
            kind: source.kind(),
        });
    }
    let start_len = session.roots.len();
    let mut materialized = Vec::with_capacity(elements.elements.len());
    for element_value in &elements.elements {
        let copied = copy_value_inner(
            source_heap,
            &element_value.value,
            destination_heap,
            image,
            element.ty(),
            element.plan(),
            session,
        )?;
        materialized.push(copied);
    }
    let array = destination_heap.allocate_array(&materialized, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(array);
    Ok(array)
}

fn copy_map(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    let layout = destination_entry.container_layout().ok_or_else(|| {
        LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        }
    })?;
    if !matches!(
        layout.kind(),
        LinkedContainerLayoutKind::Map | LinkedContainerLayoutKind::JsonObject
    ) {
        return Err(LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        });
    }
    let key = layout
        .key()
        .ok_or_else(|| LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        })?;
    let value = layout
        .value()
        .ok_or_else(|| LocalInterfaceMaterializeError::UnsupportedType {
            type_ref: format!("{:?}", destination_entry.type_ref()),
        })?;
    let len = source_heap.map_len(source)?;
    let start_len = session.roots.len();
    let mut entries = Vec::with_capacity(len);
    for ordinal in 0..len {
        let entry = source_heap.map_entry_at(source, ordinal)?;
        let copied_key = copy_value_inner(
            source_heap,
            &entry.key,
            destination_heap,
            image,
            key.ty(),
            key.plan(),
            session,
        )?;
        let copied_value = copy_value_inner(
            source_heap,
            &entry.value,
            destination_heap,
            image,
            value.ty(),
            value.plan(),
            session,
        )?;
        entries.push(VmMapEntry {
            key: copied_key,
            value: copied_value,
        });
    }
    let map = destination_heap.allocate_map(&entries, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(map);
    Ok(map)
}

fn copy_record(
    source_heap: &dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    destination_entry: &LinkedTypeEntry,
    tag: CompactTypeTag,
    session: &mut MaterializeSession,
) -> Result<ValueSlot, LocalInterfaceMaterializeError> {
    let shape = image
        .shapes()
        .iter()
        .find(|shape: &&LinkedShapeEntry| {
            image
                .types()
                .get(usize::try_from(shape.nominal_type().get()).unwrap_or(usize::MAX))
                .filter(|entry| entry.index() == shape.nominal_type())
                .is_some_and(|entry| entry.type_ref() == destination_entry.type_ref())
        })
        .ok_or_else(|| LocalInterfaceMaterializeError::MissingShape {
            type_index: destination_entry.index().get(),
        })?;
    let start_len = session.roots.len();
    let mut fields = Vec::with_capacity(shape.fields().len());
    for field in shape.fields() {
        let source_field = source_heap.record_field(source, field.name())?;
        let copied = copy_value_inner(
            source_heap,
            &source_field,
            destination_heap,
            image,
            field.ty(),
            field.plan(),
            session,
        )?;
        fields.push(VmRecordField {
            name: field.name().to_string(),
            value: copied,
        });
    }
    let record = destination_heap.allocate_record(&fields, tag, ValueFlags::new(0))?;
    session.roots.truncate(start_len);
    session.commit(record);
    Ok(record)
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
