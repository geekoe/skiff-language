use std::{collections::HashMap, sync::Arc};

use serde_json::Value;

use crate::program::{PackageSlot, PackageUnit, ProgramError, ProgramResult, ServiceUnit};

pub(super) fn package_activation_configs(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<Vec<Value>> {
    let mut package_configs = vec![Value::Null; packages.len()];
    for dependency in &service.package_dependencies {
        record_dependency_activation_config(
            &dependency.id,
            &dependency.config,
            packages,
            package_slots_by_id,
            &mut package_configs,
        )?;
    }
    for package in packages {
        for dependency in &package.dependencies {
            record_dependency_activation_config(
                &dependency.id,
                &dependency.config,
                packages,
                package_slots_by_id,
                &mut package_configs,
            )?;
        }
    }
    Ok(package_configs)
}

fn record_dependency_activation_config(
    dependency_package_id: &str,
    dependency_config: &Value,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
    package_configs: &mut [Value],
) -> ProgramResult<()> {
    if dependency_config.is_null() {
        return Ok(());
    }
    let Some(slot) = package_slots_by_id.get(dependency_package_id).copied() else {
        return Err(ProgramError::PackageDependencyPackageNotLoaded {
            package_id: dependency_package_id.to_string(),
        });
    };
    let Some(package) = packages.get(slot) else {
        return Err(ProgramError::PackageSlotOutOfBounds {
            slot,
            package_count: packages.len(),
        });
    };
    let current = &mut package_configs[slot];
    if current.is_null() {
        *current = dependency_config.clone();
        return Ok(());
    }
    if merge_activation_config(current, dependency_config) {
        return Ok(());
    }
    Err(ProgramError::PackageConfigConflict {
        package_slot: slot,
        package_id: package.package_id.clone(),
    })
}

fn merge_activation_config(current: &mut Value, incoming: &Value) -> bool {
    match (current, incoming) {
        (Value::Object(current_object), Value::Object(incoming_object)) => {
            for (key, incoming_value) in incoming_object {
                if let Some(current_value) = current_object.get_mut(key) {
                    if !merge_activation_config(current_value, incoming_value) {
                        return false;
                    }
                } else {
                    current_object.insert(key.clone(), incoming_value.clone());
                }
            }
            true
        }
        (current_value, incoming_value) => current_value == incoming_value,
    }
}
