mod common;

use std::sync::Arc;

use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, PackageCodeSlot, RuntimeAssembly,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{
    HydratedPackageCode, LinkedActorMethodImplementation, LinkedCallTarget, LinkedExprIr,
    LinkedTypeRef, PublicationResourceTable,
};

#[test]
fn real_actor_source_links_to_routed_actor_dispatch() {
    let project_dir = TestDir::new("skiff-compiler", "actor-dispatch-linking");
    project_dir.write(
        "package.yml",
        "id: example.com/actor-dispatch-linking\nversion: 1.0.0\n",
    );
    project_dir.write("api.yml", "{}\n");
    project_dir.write(
        "main.skiff",
        r#"
actor UserActor id string {
  displayName: string,
}

impl UserActor {
  function rename(self: UserActor, value: string) -> string {
    self.displayName = value
    return self.displayName
  }
}

function invoke(actor: UserActor) -> string {
  return actor.rename("Grace")
}
"#,
    );

    let project =
        compile_package_project(project_dir.path()).expect("real Actor source should compile");
    let package = project.package.artifact.clone();
    let package_ref =
        skiff_artifact_identity::package_artifact_ref(&package).expect("package must be canonical");
    let source_files = project
        .package
        .file_ir_units
        .iter()
        .map(|published| Arc::new(published.unit.clone()))
        .collect::<Vec<_>>();
    let source_declaration = source_files
        .iter()
        .flat_map(|file| &file.actor_declarations)
        .find(|declaration| declaration.abi.actor_name == "UserActor")
        .expect("compiler artifact should contain the Actor declaration");
    assert!(
        source_files.iter().all(|file| file
            .declarations
            .types
            .values()
            .all(|declaration| !declaration.symbol.ends_with(".UserActor"))),
        "Actor handle must not be represented by a fake type declaration"
    );
    let source_method = source_declaration
        .abi
        .public_methods
        .iter()
        .find(|method| method.name == "rename")
        .expect("compiler artifact should contain the Actor method");
    let source_actor_abi = source_declaration.actor_abi_identity.clone();
    let source_actor_implementation = source_declaration.actor_implementation_identity.clone();
    let source_method_identity = source_method.method_identity.clone();

    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("actor-source-link-fixture"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref,
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = skiff_runtime_linker::link_package_fixture_from_runtime_assembly(
        &assembly,
        [HydratedPackageCode::new(
            Arc::new(package.clone()),
            source_files,
            PublicationResourceTable::default(),
        )],
    )
    .expect("compiler artifact should link");

    let linked_file = image
        .code_by_build(&package.package_build_id)
        .expect("compiled package should have a linked code slot")
        .files()
        .iter()
        .find(|file| file.module_path == "main")
        .expect("main source file should be linked");
    let linked_declaration = linked_file
        .actor_declarations
        .iter()
        .find(|declaration| declaration.actor_name == "UserActor")
        .expect("linked file should retain the Actor declaration");
    assert_eq!(linked_declaration.actor_abi_identity, source_actor_abi);
    assert_eq!(
        linked_declaration.actor_implementation_identity,
        source_actor_implementation
    );

    let invoke = linked_file
        .executables
        .iter()
        .find(|executable| executable.symbol.ends_with(".invoke"))
        .expect("source invoke function should be linked");
    assert!(matches!(
        &invoke.params[0].ty,
        LinkedTypeRef::ServiceSymbol { symbol }
            if symbol.module_path == "main" && symbol.symbol == "UserActor"
    ));
    let dispatch = invoke
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call } => match &call.target {
                LinkedCallTarget::ActorDispatch { plan } => Some(plan),
                _ => None,
            },
            _ => None,
        })
        .expect("Actor receiver call must become routed ActorDispatch");
    assert_eq!(dispatch.declaration_owner.actor_symbol, "UserActor");
    assert_eq!(dispatch.actor_abi_identity, source_actor_abi);
    assert_eq!(
        dispatch.actor_implementation_identity,
        source_actor_implementation
    );
    assert_eq!(dispatch.method_identity, source_method_identity);

    let linked_method = linked_declaration
        .public_methods
        .iter()
        .find(|method| method.name == "rename")
        .expect("linked declaration should retain the method table");
    assert_eq!(linked_method.method_identity, source_method_identity);
    assert!(matches!(
        linked_method.implementation,
        LinkedActorMethodImplementation::Executable { .. }
    ));
}
