use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::{
    error::{Result, RuntimeError},
    runtime_value::{RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue},
    type_descriptor::RuntimeRecordFieldPlan as RecordField,
};

use super::keys::runtime_field_name_from_map_key;

#[derive(Clone, Copy, Debug)]
pub(super) enum RecordProjectionSource {
    Wire,
    Runtime,
}

impl RecordProjectionSource {
    fn should_reject_extra_key(self, key: &str) -> bool {
        let _ = self;
        let _ = key;
        true
    }

    fn missing_required_error(self, field_name: &str) -> RuntimeError {
        let prefix = match self {
            Self::Wire => "record field",
            Self::Runtime => "runtime record field",
        };
        RuntimeError::Decode(format!("{prefix} {field_name} is required"))
    }
}

pub(super) struct RuntimeRecordShape<'a> {
    fields: &'a [RecordField],
}

impl<'a> RuntimeRecordShape<'a> {
    pub(super) fn new(fields: &'a [RecordField]) -> Self {
        Self { fields }
    }

    pub(super) fn for_plan(fields: &'a [RecordField], _boundary_record_kind: Option<&str>) -> Self {
        Self { fields }
    }

    pub(super) fn runtime_object(&self, fields: RuntimeObjectFields) -> RuntimeObject {
        RuntimeObject::unshaped(fields)
    }

    pub(super) fn project_json_object<'source>(
        &self,
        object: &'source Map<String, Value>,
    ) -> Result<RecordProjection<'a, 'source, Value>> {
        self.project(
            object.keys(),
            |field_name| object.get(field_name),
            RecordProjectionSource::Wire,
        )
    }

    pub(super) fn project_runtime_fields<'source>(
        &self,
        fields: &'source RuntimeObjectFields,
        source: RecordProjectionSource,
    ) -> Result<RecordProjection<'a, 'source, RuntimeValue>> {
        self.project(fields.keys(), |field_name| fields.get(field_name), source)
    }

    fn project<'source, T>(
        &self,
        keys: impl Iterator<Item = &'source String>,
        mut get: impl FnMut(&str) -> Option<&'source T>,
        source: RecordProjectionSource,
    ) -> Result<RecordProjection<'a, 'source, T>> {
        self.reject_extra_fields(keys, source)?;
        let mut projected_fields = Vec::with_capacity(self.fields.len());
        for field in self.fields {
            let value = match get(&field.name) {
                Some(value) => RecordProjectionValue::Present(value),
                None if field.required => return Err(source.missing_required_error(&field.name)),
                None => RecordProjectionValue::MissingOptionalNull,
            };
            projected_fields.push(RecordProjectionField { field, value });
        }
        Ok(RecordProjection {
            fields: projected_fields,
        })
    }

    fn reject_extra_fields<'source>(
        &self,
        keys: impl Iterator<Item = &'source String>,
        source: RecordProjectionSource,
    ) -> Result<()> {
        let allowed = self
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<BTreeSet<_>>();
        for key in keys {
            if source.should_reject_extra_key(key) && !allowed.contains(key.as_str()) {
                return Err(RuntimeError::Decode(format!(
                    "record field {key} is not declared by descriptor"
                )));
            }
        }
        Ok(())
    }
}

pub(super) struct RecordProjection<'fields, 'source, T> {
    fields: Vec<RecordProjectionField<'fields, 'source, T>>,
}

impl<'fields, 'source, T> RecordProjection<'fields, 'source, T> {
    pub(super) fn into_fields(self) -> Vec<RecordProjectionField<'fields, 'source, T>> {
        self.fields
    }
}

pub(super) struct RecordProjectionField<'fields, 'source, T> {
    pub(super) field: &'fields RecordField,
    pub(super) value: RecordProjectionValue<'source, T>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RecordProjectionValue<'source, T> {
    Present(&'source T),
    MissingOptionalNull,
}

pub(super) fn runtime_object_fields_from_map(map: RuntimeMap) -> Result<RuntimeObjectFields> {
    let mut fields = RuntimeObjectFields::new();
    for (key, value) in map {
        fields.insert(runtime_field_name_from_map_key(&key)?, value);
    }
    Ok(fields)
}
