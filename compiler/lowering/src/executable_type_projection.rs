use skiff_artifact_model::{PackageTypeRef, TypeRefIr};
use skiff_compiler_core::type_ref::package_type_ref_to_ir;

/// Projects an exact source type into the representation needed to execute a
/// File IR body. Contract identity deliberately stays outside File IR.
pub(crate) fn execution_type_ref(ty: &PackageTypeRef) -> TypeRefIr {
    package_type_ref_to_ir(ty)
}

#[cfg(test)]
mod tests;
