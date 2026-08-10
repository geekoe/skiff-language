use skiff_artifact_model::{
    bytecode::limits, LiteralIr, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef,
    ServiceSymbolRef, TypeRefIr,
};

use crate::{
    CandidateLocation, CandidateTable, CandidateTypeStructureLimit, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts,
};

const MAX_LINKED_TYPE_NODES: u64 = limits::MAX_POOL_ENTRIES;
const MAX_LINKED_TYPE_PAYLOAD_BYTES: u64 = limits::MAX_ARTIFACT_BYTES;

#[derive(Clone, Copy)]
struct TypePreflightLimits {
    nodes: u64,
    payload_bytes: u64,
    depth: u64,
}

const AUTHORITY_LIMITS: TypePreflightLimits = TypePreflightLimits {
    nodes: MAX_LINKED_TYPE_NODES,
    payload_bytes: MAX_LINKED_TYPE_PAYLOAD_BYTES,
    depth: limits::MAX_NESTING_DEPTH,
};

pub(super) fn validate_parts(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut budget = TypePreflightBudget::new(AUTHORITY_LIMITS);
    for row in &parts.types {
        validate_type_root(
            row.type_ref(),
            type_location(row.index().get()),
            &mut budget,
        )?;
    }
    for table in &parts.interface_tables {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::InterfaceTables,
            row: table.index().get(),
        };
        let artifact = table.interface().artifact();
        budget.charge_payload(artifact.interface_abi_id.len(), location)?;
        for argument in &artifact.canonical_type_args {
            validate_type_root(argument, location, &mut budget)?;
        }
    }

    // Container, shape, signature, frame, and value positions retain only
    // TypeIndex values. The complete type-table walk above bounds every
    // TypeRefIr those positions can dereference during local validation.
    Ok(())
}

#[cfg(test)]
fn validate_type_roots<'a>(
    roots: impl IntoIterator<Item = (CandidateLocation, &'a TypeRefIr)>,
    limits: TypePreflightLimits,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut budget = TypePreflightBudget::new(limits);
    for (location, root) in roots {
        validate_type_root(root, location, &mut budget)?;
    }
    Ok(())
}

fn validate_type_root(
    root: &TypeRefIr,
    location: CandidateLocation,
    budget: &mut TypePreflightBudget,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut stack = Vec::new();
    budget.schedule(&mut stack, root, 1, location)?;
    while let Some((ty, depth)) = stack.pop() {
        let child_depth = depth.saturating_add(1);
        match ty {
            TypeRefIr::Builtin { name, args } => {
                budget.charge_payload(name.len(), location)?;
                budget.schedule_all(&mut stack, args, child_depth, location)?;
            }
            TypeRefIr::LocalType { .. } => {}
            TypeRefIr::PublicationType { module_path, .. } => {
                budget.charge_payload(module_path.len(), location)?;
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                budget.charge_service_symbol(symbol, location)?;
            }
            TypeRefIr::PackageSymbol { symbol } => {
                budget.charge_package_symbol(symbol, location)?;
            }
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => {
                budget.charge_payload(package_id.len(), location)?;
                budget.charge_payload(stable_schema_key.len(), location)?;
                budget.charge_payload(package_schema_type_id.as_str().len(), location)?;
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                budget.charge_nominal_base(base, location)?;
                budget.schedule_all(&mut stack, arguments, child_depth, location)?;
            }
            TypeRefIr::Record { fields } => {
                for (name, field) in fields {
                    budget.charge_payload(name.len(), location)?;
                    budget.schedule(&mut stack, field, child_depth, location)?;
                }
            }
            TypeRefIr::Union { items } => {
                budget.schedule_all(&mut stack, items, child_depth, location)?;
            }
            TypeRefIr::Nullable { inner } => {
                budget.schedule(&mut stack, inner, child_depth, location)?;
            }
            TypeRefIr::Literal { value } => {
                match value {
                    LiteralIr::String { value } => {
                        budget.charge_payload(value.len(), location)?;
                    }
                    LiteralIr::Number { value } => {
                        // `arbitrary_precision` is not enabled, so this leaf
                        // render is constant-bounded and never recursive.
                        budget.charge_payload(value.to_string().len(), location)?;
                    }
                    LiteralIr::Null | LiteralIr::Bool { .. } => {}
                }
            }
            TypeRefIr::TypeParam { name } => {
                budget.charge_payload(name.len(), location)?;
            }
            TypeRefIr::AnyInterface { interface } => {
                budget.charge_payload(interface.interface_abi_id.len(), location)?;
                budget.schedule_all(
                    &mut stack,
                    &interface.canonical_type_args,
                    child_depth,
                    location,
                )?;
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for parameter in params {
                    budget.charge_payload(parameter.name.len(), location)?;
                    budget.schedule(&mut stack, &parameter.ty, child_depth, location)?;
                }
                budget.schedule(&mut stack, return_type, child_depth, location)?;
            }
        }
    }
    Ok(())
}

struct TypePreflightBudget {
    limits: TypePreflightLimits,
    nodes: u64,
    payload_bytes: u64,
}

impl TypePreflightBudget {
    const fn new(limits: TypePreflightLimits) -> Self {
        Self {
            limits,
            nodes: 0,
            payload_bytes: 0,
        }
    }

