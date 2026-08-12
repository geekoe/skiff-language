//! Checked emitter facts that File IR already carries exactly.
//!
//! These DTOs never fill a source-model hole. Inout loans retain their exact
//! callee parameter ordinal, and no value-transfer policy is implied by a
//! writable place.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    validate_supported_receiver_builtin_op, AssignTargetIr, ContractOperationId, ExprIr, ExprRefIr,
    InOutPathSegmentIr, InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, PatternIr,
    ServiceProtocolIdentity, TypeRefIr,
};

use super::{MirExpression, MirIndexAccessFacts, MirSlot, MirSlotKind};

/// Exact compiler-owned stream facts for one function or stream-typed
/// expression. `item_type` is the source-owned `T` in `Stream<T>`.
#[derive(Debug, Clone, PartialEq)]
pub struct MirStreamResultFacts {
    pub item_type: TypeRefIr,
}

/// Exact completion facts for one function-owned `ExprIr::ValueBlock`.
///
/// `completion_targets` are the CFG blocks that currently complete into the
/// expression's statement continuation. An emitter linearizing the block can
/// redirect exactly those targets to the resume PC and then evaluate `result`.
#[derive(Debug, Clone, PartialEq)]
pub struct MirExpressionBlockFact {
    pub body_block: u32,
    pub continuation_block: u32,
    pub result: ExprRefIr,
    pub completion_targets: Vec<u32>,
}

/// Consumer-owned remote interface table facts retained by MIR.
///
/// The service requirement slot is not reconstructed from a public instance
/// key or provider identity; it is copied from the exact consumer
/// service-requirement authority. Emission fails closed when those facts are
/// absent.
#[derive(Debug, Clone, PartialEq)]
pub struct MirRemoteInterfaceFacts {
    pub service_requirement_slot: u32,
    pub public_instance_key: String,
    pub interface: InterfaceInstantiationRef,
    pub methods: Vec<MirRemoteInterfaceMethodFacts>,
    pub callee_protocol_identity: ServiceProtocolIdentity,
}

/// One exact remote interface method row. The contract operation id is the
/// canonical service-requirement operation authority, not a provider-local
/// executable identity.
#[derive(Debug, Clone, PartialEq)]
pub struct MirRemoteInterfaceMethodFacts {
    pub slot: u32,
    pub method_abi_id: String,
    pub signature: InterfaceMethodSlotSignatureIr,
    pub contract_operation_id: ContractOperationId,
}

/// Exact writable root retained by MIR.
#[derive(Debug, Clone, PartialEq)]
pub enum MirWritableRoot {
    /// Function-local slot. [`super::MirSlot::writable_local`] is the retained
    /// source proof when this root is loaned as inout.
    Slot { slot: u32 },
    /// Actor durable field root, including its exact static type.
    ActorSelfField {
        field: String,
        field_type: TypeRefIr,
    },
}

/// One exact selector in a writable assignment/receiver path.
#[derive(Debug, Clone, PartialEq)]
pub enum MirWritablePathSegment {
    Field {
        name: String,
    },
    /// Exact index operand owned by this function's expression table.
    Index {
        index: ExprRefIr,
        index_type: TypeRefIr,
        access: Box<MirIndexAccessFacts>,
    },
}

/// A writable root plus its complete retained selector path.
#[derive(Debug, Clone, PartialEq)]
pub struct MirWritablePlace {
    pub root: MirWritableRoot,
    pub path: Vec<MirWritablePathSegment>,
}

/// One exact selector in an inout writable path.
#[derive(Debug, Clone, PartialEq)]
pub enum MirInOutPathSegment {
    Field {
        name: String,
    },
    Index {
        selector: ExprRefIr,
        selector_type: TypeRefIr,
        access: Box<MirIndexAccessFacts>,
    },
}

/// One File IR inout loan at its exact callee parameter coordinate.
#[derive(Debug, Clone, PartialEq)]
pub struct MirInOutLoan {
    pub parameter_ordinal: u32,
    pub root_slot: u32,
    pub root_type: TypeRefIr,
    pub path: Vec<MirInOutPathSegment>,
}

/// Writable facts owned by one call expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MirCallWritableFacts {
    /// Present only for an exactly-known receiver builtin whose registry fact
    /// says it mutates its receiver.
    pub mutating_receiver: Option<MirWritablePlace>,
    /// Exact loans sorted by callee parameter ordinal.
    pub inout_loans: Vec<MirInOutLoan>,
}

