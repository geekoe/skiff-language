use std::fs;

mod common;
use common::{
    cli_command::{assert_success, CliCommand},
    TestDir,
};

#[test]
fn cli_applies_service_profile_overlay() {
    let temp = TestDir::new("skiff-compiler", "service-profile-overlay");
    let service_root = temp.path().join("service");
    fs::create_dir_all(service_root.join("api")).unwrap();
    fs::create_dir_all(service_root.join("internal")).unwrap();
    fs::write(
        service_root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
timeout:
  default: 1000
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api.yml"),
        r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("service.prod.yml"),
        r#"
timeout:
  default: 9000
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("api").join("example.skiff"),
        r#"
type Output {}
interface ExampleService {
  function handle() -> Output
}
"#,
    )
    .unwrap();
    fs::write(
        service_root.join("internal").join("example.skiff"),
        r#"
function handle() -> root.api.example.Output {
  return {}
}

type ExampleService {}

impl ExampleService {
  function handle(self: ExampleService) -> root.api.example.Output {
    return root.internal.example.handle()
  }
}
"#,
    )
    .unwrap();

    let out_path = temp.path().join("service-assembly.json");
    let manifest_path = temp.path().join("router-manifest.json");
    let output = CliCommand::compile(&service_root)
        .arg("--out")
        .arg(&out_path)
        .arg("--manifest-out")
        .arg(&manifest_path)
        .arg("--profile")
        .arg("prod")
        .output();

    assert_success(&output);

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["timeout"]["defaultMs"], 9000);
}
