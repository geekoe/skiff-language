use crate::shared::ast::TypeRef;

use super::type_index::{PackageDbTypeIndex, PackageDbTypeRecord};

pub(super) struct ValidatedDbFieldPath<'a> {
    pub(super) owner: &'a PackageDbTypeRecord<'a>,
    pub(super) ty: &'a TypeRef,
}

pub(super) fn validate_record_field_path<'a>(
    context: &str,
    type_name: &str,
    field_path: &[String],
    record: &'a PackageDbTypeRecord<'a>,
    type_index: &'a PackageDbTypeIndex<'a>,
    violations: &mut Vec<String>,
) -> Option<ValidatedDbFieldPath<'a>> {
    if field_path.is_empty() {
        violations.push(format!("{context} for {type_name} cannot be empty"));
        return None;
    }
    let mut current = record;
    for (index, segment) in field_path.iter().enumerate() {
        let Some(field) = current.fields.iter().find(|field| field.name == *segment) else {
            violations.push(format!(
                "{context} {} on {} references unknown field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return None;
        };
        if index == field_path.len() - 1 {
            return Some(ValidatedDbFieldPath {
                owner: current,
                ty: field.ty,
            });
        }
        let Some(next_record) = type_index.resolve_record_type(current, field.ty) else {
            violations.push(format!(
                "{context} {} on {} cannot traverse non-record field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return None;
        };
        current = next_record;
    }
    None
}