/// Exact container semantics for a single-binding `for` statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirForInItemKind {
    ArrayItem,
    StreamItem,
    MapKey,
}

/// Checked binding facts for a `for` statement.
#[derive(Debug, Clone, PartialEq)]
pub enum MirForInBinding {
    Item {
        slot: u32,
        ty: TypeRefIr,
        kind: MirForInItemKind,
    },
    MapEntry {
        key_slot: u32,
        key_type: TypeRefIr,
        value_slot: u32,
        value_type: TypeRefIr,
    },
}

/// Fully typed `for` semantics, derived only after matching the File IR
/// `item_type` against the exact owned iterable expression type.
#[derive(Debug, Clone, PartialEq)]
pub struct MirForInFacts {
    pub iterable_type: TypeRefIr,
    pub binding: MirForInBinding,
}

pub(super) fn assignment_place(
    target: &AssignTargetIr,
    expressions: &[MirExpression],
    slots: &[MirSlot],
    index_accesses: &BTreeMap<u32, MirIndexAccessFacts>,
) -> Result<MirWritablePlace, String> {
    match target {
        AssignTargetIr::Slot { slot } => {
            validate_root_slot(*slot, slots)?;
            Ok(MirWritablePlace {
                root: MirWritableRoot::Slot { slot: *slot },
                path: Vec::new(),
            })
        }
        AssignTargetIr::ActorSelfField { field, field_type } => Ok(MirWritablePlace {
            root: MirWritableRoot::ActorSelfField {
                field: field.clone(),
                field_type: field_type.clone(),
            },
            path: Vec::new(),
        }),
        AssignTargetIr::Field { object, field } => {
            let mut place = expression_place(*object, expressions, slots, index_accesses)?;
            place.path.push(MirWritablePathSegment::Field {
                name: field.clone(),
            });
            Ok(place)
        }
        AssignTargetIr::Index { object, index } => {
            let index_type = expression_at(*index, expressions)?.ty.clone();
            let access = index_access(*index, expressions, index_accesses)?.clone();
            let mut place = expression_place(*object, expressions, slots, index_accesses)?;
            place.path.push(MirWritablePathSegment::Index {
                index: *index,
                index_type,
                access: Box::new(access),
            });
            Ok(place)
        }
    }
}

