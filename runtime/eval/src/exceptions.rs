use std::collections::HashSet;

use super::*;
use crate::error::{unwrap_diagnostic_source_context, TypeIdentity, WirePayload};
use skiff_runtime_linked_program::{
    FileAddr, LinkedFileUnit, LinkedTypeDescriptor, LinkedTypeRef, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;

pub fn user_exception_for_catch(error: &RuntimeError) -> Option<&UserException> {
    match unwrap_diagnostic_source_context(error) {
        RuntimeError::UserException(exception) => Some(exception),
        _ => None,
    }
}

pub fn exception_envelope_for_catch(
    error: &RuntimeError,
    leaves: &[TypeIdentity],
) -> Result<Option<Value>> {
    if let Some(exception) = user_exception_for_catch(error) {
        if catch_identity_matches(exception.actual_payload_type(), leaves) {
            return Ok(Some(exception.envelope()));
        }
        return Ok(None);
    }
    if leaves.is_empty() {
        return Ok(None);
    }
    let Some((identity, payload)) = error.catch_projection() else {
        return Ok(None);
    };
    if !catch_identity_matches(&identity, leaves) {
        return Ok(None);
    }
    Ok(Some(
        UserException::from_typed_payload(payload, identity.clone(), Some(identity))?.envelope(),
    ))
}

pub fn catch_ok(value: RuntimeValue, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let mut object = RuntimeObjectFields::new();
    object.insert("tag".to_string(), RuntimeValue::String("ok".to_string()));
    object.insert("value".to_string(), value);
    runtime_object_from_fields(object, heap)
}

pub fn catch_err(exception: Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let mut object = RuntimeObjectFields::new();
    object.insert("tag".to_string(), RuntimeValue::String("err".to_string()));
    object.insert(
        "exception".to_string(),
        runtime_from_wire(&exception, heap)?,
    );
    runtime_object_from_fields(object, heap)
}

pub fn throw_payload_actual_type<'p>(
    payload_type: &LinkedTypeRef,
    program: impl Into<ProgramTypeView<'p>>,
) -> Result<TypeIdentity> {
    let program = program.into();
    match payload_type {
        LinkedTypeRef::Address { addr } => {
            program.types.descriptor(addr).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram type address {addr} is not interned"
                ))
            })?;
            if let Some(identity) = standard_error_type_identity_for_addr(addr, program) {
                return Ok(identity);
            }
            Ok(TypeIdentity::address(addr.clone()))
        }
        _ => {
            let leaves = catch_type_leaves(payload_type, program)?;
            match leaves.as_slice() {
                [actual] => Ok(actual.clone()),
                [] => Err(RuntimeError::Decode(
                    "throw payload type must name a concrete error type".to_string(),
                )),
                _ => Err(RuntimeError::Decode(
                    "throw payload type must resolve to exactly one concrete error type"
                        .to_string(),
                )),
            }
        }
    }
}

pub fn catch_type_leaves<'p>(
    catch_type: &LinkedTypeRef,
    program: impl Into<ProgramTypeView<'p>>,
) -> Result<Vec<TypeIdentity>> {
    let program = program.into();
    let mut leaves = Vec::new();
    let mut visiting = HashSet::new();
    collect_catch_type_leaves(catch_type, program, &mut visiting, &mut leaves)?;
    if leaves.is_empty() {
        return Err(RuntimeError::Decode(
            "catchType must name at least one concrete error type".to_string(),
        ));
    }
    Ok(leaves)
}

fn collect_catch_type_leaves(
    catch_type: &LinkedTypeRef,
    program: ProgramTypeView<'_>,
    visiting: &mut HashSet<TypeAddr>,
    leaves: &mut Vec<TypeIdentity>,
) -> Result<()> {
    match catch_type {
        LinkedTypeRef::Address { addr } => {
            let descriptor = program.types.descriptor(addr).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram type address {addr} is not interned"
                ))
            })?;
            if let Some(identity) = standard_error_type_identity_for_addr(addr, program) {
                push_catch_leaf(identity, leaves);
            }
            if !visiting.insert(addr.clone()) {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram type address {addr} has cyclic catch descriptor"
                )));
            }
            match descriptor {
                LinkedTypeDescriptor::Record { .. } | LinkedTypeDescriptor::Native { .. } => {
                    push_catch_leaf(TypeIdentity::address(addr.clone()), leaves);
                }
                LinkedTypeDescriptor::Alias { target } => {
                    collect_catch_type_leaves(&target, program, visiting, leaves)?;
                }
                LinkedTypeDescriptor::Union { variants } => {
                    for variant in variants {
                        collect_catch_type_leaves(&variant, program, visiting, leaves)?;
                    }
                }
            }
            visiting.remove(addr);
        }
        LinkedTypeRef::Union { items } => {
            for item in items {
                collect_catch_type_leaves(item, program, visiting, leaves)?;
            }
        }
        LinkedTypeRef::Native { name, args } if args.is_empty() => {
            if let Some(identity) = standard_error_type_identity(name) {
                push_catch_leaf(identity, leaves);
            }
        }
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram type ref {} was not linked before execution",
                program_types::program_type_ref_kind(catch_type)
            )));
        }
        LinkedTypeRef::Native { .. }
        | LinkedTypeRef::Record { .. }
        | LinkedTypeRef::Nullable { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::TypeParam { .. }
        | LinkedTypeRef::AnyInterface { .. }
        | LinkedTypeRef::Function { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => {}
    }
    Ok(())
}

