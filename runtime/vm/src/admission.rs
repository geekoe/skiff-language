use skiff_artifact_model::TypeRefIr;
use skiff_runtime_linked_bytecode::{LinkedValueTransferPlan, TypeIndex};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::vm_value::{ValueKind, ValueSlot};

use crate::{VmEntryArgumentRejection, VmError};

pub(crate) fn is_self_describing_immediate(value: &ValueSlot) -> bool {
    matches!(
        value.kind(),
        Some(
            ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date
        )
    )
}

pub(crate) fn is_discardable_root(value: &ValueSlot) -> bool {
    is_self_describing_immediate(value) || matches!(value.kind(), Some(ValueKind::ConstRef))
}

pub(crate) fn validate_entry_arguments(
    program: &DeploymentExecutionImage,
    expected_types: &[TypeIndex],
    expected_plans: &[LinkedValueTransferPlan],
    arguments: &[ValueSlot],
) -> Result<(), VmError> {
    if expected_types.len() != expected_plans.len() || expected_types.len() != arguments.len() {
        return Err(VmError::EntryArgumentCountMismatch {
            expected: expected_types.len(),
            actual: arguments.len(),
        });
    }

    for (ordinal, (argument, expected)) in arguments
        .iter()
        .zip(expected_types.iter().copied())
        .enumerate()
    {
        let linked_type = program
            .types()
            .get(expected.get() as usize)
            .filter(|linked_type| linked_type.index() == expected);
        validate_entry_argument(
            ordinal,
            argument,
            expected,
            linked_type.map(|linked_type| linked_type.type_ref()),
            expected_plans.get(ordinal),
        )?;
    }
    Ok(())
}

fn validate_entry_argument(
    ordinal: usize,
    argument: &ValueSlot,
    expected: TypeIndex,
    expected_type: Option<&TypeRefIr>,
    expected_plan: Option<&LinkedValueTransferPlan>,
) -> Result<(), VmError> {
    let kind = argument.kind();
    let rejection = match kind {
        None => Some(VmEntryArgumentRejection::InvalidMetadata),
        Some(ValueKind::ConstRef) => Some(VmEntryArgumentRejection::ImageScopedConstant),
        Some(ValueKind::ActorStateRef) => Some(VmEntryArgumentRejection::ActorState),
        Some(ValueKind::ResourceRef) => {
            if is_exact_stream_resource(expected_type, expected_plan) {
                None
            } else {
                Some(VmEntryArgumentRejection::AffineResource)
            }
        }
        Some(ValueKind::CallbackClosureRef) => Some(VmEntryArgumentRejection::CallbackClosure),
        Some(ValueKind::RequestHeapRef) => {
            if expected_type.is_some() {
                None
            } else {
                Some(VmEntryArgumentRejection::HeapTypeProofUnavailable)
            }
        }
        Some(_) if is_self_describing_immediate(argument) => None,
        Some(_) => Some(VmEntryArgumentRejection::InvalidMetadata),
    };
    if let Some(reason) = rejection {
        return Err(VmError::EntryArgumentRejected {
            ordinal,
            kind,
            reason,
        });
    }

    if is_exact_stream_resource(expected_type, expected_plan) {
        return Ok(());
    }

    if matches!(kind, Some(ValueKind::RequestHeapRef)) && expected_type.is_some() {
        return Ok(());
    }

    if expected_type.is_some_and(|expected_type| exact_immediate_type_matches(expected_type, kind))
    {
        return Ok(());
    }

    Err(VmError::EntryArgumentTypeMismatch {
        ordinal,
        expected,
        actual: kind,
    })
}

fn is_exact_stream_resource(
    expected_type: Option<&TypeRefIr>,
    expected_plan: Option<&LinkedValueTransferPlan>,
) -> bool {
    matches!(
        (expected_type, expected_plan),
        (
            Some(TypeRefIr::Builtin { name, args }),
            Some(LinkedValueTransferPlan::AffineResource { .. })
        ) if name == "Stream" && args.len() == 1
    )
}

