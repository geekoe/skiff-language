use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    OperationAbiRef, PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationSchemaType,
};

use crate::operation::{operation_abi_identity, OperationAbiIdentityInput};
use crate::publication::publication_abi_identity;
use crate::{ArtifactIdentityError, Result};

/// Validates the typed operation/schema reference graph without consulting compiler or storage.
pub fn validate_publication_abi_surface(unit: &PublicationAbiUnit) -> Result<()> {
    let exports = operation_index(&unit.operation_exports, "operationExports")?;
    let operation_abi = publication_operation_index(&unit.operation_abi)?;

    for operation_id in exports.keys() {
        if !operation_abi.contains_key(operation_id) {
            return invalid(format!(
                "operationExports references operationAbiId {operation_id} without an operationAbi descriptor"
            ));
        }
    }
    for operation_id in operation_abi.keys() {
        if !exports.contains_key(operation_id) {
            return invalid(format!(
                "operationAbi descriptor {operation_id} is not present in operationExports"
            ));
        }
    }

    let schema = schema_index(&unit.schema_closure, "schemaClosure")?;
    for descriptor in &unit.operation_abi {
        validate_operation_descriptor(descriptor, &exports, &schema)?;
    }
    validate_source_call_index(unit, &exports)?;
    validate_public_instances(unit, &exports)?;
    Ok(())
}

/// Validates the complete surface and then recomputes the declared publication ABI identity.
pub fn validate_publication_abi_identity(unit: &PublicationAbiUnit) -> Result<()> {
    validate_publication_abi_surface(unit)?;
    let computed = publication_abi_identity(unit)?;
    if unit.abi_identity != computed {
        return Err(ArtifactIdentityError::PublicationAbiIdentityMismatch {
            declared: unit.abi_identity.clone(),
            computed,
        });
    }
    Ok(())
}

fn operation_index<'a>(
    operations: &'a [OperationAbiRef],
    label: &str,
) -> Result<BTreeMap<&'a str, &'a OperationAbiRef>> {
    let mut index = BTreeMap::new();
    for operation in operations {
        validate_operation_ref_shape(operation, label)?;
        let operation_id = operation.operation_abi_id.as_str();
        if index.insert(operation_id, operation).is_some() {
            return invalid(format!("{label} duplicates operationAbiId {operation_id}"));
        }
    }
    Ok(index)
}

fn publication_operation_index<'a>(
    operations: &'a [PublicationOperationAbi],
) -> Result<BTreeMap<&'a str, &'a PublicationOperationAbi>> {
    let mut index = BTreeMap::new();
    for operation in operations {
        let operation_id = operation.operation.operation_abi_id.as_str();
        if operation_id.is_empty() {
            return invalid("operationAbi contains an empty operationAbiId".to_string());
        }
        if index.insert(operation_id, operation).is_some() {
            return invalid(format!(
                "operationAbi duplicates operationAbiId {operation_id}"
            ));
        }
    }
    Ok(index)
}

fn schema_index<'a>(
    schema: &'a [PublicationSchemaType],
    label: &str,
) -> Result<BTreeMap<&'a str, &'a PublicationSchemaType>> {
    let mut index = BTreeMap::new();
    for schema_type in schema {
        if schema_type.abi_type_id.is_empty() {
            return invalid(format!("{label} contains an empty abiTypeId"));
        }
        let key = schema_type.abi_type_id.as_str();
        if index.insert(key, schema_type).is_some() {
            return invalid(format!("{label} duplicates abiTypeId {key}"));
        }
    }
    Ok(index)
}

fn validate_operation_descriptor(
    descriptor: &PublicationOperationAbi,
    exports: &BTreeMap<&str, &OperationAbiRef>,
    publication_schema: &BTreeMap<&str, &PublicationSchemaType>,
) -> Result<()> {
    validate_operation_ref_shape(&descriptor.operation, "operationAbi")?;
    let operation_id = descriptor.operation.operation_abi_id.as_str();
    let Some(export) = exports.get(operation_id) else {
        return invalid(format!(
            "operationAbi descriptor {operation_id} is not exported"
        ));
    };
    ensure_same_operation_ref(
        export,
        &descriptor.operation,
        &format!("operationAbi descriptor {operation_id}"),
    )?;

    let descriptor_schema = schema_index(
        &descriptor.schema_closure,
        &format!("operationAbi {operation_id} schemaClosure"),
    )?;
    for (key, schema_type) in descriptor_schema {
        let Some(publication_schema_type) = publication_schema.get(key) else {
            return invalid(format!(
                "operationAbi {operation_id} schemaClosure key {key} is missing from publication schemaClosure"
            ));
        };
        if *publication_schema_type != schema_type {
            return invalid(format!(
                "operationAbi {operation_id} schemaClosure key {key} does not match publication schemaClosure"
            ));
        }
    }

    let computed = operation_abi_identity(&OperationAbiIdentityInput {
        kind: descriptor.operation.kind,
        public_path: &descriptor.operation.public_path,
        public_instance_key: descriptor.operation.public_instance_key.as_deref(),
        interface: descriptor.operation.interface.as_ref(),
        method_abi_id: descriptor.operation.method_abi_id.as_deref(),
        public_signature: &descriptor.public_signature,
        schema_closure: &descriptor.schema_closure,
        stream_effect_throw_config: &descriptor.stream_effect_throw_config,
    })?;
    if operation_id != computed {
        return invalid(format!(
            "operationAbi {operation_id} does not match descriptor identity {computed}"
        ));
    }
    Ok(())
}

