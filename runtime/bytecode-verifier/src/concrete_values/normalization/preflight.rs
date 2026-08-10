use skiff_artifact_model::{
    bytecode::limits::{MAX_ARTIFACT_BYTES, MAX_NESTING_DEPTH, MAX_POOL_ENTRIES},
    LiteralIr, NominalTypeRefBaseIr, PackageRefIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate};

use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

/// Iterative protection for candidate-controlled `TypeRefIr` bodies. This
/// completes before any recursive equality or policy traversal is attempted.
pub(in crate::concrete_values) fn preflight_candidate_types(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let mut budget = TraversalBudget::new(limits, true);
    for row in candidate.types() {
        budget.inspect(
            row.type_ref(),
            VerificationLocation::Table {
                table: CandidateTable::Types,
                row: row.index().get(),
            },
        )?;
    }
    Ok(())
}

pub(in crate::concrete_values) struct OwnerNormalizationBudget {
    inner: TraversalBudget,
}

impl OwnerNormalizationBudget {
    pub(in crate::concrete_values) const fn new(limits: &VerificationLimits) -> Self {
        Self {
            inner: TraversalBudget::new(limits, false),
        }
    }

    pub(in crate::concrete_values) fn inspect(
        &mut self,
        ty: &TypeRefIr,
        location: VerificationLocation,
    ) -> Result<(), VerificationError> {
        self.inner.inspect(ty, location)
    }

    pub(in crate::concrete_values) const fn used_nodes(&self) -> u64 {
        self.inner.used_nodes
    }

    pub(in crate::concrete_values) const fn used_bytes(&self) -> u64 {
        self.inner.used_bytes
    }

    pub(in crate::concrete_values) const fn max_nodes(&self) -> u64 {
        self.inner.max_nodes
    }

    pub(in crate::concrete_values) const fn max_bytes(&self) -> u64 {
        self.inner.max_bytes
    }

    pub(in crate::concrete_values) fn limit_exceeded(
        &self,
        limit: VerificationLimit,
        additional: u64,
        location: VerificationLocation,
    ) -> VerificationError {
        let (used, max) = match limit {
            VerificationLimit::ValueLifecycleNodes => (self.inner.used_nodes, self.inner.max_nodes),
            VerificationLimit::ValueLifecycleCanonicalBytes => {
                (self.inner.used_bytes, self.inner.max_bytes)
            }
            _ => (0, 0),
        };
        VerificationError::LimitExceeded {
            limit,
            actual: used.checked_add(additional).unwrap_or(u64::MAX),
            max,
            location,
        }
    }
}

struct TraversalBudget {
    max_nodes: u64,
    max_bytes: u64,
    max_depth: u64,
    used_nodes: u64,
    used_bytes: u64,
}

impl TraversalBudget {
    const fn new(limits: &VerificationLimits, hard_resource_caps: bool) -> Self {
        Self {
            max_nodes: if !hard_resource_caps || limits.max_value_lifecycle_nodes < MAX_POOL_ENTRIES
            {
                limits.max_value_lifecycle_nodes
            } else {
                MAX_POOL_ENTRIES
            },
            max_bytes: if !hard_resource_caps
                || limits.max_value_lifecycle_canonical_bytes < MAX_ARTIFACT_BYTES
            {
                limits.max_value_lifecycle_canonical_bytes
            } else {
                MAX_ARTIFACT_BYTES
            },
            max_depth: limits.max_type_nesting_depth,
            used_nodes: 0,
            used_bytes: 0,
        }
    }

    fn inspect(
        &mut self,
        root: &TypeRefIr,
        location: VerificationLocation,
    ) -> Result<(), VerificationError> {
        let mut stack = vec![(root, 1_u64)];
        let mut lower_bound = 0_u64;
        while let Some((ty, depth)) = stack.pop() {
            let effective_max_depth = self.max_depth.min(MAX_NESTING_DEPTH);
            if depth > effective_max_depth {
                return Err(limit(
                    VerificationLimit::TypeNestingDepth,
                    depth,
                    effective_max_depth,
                    location,
                ));
            }
            self.used_nodes = checked_add_limit(
                self.used_nodes,
                1,
                self.max_nodes,
                VerificationLimit::ValueLifecycleNodes,
                location,
            )?;
            let remaining_bytes = self.max_bytes.saturating_sub(self.used_bytes);
            lower_bound = lower_bound
                .checked_add(payload_bytes(ty))
                .unwrap_or(u64::MAX);
            if lower_bound > remaining_bytes {
                return Err(limit(
                    VerificationLimit::ValueLifecycleCanonicalBytes,
                    self.used_bytes.checked_add(lower_bound).unwrap_or(u64::MAX),
                    self.max_bytes,
                    location,
                ));
            }
            let direct_children = child_count(ty);
            let remaining_nodes = self.max_nodes.saturating_sub(self.used_nodes);
            let pending_children = u64::try_from(stack.len()).unwrap_or(u64::MAX);
            let required_nodes = pending_children
                .checked_add(direct_children)
                .unwrap_or(u64::MAX);
            if required_nodes > remaining_nodes {
                return Err(limit(
                    VerificationLimit::ValueLifecycleNodes,
                    self.used_nodes
                        .checked_add(required_nodes)
                        .unwrap_or(u64::MAX),
                    self.max_nodes,
                    location,
                ));
            }
            push_children(ty, depth, &mut stack);
        }

        // The cheap lower bound rejects oversized strings without first
        // allocating another attacker-sized JSON buffer. Depth and nodes have
        // already been bounded before serde performs any recursive work.
        let canonical = skiff_canonical_json::canonical_json_bytes(root).map_err(|error| {
            VerificationError::SemanticViolation {
                obligation: VerificationObligation::ConcreteTypeAndShape,
                location,
                detail: format!("type preflight canonical projection failed: {error}"),
            }
        })?;
        let bytes = u64::try_from(canonical.len()).unwrap_or(u64::MAX);
        self.used_bytes = checked_add_limit(
            self.used_bytes,
            bytes,
            self.max_bytes,
            VerificationLimit::ValueLifecycleCanonicalBytes,
            location,
        )?;
        Ok(())
    }
}