fn exact_immediate_type_matches(expected: &TypeRefIr, actual: Option<ValueKind>) -> bool {
    let TypeRefIr::Builtin { name, args } = expected else {
        return false;
    };
    if !args.is_empty() {
        return false;
    }
    matches!(
        (name.as_str(), actual),
        ("null", Some(ValueKind::Null))
            | ("bool", Some(ValueKind::Bool))
            | ("number", Some(ValueKind::Number))
            | ("integer", Some(ValueKind::Integer))
            | ("Date", Some(ValueKind::Date))
    )
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::TypeRefIr;
    use skiff_runtime_linked_bytecode::{
        LinkedResourceDropPlan, LinkedValueTransferPlan, TypeIndex,
    };
    use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

    use super::validate_entry_argument;
    use crate::{VmEntryArgumentRejection, VmError};

    const TAG: CompactTypeTag = CompactTypeTag::new(7);
    const FLAGS: ValueFlags = ValueFlags::new(0);
    const HANDLE: VmHandle = VmHandle::new(11);

    #[test]
    fn external_entry_accepts_only_exact_self_describing_builtin_types() {
        let cases = [
            (ValueSlot::null(), "null"),
            (ValueSlot::bool(true), "bool"),
            (ValueSlot::number(1.5), "number"),
            (ValueSlot::integer(-2), "integer"),
            (ValueSlot::date(3), "Date"),
        ];

        for (argument, expected) in cases {
            assert!(validate_entry_argument(
                0,
                &argument,
                TypeIndex::new(0),
                Some(&TypeRefIr::builtin(expected)),
                None,
            )
            .is_ok());
        }
    }

    #[test]
    fn external_entry_rejects_immediate_kind_mismatch_and_composite_types() {
        for expected in [
            TypeRefIr::builtin("integer"),
            TypeRefIr::builtin("boolean"),
            TypeRefIr::builtin("unknown"),
            TypeRefIr::Builtin {
                name: "bool".to_string(),
                args: vec![TypeRefIr::builtin("integer")],
            },
            TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::builtin("bool")),
            },
            TypeRefIr::Union {
                items: vec![TypeRefIr::builtin("bool"), TypeRefIr::builtin("null")],
            },
        ] {
            assert_eq!(
                validate_entry_argument(
                    4,
                    &ValueSlot::bool(true),
                    TypeIndex::new(7),
                    Some(&expected),
                    None,
                ),
                Err(VmError::EntryArgumentTypeMismatch {
                    ordinal: 4,
                    expected: TypeIndex::new(7),
                    actual: Some(skiff_runtime_model::vm_value::ValueKind::Bool),
                })
            );
        }

        assert_eq!(
            validate_entry_argument(4, &ValueSlot::bool(true), TypeIndex::new(7), None, None),
            Err(VmError::EntryArgumentTypeMismatch {
                ordinal: 4,
                expected: TypeIndex::new(7),
                actual: Some(skiff_runtime_model::vm_value::ValueKind::Bool),
            })
        );
    }

    #[test]
    fn external_entry_accepts_heap_refs_with_signature_type() {
        let argument = ValueSlot::request_heap_ref(HANDLE, TAG, FLAGS);
        assert!(validate_entry_argument(
            0,
            &argument,
            TypeIndex::new(0),
            Some(&TypeRefIr::builtin("string")),
            None,
        )
        .is_ok());
    }

    #[test]
    fn external_entry_rejects_image_scoped_constant_handles() {
        let argument = ValueSlot::const_ref(HANDLE, TAG, FLAGS);
        let error = validate_entry_argument(
            0,
            &argument,
            TypeIndex::new(0),
            Some(&TypeRefIr::builtin("integer")),
            None,
        );

        assert_eq!(
            error,
            Err(VmError::EntryArgumentRejected {
                ordinal: 0,
                kind: Some(skiff_runtime_model::vm_value::ValueKind::ConstRef),
                reason: VmEntryArgumentRejection::ImageScopedConstant,
            })
        );
    }

    #[test]
    fn external_entry_rejects_internal_and_affine_handles() {
        let cases = [
            (
                ValueSlot::actor_state_ref(HANDLE, TAG, FLAGS),
                VmEntryArgumentRejection::ActorState,
            ),
            (
                ValueSlot::resource_ref(HANDLE, TAG, FLAGS),
                VmEntryArgumentRejection::AffineResource,
            ),
            (
                ValueSlot::callback_closure_ref(HANDLE, TAG, FLAGS),
                VmEntryArgumentRejection::CallbackClosure,
            ),
        ];

        for (argument, reason) in cases {
            assert!(matches!(
                validate_entry_argument(
                    0,
                    &argument,
                    TypeIndex::new(0),
                    Some(&TypeRefIr::builtin("integer")),
                    None,
                ),
                Err(VmError::EntryArgumentRejected {
                    ordinal: 0,
                    reason: actual,
                    ..
                }) if actual == reason
            ));
        }
    }

    #[test]
    fn external_entry_accepts_exact_affine_stream_resource_with_lifecycle_plan() {
        let argument = ValueSlot::resource_ref(HANDLE, TAG, FLAGS);
        let expected_type = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let expected_plan = LinkedValueTransferPlan::AffineResource {
            drop: LinkedResourceDropPlan::ResourceTableRelease,
        };

        assert!(validate_entry_argument(
            0,
            &argument,
            TypeIndex::new(0),
            Some(&expected_type),
            Some(&expected_plan),
        )
        .is_ok());
    }
}
