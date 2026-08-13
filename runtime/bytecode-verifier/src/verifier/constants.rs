use std::fmt;

use skiff_artifact_model::{
    BytecodeConstantRef, BytecodePoolEntry, FrozenConstantNode, LiteralIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    CandidateTable, ConstantIndex, LinkedBytecodeCandidate, LinkedConstantEntry,
    LinkedConstantReference, LinkedFrozenConstantValue, LinkedValueDropPlan,
    LinkedValueTransferPlan,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;
use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Immutable values materialized from the verified frozen constant graph.
///
/// Fields and construction are private to the verifier. Scalar literals are
/// materialized as immediates; strings are represented by a [`ValueSlot`] of
/// kind `ConstRef` whose handle is meaningful only together with the same
/// pinned deployment execution image. This type never accepts values
/// or handles supplied by a caller.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedConstantHeap;
/// use skiff_runtime_model::vm_value::ValueSlot;
///
/// fn extract_values(heap: &VerifiedConstantHeap) -> &[ValueSlot] {
///     &heap.values
/// }
/// ```
pub struct VerifiedConstantHeap {
    pub(super) values: Box<[ValueSlot]>,
    pub(super) _seal: VerifiedConstantHeapSeal,
}

#[derive(Debug)]
pub(super) struct VerifiedConstantHeapSeal;

impl fmt::Debug for VerifiedConstantHeap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedConstantHeap")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedConstantHeap {
    /// Returns one verified constant value by its image-local index.
    pub fn get(&self, index: ConstantIndex) -> Option<ValueSlot> {
        let index = usize::try_from(index.get()).ok()?;
        self.values.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

pub(crate) fn prove_and_build_constant_heap(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<VerifiedConstantHeap, VerificationError> {
    prove_authority_counts(hydrated, candidate)?;
    prove_literal_nodes(candidate)?;
    prove_constant_roots(hydrated, candidate)?;

    let mut values = Vec::with_capacity(candidate.constants().len());
    for constant in candidate.constants() {
        values.push(materialize_constant(hydrated, candidate, constant)?);
    }
    Ok(VerifiedConstantHeap {
        values: values.into_boxed_slice(),
        _seal: VerifiedConstantHeapSeal,
    })
}

fn prove_authority_counts(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let mut source_constants = 0_usize;
    let mut source_roots = 0_usize;
    let mut source_nodes = 0_usize;
    for package in hydrated
        .packages()
        .values()
        .filter(|package| package.has_bytecode())
    {
        let view = package
            .bytecode()
            .ok_or_else(|| authority_overflow())?
            .view();
        source_constants = source_constants
            .checked_add(view.pools().constants.len())
            .ok_or_else(authority_overflow)?;
        source_roots = source_roots
            .checked_add(view.constant_roots().len())
            .ok_or_else(authority_overflow)?;
        source_nodes = source_nodes
            .checked_add(view.frozen_constant_graph().nodes.len())
            .ok_or_else(authority_overflow)?;
    }

    if candidate.constants().len() != source_constants {
        return Err(authority_count_mismatch(
            CandidateTable::Constants,
            first_candidate_constant_location(candidate),
            source_constants,
            candidate.constants().len(),
        ));
    }
    if candidate.constant_roots().len() != source_roots {
        return Err(authority_count_mismatch(
            CandidateTable::ConstantRoots,
            first_candidate_constant_location(candidate),
            source_roots,
            candidate.constant_roots().len(),
        ));
    }
    if candidate.frozen_constant_nodes().len() != source_nodes {
        return Err(authority_count_mismatch(
            CandidateTable::FrozenConstantNodes,
            first_candidate_constant_location(candidate),
            source_nodes,
            candidate.frozen_constant_nodes().len(),
        ));
    }
    Ok(())
}

fn prove_literal_nodes(candidate: &LinkedBytecodeCandidate) -> Result<(), VerificationError> {
    for node in candidate.frozen_constant_nodes() {
        if !matches!(node.value(), LinkedFrozenConstantValue::Literal(_)) {
            return Err(unavailable(table_location(
                CandidateTable::FrozenConstantNodes,
                node.index().get(),
            )));
        }
    }
    Ok(())
}

fn prove_constant_roots(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for (build_id, package) in hydrated
        .packages()
        .iter()
        .filter(|(_, package)| package.has_bytecode())
    {
        for (symbol_path, artifact_index) in package
            .bytecode()
            .ok_or_else(|| {
                violation(
                    VerificationLocation::Image,
                    "bytecode package has no hydrated bytecode".to_string(),
                )
            })?
            .view()
            .constant_roots()
        {
            let root = candidate.constant_roots().iter().find(|root| {
                root.owner_package_build_id() == build_id
                    && root.symbol_path().as_str() == symbol_path
            });
            let Some(root) = root else {
                return Err(frozen_constant_violation(
                    VerificationLocation::Image,
                    format!("candidate is missing constant root {build_id}/{symbol_path}"),
                ));
            };
            let constant = candidate
                .constants()
                .get(root.constant().get() as usize)
                .ok_or_else(|| {
                    frozen_constant_violation(
                        VerificationLocation::Image,
                        format!("constant root {build_id}/{symbol_path} is out of bounds"),
                    )
                })?;
            if constant.origin().package_build_id() != build_id
                || constant.origin().artifact_index().get() != *artifact_index
                || constant.origin().specialization().is_some()
            {
                return Err(frozen_constant_violation(
                    VerificationLocation::Image,
                    format!(
                        "constant root {build_id}/{symbol_path} does not select its exact package-global artifact row"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn materialize_constant(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    constant: &LinkedConstantEntry,
) -> Result<ValueSlot, VerificationError> {
    let location = table_location(CandidateTable::Constants, constant.index().get());
    let LinkedConstantReference::LocalNode { node } = constant.reference() else {
        return Err(unavailable(location));
    };
    let linked_node = candidate
        .frozen_constant_nodes()
        .get(node.get() as usize)
        .filter(|row| row.index() == *node)
        .ok_or_else(|| violation(location, "constant node is out of bounds"))?;
    let node_location = table_location(CandidateTable::FrozenConstantNodes, node.get());
    let LinkedFrozenConstantValue::Literal(literal) = linked_node.value() else {
        return Err(unavailable(node_location));
    };
    let package = hydrated
        .packages()
        .get(constant.origin().package_build_id())
        .ok_or_else(|| violation(location, "constant origin package is not hydrated"))?;

    let source_constant =
        source_constant_row(package, constant.origin().artifact_index().get(), location)?;
    let BytecodePoolEntry::ConstantRef {
        reference,
        type_ref,
        ..
    } = source_constant
    else {
        return Err(violation(
            location,
            "constant origin row has the wrong pool kind",
        ));
    };
    let BytecodeConstantRef::LocalNode { node_index } = reference else {
        return Err(unavailable(location));
    };
    if linked_node.origin().package_build_id() != &package.reference().package_build_id
        || linked_node.origin().artifact_index().get() != *node_index
        || linked_node.origin().specialization().is_some()
    {
        return Err(violation(
            location,
            "linked constant node does not match its exact source node",
        ));
    }
    let source_node = source_frozen_node(package, *node_index, node_location)?;
    let FrozenConstantNode::Literal {
        literal: source_literal,
    } = source_node
    else {
        return Err(unavailable(node_location));
    };
    if source_literal != literal {
        return Err(violation(
            location,
            "linked literal differs from its exact source literal",
        ));
    }

    let source_type = source_type_ref(package, *type_ref, location)?;
    require_literal_carrier(literal, source_type, location)?;
    let linked_type = candidate
        .types()
        .get(constant.ty().get() as usize)
        .filter(|row| row.index() == constant.ty())
        .ok_or_else(|| violation(location, "constant linked type is out of bounds"))?;
    if linked_type.origin().package_build_id() != &package.reference().package_build_id
        || linked_type.origin().artifact_index().get() != *type_ref
        || linked_type.origin().specialization().is_some()
    {
        return Err(violation(
            location,
            "linked constant type does not match its exact source type",
        ));
    }
    require_literal_carrier(literal, linked_type.type_ref(), location)?;
    require_literal_plan(literal, constant.plan(), location)?;

    match literal {
        LiteralIr::Null => Ok(ValueSlot::null()),
        LiteralIr::Bool { value } => Ok(ValueSlot::bool(*value)),
        LiteralIr::Number { value } => value.as_f64().map(ValueSlot::number).ok_or_else(|| {
            violation(
                location,
                "number literal cannot be materialized as an f64 immediate",
            )
        }),
        LiteralIr::String { .. } => Ok(ValueSlot::const_ref(
            VmHandle::new(u64::from(node.get())),
            CompactTypeTag::new(constant.ty().get()),
            ValueFlags::new(0),
        )),
    }
}

fn source_constant_row(
    package: &skiff_runtime_loader::HydratedBytecodePackage,
    artifact_index: u32,
    location: VerificationLocation,
) -> Result<&BytecodePoolEntry, VerificationError> {
    let position = usize::try_from(artifact_index).map_err(|_| {
        violation(
            location,
            "constant origin artifact index does not fit usize",
        )
    })?;
    package
        .bytecode()
        .ok_or_else(|| violation(location, "constant origin package is type-only".to_string()))?
        .view()
        .pools()
        .constants
        .get(position)
        .ok_or_else(|| violation(location, "constant origin row is absent from the hydration"))
}

fn source_frozen_node(
    package: &skiff_runtime_loader::HydratedBytecodePackage,
    node_index: u32,
    location: VerificationLocation,
) -> Result<&FrozenConstantNode, VerificationError> {
    let position = usize::try_from(node_index)
        .map_err(|_| violation(location, "source node index does not fit usize"))?;
    package
        .bytecode()
        .ok_or_else(|| violation(location, "constant origin package is type-only".to_string()))?
        .view()
        .frozen_constant_graph()
        .nodes
        .get(position)
        .ok_or_else(|| violation(location, "source frozen constant node is absent"))
}

fn source_type_ref(
    package: &skiff_runtime_loader::HydratedBytecodePackage,
    type_ref: u32,
    location: VerificationLocation,
) -> Result<&TypeRefIr, VerificationError> {
    let position = usize::try_from(type_ref)
        .map_err(|_| violation(location, "type row does not fit usize"))?;
    match package
        .bytecode()
        .ok_or_else(|| violation(location, "constant origin package is type-only".to_string()))?
        .view()
        .pools()
        .types
        .get(position)
    {
        Some(BytecodePoolEntry::TypeRef { ty }) => Ok(ty),
        Some(_) => Err(violation(
            location,
            "source constant type row has the wrong pool kind",
        )),
        None => Err(violation(location, "source constant type row is absent")),
    }
}

fn require_literal_carrier(
    literal: &LiteralIr,
    ty: &TypeRefIr,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match ty {
        TypeRefIr::Literal { value } if value == literal => Ok(()),
        TypeRefIr::Literal { .. } => Err(violation(
            location,
            "literal carrier value differs from the frozen literal",
        )),
        TypeRefIr::Builtin { name, args } if args.is_empty() => {
            let expected = literal_name(literal);
            if name == expected {
                Ok(())
            } else if matches!(name.as_str(), "null" | "bool" | "number" | "string") {
                Err(violation(
                    location,
                    format!("frozen {expected} literal is declared as builtin {name}"),
                ))
            } else {
                Err(unavailable(location))
            }
        }
        TypeRefIr::PackageSymbol { symbol }
            if symbol.symbol_path == "std.time.Duration"
                && matches!(
                    &symbol.package,
                    skiff_artifact_model::PackageRefIr::PackageId { package_id }
                        if package_id == "skiff.run/std"
                ) =>
        {
            Ok(())
        }
        _ => Err(unavailable(location)),
    }
}

fn require_literal_plan(
    literal: &LiteralIr,
    plan: &LinkedValueTransferPlan,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected_drop = match literal {
        LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            LinkedValueDropPlan::Trivial
        }
        LiteralIr::String { .. } => LinkedValueDropPlan::SnapshotRelease,
    };
    if plan
        != &(LinkedValueTransferPlan::SnapshotShare {
            drop: expected_drop,
        })
    {
        return Err(violation(
            location,
            "constant lifecycle plan differs from its literal carrier",
        ));
    }
    Ok(())
}

fn literal_name(literal: &LiteralIr) -> &'static str {
    match literal {
        LiteralIr::Null => "null",
        LiteralIr::Bool { .. } => "bool",
        LiteralIr::Number { .. } => "number",
        LiteralIr::String { .. } => "string",
    }
}

fn authority_count_mismatch(
    table: CandidateTable,
    first_candidate: Option<VerificationLocation>,
    source_count: usize,
    candidate_count: usize,
) -> VerificationError {
    let location = if candidate_count > source_count {
        first_candidate.unwrap_or(VerificationLocation::Image)
    } else {
        VerificationLocation::Image
    };
    let action = if candidate_count > source_count {
        "introduced"
    } else {
        "erased"
    };
    frozen_constant_violation(
        location,
        format!(
            "candidate {action} {} authority from the exact hydration ({candidate_count} rows versus {source_count})",
            table.name()
        ),
    )
}

fn authority_overflow() -> VerificationError {
    frozen_constant_violation(
        VerificationLocation::Image,
        "frozen constant authority count overflowed usize",
    )
}

fn table_location(table: CandidateTable, row: u32) -> VerificationLocation {
    VerificationLocation::Table { table, row }
}

fn first_candidate_constant_location(
    candidate: &LinkedBytecodeCandidate,
) -> Option<VerificationLocation> {
    candidate
        .constants()
        .first()
        .map(|constant| VerificationLocation::Table {
            table: CandidateTable::Constants,
            row: constant.index().get(),
        })
        .or_else(|| {
            candidate
                .constant_roots()
                .first()
                .map(|_| VerificationLocation::Table {
                    table: CandidateTable::ConstantRoots,
                    row: 0,
                })
        })
        .or_else(|| {
            candidate
                .frozen_constant_nodes()
                .first()
                .map(|node| VerificationLocation::Table {
                    table: CandidateTable::FrozenConstantNodes,
                    row: node.index().get(),
                })
        })
}

fn frozen_constant_violation(
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::FrozenConstantSafety,
        location,
        detail: detail.into(),
    }
}

fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::FrozenConstantSafety,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::FrozenConstantSafety,
        location,
        detail: detail.into(),
    }
}
