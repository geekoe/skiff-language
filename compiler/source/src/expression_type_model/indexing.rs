use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_compiler_core::type_ref::{debug_text, BuiltinShape};

use crate::{ResolvedTypeRef, TypeResolutionContext, TypeResolutionModel};

#[derive(Clone, Debug)]
pub(super) struct ExactIndexReceiver {
    pub(super) kind: super::SourceIndexReceiverKind,
    pub(super) receiver_type: TypeRefIr,
    pub(super) selector_type: TypeRefIr,
    pub(super) result_type: TypeRefIr,
}

pub(super) fn exact_index_receiver(
    type_resolution: &TypeResolutionModel,
    receiver: &ResolvedTypeRef,
    context: &TypeResolutionContext<'_>,
) -> Result<ExactIndexReceiver, String> {
    let receiver_type = type_resolution.transparent_alias_ir(&receiver.ir, context);
    let TypeRefIr::Builtin { name, args } = &receiver_type else {
        return Err(format!(
            "type `{}` does not support bracket access",
            debug_text(&receiver_type)
        ));
    };

    let (kind, expected_arity) = match BuiltinShape::of_name(name) {
        Some(BuiltinShape::Array) => (super::SourceIndexReceiverKind::Array, 1),
        Some(BuiltinShape::Map) => (super::SourceIndexReceiverKind::Map, 2),
        Some(BuiltinShape::JsonObject) => (super::SourceIndexReceiverKind::JsonObject, 0),
        _ => {
            return Err(format!(
                "type `{}` does not support bracket access",
                debug_text(&receiver_type)
            ));
        }
    };

    if args.len() != expected_arity {
        return Err(format!(
            "index receiver `{name}` expects {expected_arity} type arguments, found {}",
            args.len()
        ));
    }
    let (selector_type, result_type) = match kind {
        super::SourceIndexReceiverKind::Array => (
            TypeRefIr::builtin(BuiltinShape::Integer.name()),
            args[0].clone(),
        ),
        super::SourceIndexReceiverKind::Map => (args[0].clone(), args[1].clone()),
        super::SourceIndexReceiverKind::JsonObject => (
            TypeRefIr::builtin(BuiltinShape::String.name()),
            TypeRefIr::builtin(BuiltinShape::Json.name()),
        ),
    };
    Ok(ExactIndexReceiver {
        kind,
        receiver_type,
        selector_type,
        result_type,
    })
}

pub(super) fn selector_has_exact_type(
    type_resolution: &TypeResolutionModel,
    actual: &ResolvedTypeRef,
    expected: &TypeRefIr,
    context: &TypeResolutionContext<'_>,
) -> bool {
    let actual = type_resolution.transparent_alias_ir(&actual.ir, context);
    let expected = type_resolution.transparent_alias_ir(expected, context);
    actual == expected
        || matches!(
            (&actual, &expected),
            (
                TypeRefIr::Literal {
                    value: LiteralIr::String { .. }
                },
                TypeRefIr::Builtin { name, args }
            ) if matches!(BuiltinShape::of_name(name), Some(BuiltinShape::String))
                && args.is_empty()
        )
}
