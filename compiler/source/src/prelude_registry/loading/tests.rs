use super::*;
use std::{
    env, process,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn package_export_mappings_reject_owner_prefixed_exports() {
    let package_dir = temp_test_dir("owner-prefixed-exports");
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: skiff.run/std
"#,
    )
    .expect("package manifest should be written");
    fs::write(
        package_dir.join("api.yml"),
        r#"
std:
  client:
    Client: client.Client
"#,
    )
    .expect("api.yml should be written");

    let error = package_export_mappings("skiff.run/std", &package_dir).unwrap_err();

    assert!(
        error.contains("must not contain package or service id std"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&package_dir).expect("temporary package dir should be removed");
}

#[test]
fn package_export_mappings_read_root_api_yml_public_modules() {
    let package_dir = temp_test_dir("api-yml-public-modules");
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: skiff.run/std
"#,
    )
    .expect("package manifest should be written");
    fs::write(
        package_dir.join("api.yml"),
        r#"
http:
  HttpRequest: http.HttpRequest
json:
  encode: json.encode
"#,
    )
    .expect("api.yml should be written");

    let mappings = package_export_mappings("skiff.run/std", &package_dir)
        .expect("api.yml mappings should load");

    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].source_module, "http");
    assert_eq!(mappings[0].public_module, "std.http");
    assert_eq!(mappings[1].source_module, "json");
    assert_eq!(mappings[1].public_module, "std.json");

    fs::remove_dir_all(&package_dir).expect("temporary package dir should be removed");
}

#[test]
fn package_export_mappings_missing_api_yml_is_empty() {
    let package_dir = temp_test_dir("missing-api-yml");
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: skiff.run/std
"#,
    )
    .expect("package manifest should be written");

    let mappings = package_export_mappings("skiff.run/std", &package_dir)
        .expect("missing api.yml should be allowed");

    assert!(mappings.is_empty());

    fs::remove_dir_all(&package_dir).expect("temporary package dir should be removed");
}

#[test]
fn package_export_mappings_empty_api_yml_is_empty() {
    let package_dir = temp_test_dir("empty-api-yml");
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: skiff.run/std
"#,
    )
    .expect("package manifest should be written");
    fs::write(package_dir.join("api.yml"), "\n").expect("api.yml should be written");

    let mappings = package_export_mappings("skiff.run/std", &package_dir)
        .expect("empty api.yml should be allowed");

    assert!(mappings.is_empty());

    fs::remove_dir_all(&package_dir).expect("temporary package dir should be removed");
}

#[test]
fn package_export_mappings_reject_top_level_manifest_api() {
    let package_dir = temp_test_dir("manifest-api-removed");
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: skiff.run/std
api:
  - path: std.client
    file: client
"#,
    )
    .expect("package manifest should be written");

    let error = package_export_mappings("skiff.run/std", &package_dir).unwrap_err();

    assert!(
        error.contains("api has been removed; declare public API in api.yml"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&package_dir).expect("temporary package dir should be removed");
}

#[test]
fn native_impl_method_shapes_include_receiver_only_for_instance_methods() {
    let mut registry = PreludeRegistry::empty();
    registry
        .add_source(
            "date",
            r#"
impl Date {
  native static function now() -> Date
  native function addMilliseconds(ms: integer) -> Date
}
"#,
        )
        .expect("native impl source should load");

    let static_shape = &registry
        .raw_declared_native_bindings
        .get("Date.now")
        .expect("static native method should be registered")
        .shape;
    assert_eq!(static_shape.params, Vec::<String>::new());

    let instance_shape = &registry
        .raw_declared_native_bindings
        .get("Date.addMilliseconds")
        .expect("instance native method should be registered")
        .shape;
    assert_eq!(instance_shape.params, vec!["Date", "integer"]);
}

fn temp_test_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let dir = env::temp_dir().join(format!("skiff-compiler-{label}-{}-{unique}", process::id()));
    fs::create_dir_all(&dir).expect("temporary package dir should be created");
    dir
}