fn push_catch_leaf(leaf: TypeIdentity, leaves: &mut Vec<TypeIdentity>) {
    if !leaves.contains(&leaf) {
        leaves.push(leaf);
    }
}

fn catch_identity_matches(identity: &TypeIdentity, leaves: &[TypeIdentity]) -> bool {
    leaves.is_empty() || leaves.iter().any(|leaf| leaf == identity)
}

fn standard_error_type_identity(name: &str) -> Option<TypeIdentity> {
    match name {
        "CancelError"
        | "TimeoutError"
        | "config.DecodeError"
        | "std.bytes.DecodeError"
        | "std.number.DecodeError"
        | "std.json.DecodeError"
        | "std.db.DecodeError"
        | "std.file.FileError"
        | "std.resource.ResourceError"
        | "std.time.DecodeError"
        | "std.service.ProviderUnavailableError"
        | "std.service.ProtocolError"
        | "std.http.HttpError" => Some(TypeIdentity::builtin(name)),
        _ => None,
    }
}

fn standard_error_type_identity_for_addr(
    addr: &TypeAddr,
    program: ProgramTypeView<'_>,
) -> Option<TypeIdentity> {
    let symbol = standard_type_symbol_for_addr(addr, program)?;
    standard_error_type_identity(&symbol)
}

fn standard_type_symbol_for_addr(addr: &TypeAddr, program: ProgramTypeView<'_>) -> Option<String> {
    let UnitAddr::Package(slot) = &addr.unit else {
        return None;
    };
    let slot = *slot;
    let package = program.packages.get(slot)?;
    if package.package_id != "skiff.run/std" {
        return None;
    }
    let file = package_file_for_addr(program, slot, &addr.file)?;
    let decl = file.types.get(addr.type_index)?;
    if decl.name.contains('.') {
        return Some(decl.name.clone());
    }
    Some(format!("{}.{}", file.module_path, decl.name))
}

fn package_file_for_addr<'a>(
    program: ProgramTypeView<'a>,
    slot: usize,
    file_addr: &FileAddr,
) -> Option<&'a LinkedFileUnit> {
    let files = program.package_files.get(slot)?;
    match file_addr {
        FileAddr::LoadedFileIndex(index) => files.get(*index).map(AsRef::as_ref),
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file| file.file_ir_identity == *identity)
            .map(AsRef::as_ref),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_envelope_for_catch_returns_exact_user_exception_envelope() {
        let identity = TypeIdentity::builtin("pkg.UserError");
        let exception = UserException::from_typed_payload(
            serde_json::json!({ "message": "boom" }),
            identity.clone(),
            Some(identity.clone()),
        )
        .expect("user exception should be valid");
        let expected_envelope = exception.envelope();
        let error = RuntimeError::UserException(exception)
            .with_diagnostic_frame(serde_json::json!({ "sourceId": 7 }));

        let envelope = exception_envelope_for_catch(&error, &[identity])
            .expect("catch projection should not fail");

        assert_eq!(envelope, Some(expected_envelope));
    }

    #[test]
    fn exception_envelope_for_catch_uses_opaque_wire_payload_projection() {
        let error = RuntimeError::from(
            skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
                target: "svc.account".to_string(),
            },
        );

        let envelope = exception_envelope_for_catch(
            &error,
            &[TypeIdentity::builtin("std.service.ProtocolError")],
        )
        .expect("catch projection should not fail")
        .expect("opaque protocol error should be caught");

        assert_eq!(
            envelope["error"],
            serde_json::json!({
                "target": "svc.account",
                "message": "binary HTTP request metadata is missing",
            })
        );
        assert_eq!(
            envelope["__skiffActualPayloadTypeDebug"],
            "std.service.ProtocolError"
        );
    }
}