    fn schedule<'a>(
        &mut self,
        stack: &mut Vec<(&'a TypeRefIr, u64)>,
        ty: &'a TypeRefIr,
        depth: u64,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        if depth > self.limits.depth {
            return Err(limit_error(
                location,
                CandidateTypeStructureLimit::TypeDepth,
                self.limits.depth,
                depth,
            ));
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.nodes {
            return Err(limit_error(
                location,
                CandidateTypeStructureLimit::TypeNodes,
                self.limits.nodes,
                self.nodes,
            ));
        }
        stack.push((ty, depth));
        Ok(())
    }

    fn schedule_all<'a>(
        &mut self,
        stack: &mut Vec<(&'a TypeRefIr, u64)>,
        types: &'a [TypeRefIr],
        depth: u64,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        for ty in types {
            self.schedule(stack, ty, depth, location)?;
        }
        Ok(())
    }

    fn charge_payload(
        &mut self,
        bytes: usize,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.payload_bytes = self.payload_bytes.saturating_add(bytes);
        if self.payload_bytes > self.limits.payload_bytes {
            return Err(limit_error(
                location,
                CandidateTypeStructureLimit::TypePayloadBytes,
                self.limits.payload_bytes,
                self.payload_bytes,
            ));
        }
        Ok(())
    }

    fn charge_service_symbol(
        &mut self,
        symbol: &ServiceSymbolRef,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        self.charge_payload(symbol.module_path.len(), location)?;
        self.charge_payload(symbol.symbol.len(), location)
    }

    fn charge_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        match &symbol.package {
            PackageRefIr::PackageId { package_id } => {
                self.charge_payload(package_id.len(), location)?;
            }
            PackageRefIr::Dependency { dependency_ref } => {
                self.charge_payload(dependency_ref.len(), location)?;
            }
        }
        self.charge_payload(symbol.symbol_path.len(), location)?;
        if let Some(abi_expectation) = &symbol.abi_expectation {
            self.charge_payload(abi_expectation.len(), location)?;
        }
        Ok(())
    }

    fn charge_nominal_base(
        &mut self,
        base: &NominalTypeRefBaseIr,
        location: CandidateLocation,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        match base {
            NominalTypeRefBaseIr::LocalType { .. } => Ok(()),
            NominalTypeRefBaseIr::PublicationType { module_path, .. } => {
                self.charge_payload(module_path.len(), location)
            }
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                self.charge_service_symbol(symbol, location)
            }
            NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                self.charge_package_symbol(symbol, location)
            }
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => {
                self.charge_payload(package_id.len(), location)?;
                self.charge_payload(stable_schema_key.len(), location)?;
                self.charge_payload(package_schema_type_id.as_str().len(), location)
            }
        }
    }
}

const fn type_location(row: u32) -> CandidateLocation {
    CandidateLocation::TableRow {
        table: CandidateTable::Types,
        row,
    }
}

const fn limit_error(
    location: CandidateLocation,
    limit: CandidateTypeStructureLimit,
    max: u64,
    actual: u64,
) -> LinkedBytecodeCandidateError {
    LinkedBytecodeCandidateError::TypeStructureLimitExceeded {
        location,
        limit,
        max,
        actual,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn limits(nodes: u64, payload_bytes: u64, depth: u64) -> TypePreflightLimits {
        TypePreflightLimits {
            nodes,
            payload_bytes,
            depth,
        }
    }

    fn validate(
        root: &TypeRefIr,
        limits: TypePreflightLimits,
    ) -> Result<(), LinkedBytecodeCandidateError> {
        validate_type_roots([(type_location(0), root)], limits)
    }

    #[test]
    fn wide_record_obeys_exact_node_boundary() {
        const FIELD_COUNT: u32 = 4_096;
        let record = TypeRefIr::Record {
            fields: (0..FIELD_COUNT)
                .map(|type_index| {
                    (
                        format!("field-{type_index:04}"),
                        TypeRefIr::LocalType { type_index },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        };
        let at_limit = u64::from(FIELD_COUNT) + 1;
        assert!(validate(&record, limits(at_limit, u64::MAX, 2)).is_ok());
        assert!(matches!(
            validate(&record, limits(at_limit - 1, u64::MAX, 2)),
            Err(LinkedBytecodeCandidateError::TypeStructureLimitExceeded {
                limit: CandidateTypeStructureLimit::TypeNodes,
                max,
                actual,
                ..
            }) if max == at_limit - 1 && actual == at_limit
        ));
    }

    #[test]
    fn large_string_payload_obeys_exact_byte_boundary() {
        const PAYLOAD_BYTES: usize = 1024 * 1024;
        let ty = TypeRefIr::TypeParam {
            name: "x".repeat(PAYLOAD_BYTES),
        };
        let at_limit = u64::try_from(PAYLOAD_BYTES).expect("test payload fits u64");
        assert!(validate(&ty, limits(1, at_limit, 1)).is_ok());
        assert!(matches!(
            validate(&ty, limits(1, at_limit - 1, 1)),
            Err(LinkedBytecodeCandidateError::TypeStructureLimitExceeded {
                limit: CandidateTypeStructureLimit::TypePayloadBytes,
                max,
                actual,
                ..
            }) if max == at_limit - 1 && actual == at_limit
        ));
    }
}