fn child_count(ty: &TypeRefIr) -> u64 {
    let count = match ty {
        TypeRefIr::Builtin { args, .. } => args.len(),
        TypeRefIr::AppliedNominal { arguments, .. } => arguments.len(),
        TypeRefIr::Record { fields } => fields.len(),
        TypeRefIr::Union { items } => items.len(),
        TypeRefIr::Nullable { .. } => 1,
        TypeRefIr::AnyInterface { interface } => interface.canonical_type_args.len(),
        TypeRefIr::Function { params, .. } => params.len().saturating_add(1),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => 0,
    };
    u64::try_from(count).unwrap_or(u64::MAX)
}

fn checked_add_limit(
    used: u64,
    additional: u64,
    max: u64,
    limit_kind: VerificationLimit,
    location: VerificationLocation,
) -> Result<u64, VerificationError> {
    let actual = used.checked_add(additional).unwrap_or(u64::MAX);
    if actual > max {
        return Err(limit(limit_kind, actual, max, location));
    }
    Ok(actual)
}

fn limit(
    limit: VerificationLimit,
    actual: u64,
    max: u64,
    location: VerificationLocation,
) -> VerificationError {
    VerificationError::LimitExceeded {
        limit,
        actual,
        max,
        location,
    }
}

fn push_children<'a>(ty: &'a TypeRefIr, depth: u64, stack: &mut Vec<(&'a TypeRefIr, u64)>) {
    let child_depth = depth.saturating_add(1);
    match ty {
        TypeRefIr::Builtin { args, .. } => push_many(args, child_depth, stack),
        TypeRefIr::AppliedNominal { arguments, .. } => {
            push_many(arguments, child_depth, stack);
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values().rev() {
                stack.push((field, child_depth));
            }
        }
        TypeRefIr::Union { items } => push_many(items, child_depth, stack),
        TypeRefIr::Nullable { inner } => stack.push((inner, child_depth)),
        TypeRefIr::AnyInterface { interface } => {
            push_many(&interface.canonical_type_args, child_depth, stack);
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            stack.push((return_type, child_depth));
            for parameter in params.iter().rev() {
                stack.push((&parameter.ty, child_depth));
            }
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}

fn push_many<'a>(types: &'a [TypeRefIr], depth: u64, stack: &mut Vec<(&'a TypeRefIr, u64)>) {
    for ty in types.iter().rev() {
        stack.push((ty, depth));
    }
}

fn payload_bytes(ty: &TypeRefIr) -> u64 {
    let mut bytes = 1_u64;
    let mut add = |value: &str| {
        bytes = bytes.saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
    };
    match ty {
        TypeRefIr::Builtin { name, .. } => add(name),
        TypeRefIr::PublicationType { module_path, .. } => add(module_path),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            add(&symbol.module_path);
            add(&symbol.symbol);
        }
        TypeRefIr::PackageSymbol { symbol } => {
            match &symbol.package {
                PackageRefIr::PackageId { package_id } => add(package_id),
                PackageRefIr::Dependency { dependency_ref } => add(dependency_ref),
            }
            add(&symbol.symbol_path);
            if let Some(abi) = &symbol.abi_expectation {
                add(abi);
            }
        }
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            add(package_id);
            add(stable_schema_key);
            add(package_schema_type_id.as_str());
        }
        TypeRefIr::AppliedNominal { base, .. } => add_nominal_payload(&mut add, base),
        TypeRefIr::Record { fields } => {
            for name in fields.keys() {
                add(name);
            }
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null | LiteralIr::Bool { .. } => {}
            // `serde_json::Number` has bounded scalar storage in this build;
            // the exact canonical length is charged after the structural walk.
            LiteralIr::Number { .. } => add("0"),
            LiteralIr::String { value } => add(value),
        },
        TypeRefIr::TypeParam { name } => add(name),
        TypeRefIr::AnyInterface { interface } => add(&interface.interface_abi_id),
        TypeRefIr::Function { params, .. } => {
            for parameter in params {
                add(&parameter.name);
            }
        }
        TypeRefIr::LocalType { .. } | TypeRefIr::Union { .. } | TypeRefIr::Nullable { .. } => {}
    }
    bytes
}

fn add_nominal_payload(add: &mut impl FnMut(&str), base: &NominalTypeRefBaseIr) {
    match base {
        NominalTypeRefBaseIr::LocalType { .. } => {}
        NominalTypeRefBaseIr::PublicationType { module_path, .. } => add(module_path),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            add(&symbol.module_path);
            add(&symbol.symbol);
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            match &symbol.package {
                PackageRefIr::PackageId { package_id } => add(package_id),
                PackageRefIr::Dependency { dependency_ref } => add(dependency_ref),
            }
            add(&symbol.symbol_path);
            if let Some(abi) = &symbol.abi_expectation {
                add(abi);
            }
        }
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            add(package_id);
            add(stable_schema_key);
            add(package_schema_type_id.as_str());
        }
    }
}
