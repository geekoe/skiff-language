use std::collections::BTreeMap;

use crate::bytecode::dto::{
    limits, BytecodeArtifact, BytecodeFunctionOrigin, BytecodePoolEntry, BytecodeRelocation,
};
use crate::{PackageExecutableCoordinate, ParamModeIr};

use super::{limit_error, table_error, validate_type_pool_ref, StructuralValidationError};

pub(super) fn validate_function_origins(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let mut ordinary = BTreeMap::<PackageExecutableCoordinate, &str>::new();
    let mut synthetic = BTreeMap::<(PackageExecutableCoordinate, u32), &str>::new();

    for (function_key, function) in &artifact.image.functions {
        match &function.origin {
            BytecodeFunctionOrigin::Executable { executable } => {
                validate_coordinate(function_key, executable, "origin.executable")?;
                if let Some(previous) = ordinary.insert(executable.clone(), function_key) {
                    return Err(table_error(
                        function_key,
                        format!("origin executable coordinate duplicates function {previous:?}"),
                    ));
                }
            }
            BytecodeFunctionOrigin::SyntheticCallback {
                owner,
                site_ordinal,
            } => {
                if function.self_type_ref.is_some() {
                    return Err(table_error(
                        function_key,
                        "synthetic callback must not declare selfTypeRef".to_string(),
                    ));
                }
                validate_coordinate(function_key, owner, "origin.owner")?;
                if *site_ordinal as u64 >= limits::MAX_TABLE_ENTRIES {
                    return Err(limit_error(
                        "MAX_TABLE_ENTRIES",
                        limits::MAX_TABLE_ENTRIES,
                        u64::from(*site_ordinal) + 1,
                        &format!("functions[{function_key}].origin.siteOrdinal"),
                    ));
                }
                if let Some(previous) =
                    synthetic.insert((owner.clone(), *site_ordinal), function_key)
                {
                    return Err(table_error(
                        function_key,
                        format!("synthetic callback origin duplicates function {previous:?}"),
                    ));
                }
            }
        }
        validate_self_layout(function_key, function, artifact)?;
    }

    for ((owner, site_ordinal), function_key) in &synthetic {
        if !ordinary.contains_key(owner) {
            return Err(table_error(
                function_key,
                format!(
                    "synthetic callback site {site_ordinal} owner has no ordinary executable origin"
                ),
            ));
        }
    }

    validate_local_targets(artifact)?;
    validate_callback_layout_targets(artifact)?;
    Ok(())
}

fn validate_coordinate(
    function_key: &str,
    coordinate: &PackageExecutableCoordinate,
    field: &str,
) -> Result<(), StructuralValidationError> {
    let location = |leaf: &str| format!("functions[{function_key}].{field}.{leaf}");
    for (leaf, value) in [
        ("fileIrIdentity", coordinate.file_ir_identity.as_str()),
        ("modulePath", coordinate.module_path.as_str()),
    ] {
        if value.is_empty()
            || value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
        {
            return Err(table_error(
                function_key,
                format!("{field}.{leaf} must be non-empty canonical text"),
            ));
        }
        if value.len() as u64 > limits::MAX_DEBUG_STRING_BYTES {
            return Err(limit_error(
                "MAX_DEBUG_STRING_BYTES",
                limits::MAX_DEBUG_STRING_BYTES,
                value.len() as u64,
                &location(leaf),
            ));
        }
    }
    if coordinate
        .module_path
        .split('.')
        .any(|segment| segment.is_empty())
    {
        return Err(table_error(
            function_key,
            format!("{field}.modulePath is not canonical"),
        ));
    }
    if u64::from(coordinate.executable_index) >= limits::MAX_FUNCTIONS {
        return Err(limit_error(
            "MAX_FUNCTIONS",
            limits::MAX_FUNCTIONS,
            u64::from(coordinate.executable_index) + 1,
            &location("executableIndex"),
        ));
    }
    let prefix = format!("{}::", coordinate.module_path);
    let Some(declaration) = function_key
        .strip_prefix(&prefix)
        .filter(|declaration| !declaration.is_empty())
    else {
        return Err(table_error(
            function_key,
            format!("functionKey must use canonical module prefix {prefix:?}"),
        ));
    };
    if declaration.chars().any(char::is_whitespace)
        || declaration.chars().any(char::is_control)
        || function_key.len() as u64 > limits::MAX_DEBUG_STRING_BYTES
    {
        return Err(table_error(
            function_key,
            "functionKey is not canonical bounded text".to_string(),
        ));
    }
    Ok(())
}

