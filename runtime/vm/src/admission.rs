use skiff_runtime_model::vm_value::{ValueKind, ValueSlot};

use crate::{VmEntryArgumentRejection, VmError};

pub(crate) fn validate_entry_arguments(arguments: &[ValueSlot]) -> Result<(), VmError> {
    for (ordinal, argument) in arguments.iter().enumerate() {
        let kind = argument.kind();
        let rejection = match kind {
            None => Some(VmEntryArgumentRejection::InvalidMetadata),
            Some(ValueKind::ConstRef) => Some(VmEntryArgumentRejection::ImageScopedConstant),
            Some(ValueKind::ActorStateRef) => Some(VmEntryArgumentRejection::ActorState),
            Some(ValueKind::ResourceRef) => Some(VmEntryArgumentRejection::AffineResource),
            Some(ValueKind::CallbackClosureRef) => Some(VmEntryArgumentRejection::CallbackClosure),
            Some(
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date
                | ValueKind::RequestHeapRef,
            ) => None,
        };
        if let Some(reason) = rejection {
            return Err(VmError::EntryArgumentRejected {
                ordinal,
                kind,
                reason,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

    use super::validate_entry_arguments;
    use crate::{VmEntryArgumentRejection, VmError};

    const TAG: CompactTypeTag = CompactTypeTag::new(7);
    const FLAGS: ValueFlags = ValueFlags::new(0);
    const HANDLE: VmHandle = VmHandle::new(11);

    #[test]
    fn external_entry_accepts_boundary_owned_values() {
        let values = [
            ValueSlot::null(),
            ValueSlot::bool(true),
            ValueSlot::number(1.5),
            ValueSlot::integer(-2),
            ValueSlot::date(3),
            ValueSlot::request_heap_ref(HANDLE, TAG, FLAGS),
        ];

        assert!(validate_entry_arguments(&values).is_ok());
    }

    #[test]
    fn external_entry_rejects_image_scoped_constant_handles() {
        let error = validate_entry_arguments(&[ValueSlot::const_ref(HANDLE, TAG, FLAGS)]);

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
                validate_entry_arguments(&[argument]),
                Err(VmError::EntryArgumentRejected {
                    ordinal: 0,
                    reason: actual,
                    ..
                }) if actual == reason
            ));
        }
    }
}
