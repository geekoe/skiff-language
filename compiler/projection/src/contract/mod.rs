#![allow(dead_code)]

mod abi_projection;
mod boundary;
mod conformance;
mod index;
mod model;
mod project;
mod runtime;
mod schema;
mod type_key;

#[allow(unused_imports)]
pub use abi_projection::{abi_type_id_for_named_key, project_abi_identity, AbiIdentityProjection};
pub use boundary::{validate_contract_projection_boundary, BoundaryKind, ContractBoundaryError};
#[allow(unused_imports)]
pub(crate) use boundary::{
    validate_static_type_ref_boundary_policy, BoundaryPackageTypeSource,
    BoundaryTypeRefClosureValidator,
};
#[allow(unused_imports)]
pub use conformance::{
    validate_contract_projection_conformance, ContractConformanceError,
    ContractConformanceViolation,
};
pub use index::ContractProjectionIndex;
#[allow(unused_imports)]
pub use model::{
    ContractAliasProjection, ContractApiBindingProjection, ContractFunctionParamProjection,
    ContractInterfaceOperationProjection, ContractInterfaceProjection,
    ContractOperationBindingProjection, ContractProjection, ContractProjectionTypeBinding,
    ContractProjectionUnit, ContractTypeDescriptorProjection, ContractTypeKind,
    ContractTypeProjection,
};
#[allow(unused_imports)]
pub use project::{project_contract_projection, ContractProjectionError};
#[allow(unused_imports)]
pub use runtime::canonical_contract_projection_schema_json;
#[allow(unused_imports)]
pub use schema::{
    canonical_contract_projection_schema,
    canonical_contract_projection_schema_with_public_instances, CanonicalContractProjectionSchema,
};
#[allow(unused_imports)]
pub use type_key::{
    ContractFunctionTypeParamKey, ContractLiteralKey, ContractNamedTypeKey, ContractPackageRefKey,
    ContractTypeCanonicalizationError, ContractTypeKey,
};