fn validate_source_call_index(
    unit: &PublicationAbiUnit,
    exports: &BTreeMap<&str, &OperationAbiRef>,
) -> Result<()> {
    let mut paths = BTreeSet::new();
    for entry in &unit.source_call_operation_index {
        if !paths.insert(entry.source_call_path.as_str()) {
            return invalid(format!(
                "sourceCallOperationIndex duplicates sourceCallPath {}",
                entry.source_call_path
            ));
        }
        let operation_id = entry.operation.operation_abi_id.as_str();
        let Some(export) = exports.get(operation_id) else {
            return invalid(format!(
                "sourceCallOperationIndex path {} targets dangling operationAbiId {operation_id}",
                entry.source_call_path
            ));
        };
        ensure_same_operation_ref(
            export,
            &entry.operation,
            &format!("sourceCallOperationIndex path {}", entry.source_call_path),
        )?;
    }
    Ok(())
}

fn validate_public_instances(
    unit: &PublicationAbiUnit,
    exports: &BTreeMap<&str, &OperationAbiRef>,
) -> Result<()> {
    let mut instance_keys = BTreeSet::new();
    for instance in &unit.public_instances {
        if instance.public_instance_key.is_empty() {
            return invalid("publicInstances contains an empty publicInstanceKey".to_string());
        }
        if !instance_keys.insert(instance.public_instance_key.as_str()) {
            return invalid(format!(
                "publicInstances duplicates publicInstanceKey {}",
                instance.public_instance_key
            ));
        }

        let mut interface_keys = BTreeSet::new();
        for interface in &instance.interfaces {
            let key = crate::semantic::canonical_interface_instantiation_key(interface);
            if !interface_keys.insert(key) {
                return invalid(format!(
                    "public instance {} duplicates an interface instantiation",
                    instance.public_instance_key
                ));
            }
        }

        let methods = operation_index(
            &instance.method_operations,
            &format!(
                "public instance {} methodOperations",
                instance.public_instance_key
            ),
        )?;
        for operation in methods.values() {
            if operation.kind != PublicationOperationKind::PublicInstanceMethod
                || operation.public_instance_key.as_deref()
                    != Some(instance.public_instance_key.as_str())
            {
                return invalid(format!(
                    "public instance {} method operation {} has a mismatched kind or publicInstanceKey",
                    instance.public_instance_key, operation.operation_abi_id
                ));
            }
            let Some(export) = exports.get(operation.operation_abi_id.as_str()) else {
                return invalid(format!(
                    "public instance {} targets dangling operationAbiId {}",
                    instance.public_instance_key, operation.operation_abi_id
                ));
            };
            ensure_same_operation_ref(
                export,
                operation,
                &format!(
                    "public instance {} method operation {}",
                    instance.public_instance_key, operation.operation_abi_id
                ),
            )?;
            let Some(interface) = operation.interface.as_ref() else {
                return invalid(format!(
                    "public instance {} method operation {} has no interface",
                    instance.public_instance_key, operation.operation_abi_id
                ));
            };
            if !interface_keys.contains(&crate::semantic::canonical_interface_instantiation_key(
                interface,
            )) {
                return invalid(format!(
                    "public instance {} method operation {} targets an undeclared interface",
                    instance.public_instance_key, operation.operation_abi_id
                ));
            }
        }

        let mut method_names = BTreeSet::new();
        for entry in &instance.source_call_method_index {
            if !method_names.insert(entry.method_name.as_str()) {
                return invalid(format!(
                    "public instance {} sourceCallMethodIndex duplicates methodName {}",
                    instance.public_instance_key, entry.method_name
                ));
            }
            let operation_id = entry.operation.operation_abi_id.as_str();
            let Some(method) = methods.get(operation_id) else {
                return invalid(format!(
                    "public instance {} sourceCallMethodIndex method {} targets dangling operationAbiId {operation_id}",
                    instance.public_instance_key, entry.method_name
                ));
            };
            ensure_same_operation_ref(
                method,
                &entry.operation,
                &format!(
                    "public instance {} sourceCallMethodIndex method {}",
                    instance.public_instance_key, entry.method_name
                ),
            )?;
        }
    }
    Ok(())
}

fn validate_operation_ref_shape(operation: &OperationAbiRef, label: &str) -> Result<()> {
    if operation.operation_abi_id.is_empty() {
        return invalid(format!("{label} contains an empty operationAbiId"));
    }
    match operation.kind {
        PublicationOperationKind::PublicFunction => {
            if operation.public_instance_key.is_some()
                || operation.interface.is_some()
                || operation.method_abi_id.is_some()
            {
                return invalid(format!(
                    "{label} operation {} is a public function but carries method fields",
                    operation.operation_abi_id
                ));
            }
        }
        PublicationOperationKind::PublicInstanceMethod => {
            if operation
                .public_instance_key
                .as_deref()
                .is_none_or(str::is_empty)
                || operation.interface.is_none()
                || operation.method_abi_id.as_deref().is_none_or(str::is_empty)
            {
                return invalid(format!(
                    "{label} operation {} is a public instance method but lacks method fields",
                    operation.operation_abi_id
                ));
            }
        }
    }
    Ok(())
}

fn ensure_same_operation_ref(
    expected: &OperationAbiRef,
    actual: &OperationAbiRef,
    label: &str,
) -> Result<()> {
    if expected.operation_abi_id != actual.operation_abi_id
        || expected.kind != actual.kind
        || expected.public_path != actual.public_path
        || expected.public_instance_key != actual.public_instance_key
        || expected.interface != actual.interface
        || expected.method_abi_id != actual.method_abi_id
    {
        return invalid(format!(
            "{label} does not match exported operation ref {}",
            expected.operation_abi_id
        ));
    }
    Ok(())
}

fn invalid<T>(message: String) -> Result<T> {
    Err(ArtifactIdentityError::InvalidPublicationAbiSurface { message })
}