pub(super) fn call_writable_facts(
    expression_index: u32,
    expressions: &[MirExpression],
    slots: &[MirSlot],
    index_accesses: &BTreeMap<u32, MirIndexAccessFacts>,
) -> Result<Option<MirCallWritableFacts>, String> {
    let expression = expression_at(
        ExprRefIr {
            expression: expression_index,
        },
        expressions,
    )?;
    let ExprIr::Call { call } = &expression.expression else {
        return Ok(None);
    };

    let mutating_receiver = match &call.target {
        skiff_artifact_model::CallTargetIr::ReceiverBuiltin { op } => {
            let spec = validate_supported_receiver_builtin_op(op)
                .map_err(|error| format!("receiver builtin is not canonical: {error}"))?;
            if spec.mutates_receiver {
                let receiver = call.args.first().copied().ok_or_else(|| {
                    "mutating receiver builtin has no receiver argument".to_string()
                })?;
                Some(expression_place(
                    receiver,
                    expressions,
                    slots,
                    index_accesses,
                )?)
            } else {
                None
            }
        }
        _ => None,
    };

    if !call.inout_args.is_empty()
        && !matches!(
            &call.target,
            skiff_artifact_model::CallTargetIr::LocalExecutable { .. }
                | skiff_artifact_model::CallTargetIr::PublicationExecutable { .. }
                | skiff_artifact_model::CallTargetIr::PackageCallable { .. }
        )
    {
        return Err(
            "inout loans require an exact local, publication-local, or package-direct target"
                .to_string(),
        );
    }

    let mut inout_loans = Vec::<MirInOutLoan>::with_capacity(call.inout_args.len());
    for loan in &call.inout_args {
        let root = validate_root_slot(loan.root_slot, slots)?;
        if root.kind != MirSlotKind::Local || !root.writable_local {
            return Err(format!(
                "inout loan for parameter {} root slot {} is not a source-confirmed writable local",
                loan.parameter_ordinal, loan.root_slot
            ));
        }
        let root_type = root.ty.clone().ok_or_else(|| {
            format!(
                "inout loan for parameter {} root slot {} has no exact type",
                loan.parameter_ordinal, loan.root_slot
            )
        })?;
        let path = loan
            .path
            .iter()
            .map(|segment| match segment {
                InOutPathSegmentIr::Field { name } => {
                    Ok(MirInOutPathSegment::Field { name: name.clone() })
                }
                InOutPathSegmentIr::Index { selector } => {
                    let selector_type = expression_at(*selector, expressions)?.ty.clone();
                    let access = index_access(*selector, expressions, index_accesses)?.clone();
                    Ok(MirInOutPathSegment::Index {
                        selector: *selector,
                        selector_type,
                        access: Box::new(access),
                    })
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        let candidate = MirInOutLoan {
            parameter_ordinal: loan.parameter_ordinal,
            root_slot: loan.root_slot,
            root_type,
            path,
        };
        if inout_loans
            .iter()
            .any(|existing| existing.parameter_ordinal == candidate.parameter_ordinal)
        {
            return Err(format!(
                "inout call repeats callee parameter ordinal {}",
                candidate.parameter_ordinal
            ));
        }
        if inout_loans
            .iter()
            .any(|existing| inout_loans_overlap(existing, &candidate))
        {
            return Err(format!(
                "inout loan for parameter {} overlaps an earlier loan for root slot {}",
                loan.parameter_ordinal, loan.root_slot
            ));
        }
        inout_loans.push(candidate);
    }
    inout_loans.sort_by_key(|loan| loan.parameter_ordinal);

    if mutating_receiver.is_none() && inout_loans.is_empty() {
        Ok(None)
    } else {
        Ok(Some(MirCallWritableFacts {
            mutating_receiver,
            inout_loans,
        }))
    }
}

pub(super) fn for_in_facts(
    item_slot: u32,
    item_type: Option<&TypeRefIr>,
    value_slot: Option<u32>,
    iterable: ExprRefIr,
    expressions: &[MirExpression],
    slots: &[MirSlot],
) -> Result<MirForInFacts, String> {
    let item_slot_entry = validate_for_binding_slot(item_slot, slots)?;
    let iterable_type = expression_at(iterable, expressions)?.ty.clone();
    let TypeRefIr::Builtin { name, args } = &iterable_type else {
        return Err(format!(
            "for iterable type {iterable_type:?} is not Array, Stream, or Map"
        ));
    };

    let binding = match value_slot {
        None => {
            let (ty, kind) = match (name.as_str(), args.as_slice()) {
                ("Array", [item]) => (item.clone(), MirForInItemKind::ArrayItem),
                ("Stream", [item]) => (item.clone(), MirForInItemKind::StreamItem),
                ("Map", [key, _]) => (key.clone(), MirForInItemKind::MapKey),
                _ => {
                    return Err(format!(
                        "single-binding for iterable type {iterable_type:?} has invalid container arity"
                    ));
                }
            };
            require_item_type(item_type, &ty)?;
            require_slot_type_if_present(item_slot_entry, &ty)?;
            MirForInBinding::Item {
                slot: item_slot,
                ty,
                kind,
            }
        }
        Some(value_slot) => {
            if value_slot == item_slot {
                return Err("for map entry reuses one slot for key and value".to_string());
            }
            let value_slot_entry = validate_for_binding_slot(value_slot, slots)?;
            let (key_type, value_type) = match (name.as_str(), args.as_slice()) {
                ("Map", [key, value]) => (key.clone(), value.clone()),
                _ => {
                    return Err(format!(
                        "entry-binding for iterable type {iterable_type:?} requires Map<K, V>"
                    ));
                }
            };
            require_item_type(item_type, &key_type)?;
            require_slot_type_if_present(item_slot_entry, &key_type)?;
            require_slot_type_if_present(value_slot_entry, &value_type)?;
            MirForInBinding::MapEntry {
                key_slot: item_slot,
                key_type,
                value_slot,
                value_type,
            }
        }
    };

    Ok(MirForInFacts {
        iterable_type,
        binding,
    })
}

pub(super) fn validate_assert_types(
    condition: ExprRefIr,
    message: Option<ExprRefIr>,
    expressions: &[MirExpression],
) -> Result<(), String> {
    let condition_type = &expression_at(condition, expressions)?.ty;
    if condition_type != &TypeRefIr::builtin("bool") {
        return Err(format!(
            "assert condition has type {condition_type:?}, expected bool"
        ));
    }
    if let Some(message) = message {
        let message_type = &expression_at(message, expressions)?.ty;
        if message_type != &TypeRefIr::builtin("string") {
            return Err(format!(
                "assert message has type {message_type:?}, expected string"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_pattern(pattern: &PatternIr, slots: &[MirSlot]) -> Result<(), String> {
    validate_pattern_inner(pattern, slots, &mut BTreeSet::new())
}

fn validate_pattern_inner(
    pattern: &PatternIr,
    slots: &[MirSlot],
    binding_slots: &mut BTreeSet<u32>,
) -> Result<(), String> {
    match pattern {
        PatternIr::Binding { slot } => {
            let slot = validate_root_slot(*slot, slots)?;
            if slot.kind != MirSlotKind::Pattern {
                return Err(format!(
                    "pattern binding slot {} has kind {:?}, expected Pattern",
                    slot.slot, slot.kind
                ));
            }
            if !binding_slots.insert(slot.slot) {
                return Err(format!("pattern repeats binding slot {}", slot.slot));
            }
        }
        PatternIr::Record { fields } => {
            let mut names = BTreeSet::new();
            for field in fields {
                if !names.insert(&field.name) {
                    return Err(format!("record pattern repeats field `{}`", field.name));
                }
                validate_pattern_inner(&field.pattern, slots, binding_slots)?;
            }
        }
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Type { .. } => {}
    }
    Ok(())
}

fn expression_place(
    reference: ExprRefIr,
    expressions: &[MirExpression],
    slots: &[MirSlot],
    index_accesses: &BTreeMap<u32, MirIndexAccessFacts>,
) -> Result<MirWritablePlace, String> {
    expression_place_inner(
        reference,
        expressions,
        slots,
        index_accesses,
        &mut BTreeSet::new(),
    )
}

fn expression_place_inner(
    reference: ExprRefIr,
    expressions: &[MirExpression],
    slots: &[MirSlot],
    index_accesses: &BTreeMap<u32, MirIndexAccessFacts>,
    seen: &mut BTreeSet<u32>,
) -> Result<MirWritablePlace, String> {
    if !seen.insert(reference.expression) {
        return Err(format!(
            "writable expression path cycles at expression {}",
            reference.expression
        ));
    }
    let expression = expression_at(reference, expressions)?;
    let place = match &expression.expression {
        ExprIr::LoadSlot { slot } => {
            validate_root_slot(*slot, slots)?;
            MirWritablePlace {
                root: MirWritableRoot::Slot { slot: *slot },
                path: Vec::new(),
            }
        }
        ExprIr::ActorSelfField { field, field_type } => MirWritablePlace {
            root: MirWritableRoot::ActorSelfField {
                field: field.clone(),
                field_type: field_type.clone(),
            },
            path: Vec::new(),
        },
        ExprIr::Field { object, field } => {
            let mut place =
                expression_place_inner(*object, expressions, slots, index_accesses, seen)?;
            place.path.push(MirWritablePathSegment::Field {
                name: field.clone(),
            });
            place
        }
        ExprIr::Index { object, index } => {
            let index_type = expression_at(*index, expressions)?.ty.clone();
            let access = index_access(*index, expressions, index_accesses)?.clone();
            let mut place =
                expression_place_inner(*object, expressions, slots, index_accesses, seen)?;
            place.path.push(MirWritablePathSegment::Index {
                index: *index,
                index_type,
                access: Box::new(access),
            });
            place
        }
        other => {
            return Err(format!(
                "expression {} ({other:?}) is not an exact writable root/path",
                reference.expression
            ));
        }
    };
    seen.remove(&reference.expression);
    Ok(place)
}

fn index_access<'a>(
    selector: ExprRefIr,
    expressions: &[MirExpression],
    index_accesses: &'a BTreeMap<u32, MirIndexAccessFacts>,
) -> Result<&'a MirIndexAccessFacts, String> {
    let selector_type = &expression_at(selector, expressions)?.ty;
    let access = index_accesses.get(&selector.expression).ok_or_else(|| {
        format!(
            "index selector {} has no exact source fact",
            selector.expression
        )
    })?;
    if &access.selector_type != selector_type {
        return Err(format!(
            "index selector {} type {selector_type:?} disagrees with source fact {:?}",
            selector.expression, access.selector_type
        ));
    }
    Ok(access)
}

fn expression_at(
    reference: ExprRefIr,
    expressions: &[MirExpression],
) -> Result<&MirExpression, String> {
    let expression = expressions
        .get(reference.expression as usize)
        .ok_or_else(|| format!("missing expression {}", reference.expression))?;
    if expression.index != reference.expression {
        return Err(format!(
            "expression lookup {} found stored index {}",
            reference.expression, expression.index
        ));
    }
    Ok(expression)
}

fn validate_root_slot(slot: u32, slots: &[MirSlot]) -> Result<&MirSlot, String> {
    let entry = slots
        .get(slot as usize)
        .ok_or_else(|| format!("writable root references missing slot {slot}"))?;
    if entry.slot != slot {
        return Err(format!(
            "writable root slot {slot} found stored index {}",
            entry.slot
        ));
    }
    Ok(entry)
}

fn validate_for_binding_slot(slot: u32, slots: &[MirSlot]) -> Result<&MirSlot, String> {
    let slot = validate_root_slot(slot, slots)?;
    if slot.kind != MirSlotKind::Local {
        return Err(format!(
            "for binding slot {} has kind {:?}, expected Local",
            slot.slot, slot.kind
        ));
    }
    Ok(slot)
}

fn require_item_type(actual: Option<&TypeRefIr>, expected: &TypeRefIr) -> Result<(), String> {
    let actual = actual.ok_or_else(|| "for statement has no exact item_type fact".to_string())?;
    if actual != expected {
        return Err(format!(
            "for item_type {actual:?} does not match iterable-derived type {expected:?}"
        ));
    }
    Ok(())
}

fn require_slot_type_if_present(slot: &MirSlot, expected: &TypeRefIr) -> Result<(), String> {
    if let Some(actual) = &slot.ty {
        if actual != expected {
            return Err(format!(
                "for binding slot {} type {actual:?} does not match iterable-derived type {expected:?}",
                slot.slot
            ));
        }
    }
    Ok(())
}

fn inout_loans_overlap(left: &MirInOutLoan, right: &MirInOutLoan) -> bool {
    left.root_slot == right.root_slot
        && (path_prefix(&left.path, &right.path) || path_prefix(&right.path, &left.path))
}

fn path_prefix(left: &[MirInOutPathSegment], right: &[MirInOutPathSegment]) -> bool {
    left.len() <= right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (
                    MirInOutPathSegment::Field { name: left },
                    MirInOutPathSegment::Field { name: right },
                ) => left == right,
                // Independently evaluated dynamic selectors can alias even when
                // their ExprRef indices differ. No source fact proves inequality,
                // so overlap validation must stay conservative.
                (MirInOutPathSegment::Index { .. }, MirInOutPathSegment::Index { .. }) => true,
                _ => false,
            })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{ExprRefIr, SourcePosition, SourceSpanRef, TypeRefIr};

    use super::{path_prefix, MirInOutPathSegment};
    use crate::mir::{MirIndexAccessFacts, MirIndexPolicy, MirIndexReceiverKind};

    fn index(selector: u32) -> MirInOutPathSegment {
        let selector_type = TypeRefIr::builtin("number");
        MirInOutPathSegment::Index {
            selector: ExprRefIr {
                expression: selector,
            },
            selector_type: selector_type.clone(),
            access: Box::new(MirIndexAccessFacts {
                receiver_kind: MirIndexReceiverKind::Array,
                receiver_type: TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::builtin("number")],
                },
                selector_type,
                result_type: TypeRefIr::builtin("number"),
                policy: MirIndexPolicy::LoanMustExist,
                source_span: SourceSpanRef {
                    source_id: 0,
                    start: SourcePosition {
                        line: 1,
                        column: 1,
                        offset: Some(0),
                    },
                    end: SourcePosition {
                        line: 1,
                        column: 2,
                        offset: Some(1),
                    },
                },
            }),
        }
    }

    #[test]
    fn independently_evaluated_index_selectors_may_overlap() {
        assert!(path_prefix(&[index(1)], &[index(2)]));
        assert!(!path_prefix(
            &[MirInOutPathSegment::Field {
                name: "left".to_string(),
            }],
            &[MirInOutPathSegment::Field {
                name: "right".to_string(),
            }],
        ));
    }
}
