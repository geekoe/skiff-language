use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PublicationResourceRef, RuntimeAssembly, RuntimeAssemblyRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_loader::{
    RuntimeAssemblyContentResolver, RuntimeAssemblyLoader, RuntimeAssemblyRecordResolver,
};

struct ExactResolver {
    assembly: Arc<RuntimeAssembly>,
    root_reads: AtomicUsize,
}

impl RuntimeAssemblyRecordResolver for ExactResolver {
    fn resolve_runtime_assembly(
        &self,
        _reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        self.root_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::clone(&self.assembly))
    }
}

impl RuntimeAssemblyContentResolver for ExactResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        anyhow::bail!("empty assembly must not resolve deployments")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        anyhow::bail!("empty assembly must not resolve contracts")
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("empty assembly must not resolve package schema")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        anyhow::bail!("empty assembly must not resolve packages")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        anyhow::bail!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("empty assembly must not resolve resources")
    }
}

fn empty_assembly() -> RuntimeAssembly {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    assembly
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_root_ref_enters_the_production_typed_loader() {
        let assembly = Arc::new(empty_assembly());
        let reference = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
        let resolver = ExactResolver {
            assembly: Arc::clone(&assembly),
            root_reads: AtomicUsize::new(0),
        };

        let hydrated = RuntimeAssemblyLoader::new(&resolver)
            .load_ref(&reference)
            .expect("exact root record should hydrate");

        assert_eq!(
            hydrated.assembly().assembly_identity,
            reference.assembly_identity
        );
        assert_eq!(resolver.root_reads.load(Ordering::SeqCst), 1);
        assert!(hydrated.code_slots().is_empty());
    }

    #[test]
    fn root_content_mismatch_is_rejected_before_graph_hydration() {
        let requested = empty_assembly();
        let reference = skiff_artifact_identity::runtime_assembly_ref(&requested).unwrap();
        let mut tampered = requested;
        tampered.schema_version = "tampered-runtime-assembly".to_string();
        let resolver = ExactResolver {
            assembly: Arc::new(tampered),
            root_reads: AtomicUsize::new(0),
        };

        let error = RuntimeAssemblyLoader::new(&resolver)
            .load_ref(&reference)
            .expect_err("tampered root content must fail closed");

        assert!(error.to_string().contains("runtime assembly"));
        assert_eq!(resolver.root_reads.load(Ordering::SeqCst), 1);
    }
}
