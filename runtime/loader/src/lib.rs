mod deployment;
mod filesystem_resolver;
mod runtime_assembly;
mod utils;

pub use deployment::{
    compose_deployment_assembly, compose_dependency_closure_assembly, DeploymentAssemblyLoader,
    DeploymentReleasePointerResolver,
};
pub use filesystem_resolver::FilesystemRuntimeAssemblyContentResolver;
pub use runtime_assembly::{
    HydratedGatewayCallable, HydratedGatewayEntry, HydratedPackageCodeSlot,
    HydratedRuntimeAssembly, HydratedStaticResource, ResolvedServiceSchema,
    RuntimeAssemblyContentResolver, RuntimeAssemblyLoader, RuntimeAssemblyRecordResolver,
    ServiceContractStore,
};