fn validate_self_layout(
    function_key: &str,
    function: &crate::bytecode::dto::RelocatableBytecodeFunction,
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let Some(self_type_ref) = function.self_type_ref else {
        return Ok(());
    };
    validate_type_pool_ref(
        &artifact.image.pools,
        self_type_ref,
        &format!("functions[{function_key}].selfTypeRef"),
    )?;
    let Some(receiver) = function.frame_layout.parameter_slots.first() else {
        return Err(table_error(
            function_key,
            "selfTypeRef requires incoming parameter ordinal 0".to_string(),
        ));
    };
    if receiver.mode != ParamModeIr::Value {
        return Err(table_error(
            function_key,
            "receiver parameter ordinal 0 must use Value mode".to_string(),
        ));
    }
    if function
        .frame_layout
        .slot_type_refs
        .get(receiver.slot as usize)
        .copied()
        != Some(self_type_ref)
    {
        return Err(table_error(
            function_key,
            "receiver slot type must equal selfTypeRef".to_string(),
        ));
    }
    Ok(())
}

fn validate_local_targets(artifact: &BytecodeArtifact) -> Result<(), StructuralValidationError> {
    for (function_key, function) in &artifact.image.functions {
        for (relocation_index, relocation) in function.relocations.iter().enumerate() {
            let location = format!("relocations[{relocation_index}]");
            match relocation {
                BytecodeRelocation::LocalExecutableRef {
                    function_key: target_key,
                    specialization,
                } => {
                    let Some(target) = artifact.image.functions.get(target_key) else {
                        return Err(table_error(
                            function_key,
                            format!("{location} local target {target_key:?} is missing"),
                        ));
                    };
                    if specialization.concrete_receiver.is_some() != target.self_type_ref.is_some()
                    {
                        return Err(table_error(
                            function_key,
                            format!(
                                "{location}.specialization.concreteReceiver presence does not match target selfTypeRef"
                            ),
                        ));
                    }
                }
                BytecodeRelocation::SyntheticCallbackRef {
                    function_key: target_key,
                } => {
                    let Some(target) = artifact.image.functions.get(target_key) else {
                        return Err(table_error(
                            function_key,
                            format!(
                                "{location} synthetic callback target {target_key:?} is missing"
                            ),
                        ));
                    };
                    if !matches!(
                        &target.origin,
                        BytecodeFunctionOrigin::SyntheticCallback { .. }
                    ) {
                        return Err(table_error(
                            function_key,
                            format!("{location} must target a synthetic callback function"),
                        ));
                    }
                }
                BytecodeRelocation::LocalInterfaceRef { interface } => {
                    for (method_index, method) in interface.methods.iter().enumerate() {
                        let Some(target) = artifact.image.functions.get(&method.function_key)
                        else {
                            return Err(table_error(
                                function_key,
                                format!(
                                    "{location}.interface.methods[{method_index}] target is missing"
                                ),
                            ));
                        };
                        if target.self_type_ref.is_none() {
                            return Err(table_error(
                                function_key,
                                format!(
                                    "{location}.interface.methods[{method_index}] target must be receiver-bound"
                                ),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_callback_layout_targets(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    for (index, entry) in artifact.image.pools.callback_capture.iter().enumerate() {
        let BytecodePoolEntry::CallbackCaptureLayout(layout) = entry else {
            continue;
        };
        let Some(target) = artifact.image.functions.get(&layout.function_key) else {
            return Err(table_error(
                &layout.function_key,
                format!("callbackCapture[{index}] target function is missing"),
            ));
        };
        if !matches!(
            &target.origin,
            BytecodeFunctionOrigin::SyntheticCallback { .. }
        ) {
            return Err(table_error(
                &layout.function_key,
                format!("callbackCapture[{index}] must target a synthetic callback function"),
            ));
        }
    }
    Ok(())
}
