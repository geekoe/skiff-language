//! Checked emitter facts that File IR already carries exactly.
//!
//! These DTOs never fill a source-model hole. In particular, an inout loan
//! ordinal is not a callee parameter index, and no value-transfer policy is
//! implied by a writable place.

use std::collections::BTreeSet;

use skiff_artifact_model::{
    validate_supported_receiver_builtin_op, AssignTargetIr, ExprIr, ExprRefIr, InOutPathSegmentIr,
    PatternIr, TypeRefIr,
};

use super::{MirExpression, MirSlot, MirSlotKind};

/// Exact writable root retained by MIR.
#[derive(Debug, Clone, PartialEq)]
pub enum MirWritableRoot {
    /// Function-local slot. File IR does not retain whether a `Local` slot was
    /// declared with `let` or `var`; source validation remains the owner of
    /// that proof.
    Slot { slot: u32 },
    /// Actor durable field root, including its exact static type.
    ActorSelfField {
        field: String,
        field_type: TypeRefIr,
    },
}

/// One exact selector in a writable assignment/receiver path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirWritablePathSegment {
    Field {
        name: String,
    },
    /// Exact index operand owned by this function's expression table.
    Index {
        index: ExprRefIr,
    },
}

/// A writable root plus its complete retained selector path.
#[derive(Debug, Clone, PartialEq)]
pub struct MirWritablePlace {
    pub root: MirWritableRoot,
    pub path: Vec<MirWritablePathSegment>,
}

/// One File IR inout loan in its stable call-local order.
///
/// `loan_ordinal` indexes the compact `CallIr.inout_args` stream; it is
/// deliberately not named `parameter_index`. Recovering a mixed argument's
/// parameter position requires the exact callee signature/mode table rather
/// than guessing from this ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirInOutLoan {
    pub loan_ordinal: u32,
    pub root_slot: u32,
    pub path: Vec<InOutPathSegmentIr>,
}

/// Writable facts owned by one call expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MirCallWritableFacts {
    /// Present only for an exactly-known receiver builtin whose registry fact
    /// says it mutates its receiver.
    pub mutating_receiver: Option<MirWritablePlace>,
    /// Exact compact File IR loan order. See [`MirInOutLoan::loan_ordinal`].
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
            let mut place = expression_place(*object, expressions, slots)?;
            place.path.push(MirWritablePathSegment::Field {
                name: field.clone(),
            });
            Ok(place)
        }
        AssignTargetIr::Index { object, index } => {
            expression_at(*index, expressions)?;
            let mut place = expression_place(*object, expressions, slots)?;
            place
                .path
                .push(MirWritablePathSegment::Index { index: *index });
            Ok(place)
        }
    }
}

pub(super) fn call_writable_facts(
    expression_index: u32,
    expressions: &[MirExpression],
    slots: &[MirSlot],
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
                Some(expression_place(receiver, expressions, slots)?)
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

    let mut inout_loans = Vec::with_capacity(call.inout_args.len());
    for (loan_ordinal, loan) in call.inout_args.iter().enumerate() {
        let loan_ordinal = u32::try_from(loan_ordinal)
            .map_err(|_| "inout loan ordinal exceeds u32::MAX".to_string())?;
        let root = validate_root_slot(loan.root_slot, slots)?;
        if root.kind != MirSlotKind::Local {
            return Err(format!(
                "inout loan {loan_ordinal} root slot {} has kind {:?}, expected Local",
                loan.root_slot, root.kind
            ));
        }
        if loan
            .path
            .iter()
            .any(|segment| matches!(segment, InOutPathSegmentIr::Index))
        {
            return Err(format!(
                "inout loan {loan_ordinal} contains an index selector, but File IR does not retain its index operand"
            ));
        }
        let candidate = MirInOutLoan {
            loan_ordinal,
            root_slot: loan.root_slot,
            path: loan.path.clone(),
        };
        if inout_loans
            .iter()
            .any(|existing| inout_loans_overlap(existing, &candidate))
        {
            return Err(format!(
                "inout loan {loan_ordinal} overlaps an earlier loan for root slot {}",
                loan.root_slot
            ));
        }
        inout_loans.push(candidate);
    }

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
) -> Result<MirWritablePlace, String> {
    expression_place_inner(reference, expressions, slots, &mut BTreeSet::new())
}

fn expression_place_inner(
    reference: ExprRefIr,
    expressions: &[MirExpression],
    slots: &[MirSlot],
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
            let mut place = expression_place_inner(*object, expressions, slots, seen)?;
            place.path.push(MirWritablePathSegment::Field {
                name: field.clone(),
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

fn path_prefix(left: &[InOutPathSegmentIr], right: &[InOutPathSegmentIr]) -> bool {
    left.len() <= right.len() && left.iter().zip(right).all(|(left, right)| left == right)
}
