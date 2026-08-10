mod interface;
mod owner;
mod recursive;
mod schema;

use skiff_artifact_model::TypeRefIr;
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

/// Replaces every caller-relative or source-local nominal reference with the
/// exact owner form that the independent verifier can reconstruct from the
/// same hydration.
pub(in crate::bytecode) fn normalize_type(
    deployment: &HydratedDeploymentBytecode,
    caller: &HydratedBytecodePackage,
    ty: &TypeRefIr,
    location: &BytecodeLinkLocation,
) -> Result<TypeRefIr, BytecodeLinkError> {
    TypeNormalizer {
        deployment,
        caller,
        location,
    }
    .normalize(ty)
}

struct TypeNormalizer<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    caller: &'a HydratedBytecodePackage,
    location: &'a BytecodeLinkLocation,
}

impl TypeNormalizer<'_> {
    fn error(&self, detail: String) -> BytecodeLinkError {
        obligation_error(self.location.clone(), detail)
    }
}

fn obligation_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
        location,
        detail,
    }
}
