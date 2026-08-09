use std::collections::BTreeMap;

use skiff_compiler_input::{package_sources::read_package_sources, read_service_package_root};

use super::super::*;

#[test]
fn service_compile_projects_checked_public_instance_operation_facts() {
    let fixture = PublicInstanceServiceFixture::new("positive");
    let (service_root, package) = fixture.read();
    let repository_root = repository_root();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let aliases = BTreeMap::new();
    let package_id = service_root.package.id.as_str().to_string();
    let input = PackageCompileInput::new(&platform_sources, &package, &aliases, &package_id, false);

    let compiled = compile_service_package(input, &service_root)
        .expect("service projection must consume source-owned public-instance facts");

    assert_eq!(compiled.service_api.service_calls, ["worker"]);
    assert_eq!(
        compiled
            .service_api
            .available
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["worker.label"]
    );
    let instance = &compiled.service_api.contract.public_instances["worker"];
    let [interface] = instance.interfaces.as_slice() else {
        panic!("exact source interface row must reach the service contract")
    };
    let [method] = interface.methods.as_slice() else {
        panic!("exact source method slot must reach the service contract")
    };
    assert_eq!(
        compiled.service_api.contract.diagnostic_text.operations[&method.contract_operation_id],
        "worker.label"
    );
}

#[test]
fn selected_public_instance_without_threaded_facts_fails_structurally() {
    let fixture = PublicInstanceServiceFixture::new("missing-facts");
    let (service_root, package) = fixture.read();
    let repository_root = repository_root();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let aliases = BTreeMap::new();
    let package_id = service_root.package.id.as_str().to_string();
    let input = PackageCompileInput::new(&platform_sources, &package, &aliases, &package_id, false);
    let compilation = compile_package(input).expect("ordinary package compilation");
    assert_eq!(
        compilation.public_instance_operations().interfaces().len(),
        1
    );
    let (package, bytecode) = compilation.into_parts();
    let compilation = PackageCompileOutput::try_new(
        package,
        bytecode,
        skiff_compiler_contract::ServicePublicInstanceOperationFacts::default(),
    )
    .expect("empty checked fact bundle is a valid non-service package result");

    let error = project_compiled_service_api(&compilation, &service_root)
        .expect_err("selected public instance must fail closed without its exact facts");
    assert!(matches!(
        error,
        ServicePackageCompileError::ServiceApi(
            ContractDefinitionError::MissingSelectedPublicInstanceOperationFacts {
                public_instance
            }
        ) if public_instance == "worker"
    ));
}

fn repository_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest must have a repository parent")
        .to_path_buf()
}

struct PublicInstanceServiceFixture {
    root: std::path::PathBuf,
}

impl PublicInstanceServiceFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skiff-driver-public-instance-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("current time must follow Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create public-instance driver fixture");
        std::fs::write(
            root.join("package.yml"),
            "id: example.com/driver-public-instance\nversion: 1.0.0\n",
        )
        .expect("write package.yml");
        std::fs::write(
            root.join("api.yml"),
            "worker:\n  const: root.main.worker\n  interfaces:\n    - root.main.WorkerApi\n",
        )
        .expect("write api.yml");
        std::fs::write(
            root.join("service.yml"),
            "id: example.com/driver-public-instance-service\nserviceCalls: [worker]\n",
        )
        .expect("write service.yml");
        std::fs::write(
            root.join("main.skiff"),
            r#"interface WorkerApi {
  function label(self: Self) -> string
}

type Worker implements WorkerApi {}

impl Worker {
  function label() -> string { return "ready" }
}

const worker: Worker = Worker {}
"#,
        )
        .expect("write main.skiff");
        Self { root }
    }

    fn read(
        &self,
    ) -> (
        skiff_compiler_input::ServicePackageRoot,
        crate::PackageSourceInput,
    ) {
        let service_root = read_service_package_root(&self.root).expect("service fixture root");
        let raw_sources =
            read_package_sources(&service_root.package, &self.root).expect("service source files");
        let source_tree = raw_sources.source_tree();
        let source_graph = crate::PublicationSourceGraph::parse_raw_publication_sources(
            &raw_sources.into_source_graph(),
        )
        .expect("parsed service source graph");
        let package = crate::PackageSourceInput::new(
            service_root.package.publication.clone(),
            source_tree,
            source_graph,
            Vec::new(),
        );
        (service_root, package)
    }
}

impl Drop for PublicInstanceServiceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
