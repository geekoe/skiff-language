mod common;
use common::{package_project::compile_package_project, TestDir};
use skiff_artifact_identity::{
    validate_package_artifact_identities, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, NominalTypeRefBaseIr,
    PackageLocalAbiSymbol, PackageTypeRef, TypeRefIr,
};

#[test]
fn package_compile_assigns_self_validating_canonical_identities() {
    let temp = package_project(
        "canonical-identities",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let project = compile_package_project(temp.path()).expect("package should compile");
    let artifact = &project.package.artifact;

    validate_package_artifact_identities(artifact).expect("artifact identities should validate");
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with(PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX));
    assert!(artifact
        .package_local_abi
        .local_abi_identity
        .as_str()
        .starts_with(PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX));
    assert_eq!(
        project.package.published.identity,
        artifact.package_build_id.to_string()
    );
}

#[test]
fn implementation_changes_build_identity_but_not_local_abi() {
    let left = package_project(
        "implementation-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"left\" }\n",
    );
    let right = package_project(
        "implementation-identity-right",
        "run: main.run\n",
        "function run() -> string { return \"right\" }\n",
    );
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_ne!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
    assert_eq!(
        left.package.artifact.package_local_abi.local_abi_identity,
        right.package.artifact.package_local_abi.local_abi_identity
    );
}

#[test]
fn public_api_changes_local_abi_and_build_identity() {
    let left = package_project(
        "api-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let right = package_project(
        "api-identity-right",
        "run: main.run\n",
        "function run(input: string) -> string { return input }\n",
    );
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_ne!(
        left.package.artifact.package_local_abi.local_abi_identity,
        right.package.artifact.package_local_abi.local_abi_identity
    );
    assert_ne!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
}

#[test]
fn package_test_sources_do_not_change_production_artifact_identity() {
    let left = package_project(
        "test-source-identity-left",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    let right = package_project(
        "test-source-identity-right",
        "run: main.run\n",
        "function run() -> string { return \"ok\" }\n",
    );
    left.write(
        "main.test.skiff",
        "test \"left\" { assert true, \"left\" }\n",
    );
    right.write(
        "main.test.skiff",
        "test \"right\" { assert false == false, \"right\" }\n",
    );
    let left = compile_package_project(left.path()).expect("left package should compile");
    let right = compile_package_project(right.path()).expect("right package should compile");

    assert_eq!(
        left.package.artifact.package_build_id,
        right.package.artifact.package_build_id
    );
    assert!(left
        .package
        .file_ir_units
        .iter()
        .all(|file| !file.source_path.ends_with(".test.skiff")));
}

#[test]
fn fully_instantiated_generic_nominal_stays_in_local_abi_and_fails_public_boundary() {
    let temp = package_project(
        "applied-nominal-local-abi",
        "run: main.run\n",
        r#"
type Box<T> { value: T }
function run(value: Box<string>) -> Box<string> { return value }
"#,
    );

    let project = compile_package_project(temp.path()).expect("generic local ABI should compile");
    let artifact = &project.package.artifact;
    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = &artifact.package_local_abi.public_symbols["run"]
    else {
        panic!("run must remain a package callable");
    };
    let assert_box_string = |ty: &PackageTypeRef| {
        let PackageTypeRef::Local {
            local_type: TypeRefIr::AppliedNominal { base, arguments },
        } = ty
        else {
            panic!("package-local signature must retain AppliedNominal");
        };
        assert!(matches!(
            base,
            NominalTypeRefBaseIr::ServiceSymbol { symbol }
                if symbol.module_path == "main" && symbol.symbol == "Box"
        ));
        assert_eq!(arguments, &[TypeRefIr::builtin("string")]);
    };
    assert_box_string(&signature.parameters[0].ty);
    assert_box_string(&signature.return_type);
    assert!(matches!(
        &artifact.boundary_projections[callable_id],
        BoundaryCallableProjection::Unavailable { reasons }
            if reasons.contains(&BoundaryUnavailableReason::UnsupportedBoundaryType)
    ));
}

fn package_project(name: &str, api: &str, source: &str) -> TestDir {
    let temp = TestDir::new("skiff-compiler", name);
    temp.write(
        "package.yml",
        "id: example.com/identity-fixture\nversion: 1.0.0\n",
    );
    temp.write("api.yml", api);
    temp.write("main.skiff", source);
    temp
}
