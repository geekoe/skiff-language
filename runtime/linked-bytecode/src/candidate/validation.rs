use std::collections::BTreeSet;

use crate::{
    CandidateTable, LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts, LinkedConstantValue,
};

pub(super) fn validate_parts(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_dense_tables(parts)?;
    validate_unique_keys(parts)?;
    validate_root_function_bounds(parts)
}

fn validate_dense_tables(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_dense(CandidateTable::Functions, &parts.functions, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::ServiceOperations,
        &parts.service_operations,
        |row| row.index().get(),
    )?;
    validate_dense(CandidateTable::ActorMethods, &parts.actor_methods, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::InterfaceTables,
        &parts.interface_tables,
        |row| row.index().get(),
    )?;
    validate_dense(
        CandidateTable::SyntheticCallbacks,
        &parts.synthetic_callbacks,
        |row| row.index().get(),
    )?;
    validate_dense(
        CandidateTable::HostEffectAdapters,
        &parts.host_effect_adapters,
        |row| row.index().get(),
    )?;
    validate_dense(CandidateTable::Types, &parts.types, |row| row.index().get())?;
    validate_dense(CandidateTable::Shapes, &parts.shapes, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::Constants, &parts.constants, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::ResumeSites, &parts.resume_sites, |row| {
        row.index().get()
    })
}

fn validate_dense<T>(
    table: CandidateTable,
    rows: &[T],
    index: impl Fn(&T) -> u32,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut seen = BTreeSet::new();
    for (position, row) in rows.iter().enumerate() {
        let actual = index(row);
        if !seen.insert(actual) {
            return Err(LinkedBytecodeCandidateError::DuplicateIndex {
                table,
                index: actual,
            });
        }
        let expected =
            u32::try_from(position).map_err(|_| LinkedBytecodeCandidateError::TableTooLarge {
                table,
                len: rows.len(),
            })?;
        if actual != expected {
            return Err(LinkedBytecodeCandidateError::NonDenseIndex {
                table,
                position,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

fn validate_unique_keys(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut function_keys = BTreeSet::new();
    for function in &parts.functions {
        if !function_keys.insert(function.key().clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateFunctionKey {
                key: function.key().clone(),
            });
        }
    }

    let mut exact_local_keys = BTreeSet::new();
    for target in &parts.exact_local_targets {
        if !exact_local_keys.insert(target.key().clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateExactLocalTarget {
                key: target.key().clone(),
            });
        }
    }

    let mut service_operations = BTreeSet::new();
    for target in &parts.service_operations {
        let key = (
            target.service_requirement_key().clone(),
            target.contract_operation_id().clone(),
        );
        if !service_operations.insert(key.clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateServiceOperation {
                service_requirement_key: key.0,
                contract_operation_id: key.1,
            });
        }
    }

    let mut actor_methods = BTreeSet::new();
    for target in &parts.actor_methods {
        let key = (
            target.actor_abi_identity().clone(),
            target.method_identity().clone(),
        );
        if !actor_methods.insert(key.clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateActorMethod {
                actor_abi_identity: key.0,
                method_identity: key.1,
            });
        }
    }

    let mut interfaces = BTreeSet::new();
    for table in &parts.interface_tables {
        if !interfaces.insert(table.interface_identity().clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateInterfaceTable {
                interface_identity: table.interface_identity().clone(),
            });
        }
    }
    Ok(())
}

fn validate_root_function_bounds(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    for (position, target) in parts.exact_local_targets.iter().enumerate() {
        check_root_function(
            CandidateTable::ExactLocalTargets,
            position_index(
                CandidateTable::ExactLocalTargets,
                position,
                parts.exact_local_targets.len(),
            )?,
            target.function().get(),
            parts.functions.len(),
        )?;
    }
    for target in &parts.actor_methods {
        check_root_function(
            CandidateTable::ActorMethods,
            target.index().get(),
            target.function().get(),
            parts.functions.len(),
        )?;
    }
    for target in &parts.synthetic_callbacks {
        check_root_function(
            CandidateTable::SyntheticCallbacks,
            target.index().get(),
            target.function().get(),
            parts.functions.len(),
        )?;
    }
    for constant in &parts.constants {
        if let LinkedConstantValue::Behavior(function) = constant.value() {
            check_root_function(
                CandidateTable::Constants,
                constant.index().get(),
                function.get(),
                parts.functions.len(),
            )?;
        }
    }
    Ok(())
}

fn check_root_function(
    source_table: CandidateTable,
    source_index: u32,
    function_index: u32,
    function_len: usize,
) -> Result<(), LinkedBytecodeCandidateError> {
    if function_index as usize >= function_len {
        return Err(LinkedBytecodeCandidateError::RootFunctionOutOfBounds {
            source_table,
            source_index,
            function_index,
            function_len,
        });
    }
    Ok(())
}

fn position_index(
    table: CandidateTable,
    position: usize,
    len: usize,
) -> Result<u32, LinkedBytecodeCandidateError> {
    u32::try_from(position).map_err(|_| LinkedBytecodeCandidateError::TableTooLarge { table, len })
}
