#![allow(dead_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(prefix: &str, name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("{prefix}-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const DEFAULT_EXAMPLE_API_YML: &str = r#"
ExampleService: internal.example.ExampleService
api:
  example:
    Input: api.example.Input
    Output: api.example.Output
    ExampleService: api.example.ExampleService
"#;

pub struct ServiceProjectBuilder {
    temp: TestDir,
    service_dir: PathBuf,
}

impl ServiceProjectBuilder {
    pub fn new(name: &str) -> Self {
        let temp = TestDir::new("skiff-compiler", name);
        let service_dir = temp.path().join("service");
        fs::create_dir_all(&service_dir).unwrap();
        Self { temp, service_dir }
    }

    pub fn with_default_manifest(self, service_id: &str) -> Self {
        self.write_root_file(
            "service.yml",
            &format!(
                r#"
id: {service_id}
version: 1.0.0
"#
            ),
        )
        .write_root_file("api.yml", DEFAULT_EXAMPLE_API_YML)
    }

    pub fn package_model(name: &str, imports: &str, body: &str) -> Self {
        Self::with_prefix("skiff-package-model", name)
            .write_root_file(
                "service.yml",
                r#"
id: example.com/example
version: 1.0.0
"#,
            )
            .write_root_file("api.yml", DEFAULT_EXAMPLE_API_YML)
            .write_source(
                "api/example.skiff",
                r#"
            type Input {}
            type Output {}
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
            )
            .write_source(
                "internal/example.skiff",
                &format!(
                    r#"
            {imports}
            type ExampleService {{}}

            impl ExampleService {{
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {{
                {body}
              }}
            }}
        "#
                ),
            )
    }

    pub fn package_model_with_internal_module(
        name: &str,
        internal_module_name: &str,
        internal_module_body: &str,
        impl_imports: &str,
        impl_body: &str,
    ) -> Self {
        Self::with_prefix("skiff-package-path", name)
            .write_root_file(
                "service.yml",
                r#"
id: example.com/example
version: 1.0.0
"#,
            )
            .write_root_file("api.yml", DEFAULT_EXAMPLE_API_YML)
            .write_source(
                "api/example.skiff",
                r#"
            type Input {}
            type Output {}
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
            )
            .write_source(
                &format!("internal/{internal_module_name}.skiff"),
                internal_module_body,
            )
            .write_source(
                "internal/example.skiff",
                &format!(
                    r#"
            {impl_imports}
            type ExampleService {{}}

            impl ExampleService {{
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {{
                {impl_body}
              }}
            }}
        "#
                ),
            )
    }

    fn with_prefix(prefix: &str, name: &str) -> Self {
        let temp = TestDir::new(prefix, name);
        let service_dir = temp.path().to_path_buf();
        fs::create_dir_all(&service_dir).unwrap();
        Self { temp, service_dir }
    }

    pub fn write_root_file(self, relative_path: &str, contents: &str) -> Self {
        self.add_root_file(relative_path, contents);
        self
    }

    pub fn write_source(self, relative_path: &str, contents: &str) -> Self {
        self.add_source(relative_path, contents);
        self
    }

    pub fn add_root_file(&self, relative_path: &str, contents: &str) {
        write_file(self.service_dir.join(relative_path), contents);
    }

    pub fn add_source(&self, relative_path: &str, contents: &str) {
        write_file(self.service_dir.join(relative_path), contents);
    }

    pub fn with_service_package_dependency(self, package_id: &str, alias: Option<&str>) -> Self {
        self.add_service_package_dependency(package_id, alias);
        self
    }

    pub fn add_service_package_dependency(&self, package_id: &str, alias: Option<&str>) {
        let package_entry = match alias {
            Some(alias) => {
                format!("  - id: {package_id}\n    version: 0.1.0\n    alias: {alias}")
            }
            None => format!("  - id: {package_id}\n    version: 0.1.0"),
        };
        self.add_root_file(
            "service.yml",
            &format!(
                r#"
id: example.com/example
version: 1.0.0
packages:
{package_entry}
"#
            ),
        );
        self.add_root_file("api.yml", DEFAULT_EXAMPLE_API_YML);
    }

    pub fn add_local_package(&self, package_id: &str, manifest: &str) {
        write_package_manifest(self.root(), package_id, manifest);
    }

    pub fn add_package_manifest_in_dir(&self, dir_name: &str, manifest: &str) {
        write_package_manifest_in_dir(self.root(), dir_name, manifest);
    }

    pub fn add_package_source(&self, package_id: &str, relative_path: &str, source: &str) {
        write_package_source(self.root(), package_id, relative_path, source);
    }

    pub fn root(&self) -> &Path {
        &self.service_dir
    }

    pub fn path(&self, relative_path: &str) -> PathBuf {
        self.service_dir.join(relative_path)
    }

    pub fn temp_path(&self) -> &Path {
        self.temp.path()
    }
}

pub struct PackageTestFixture {
    project: PackageProjectBuilder,
}

pub struct PackageProjectBuilder {
    temp: TestDir,
    package_dir: PathBuf,
}

pub fn write_package_manifest(root: &Path, package_id: &str, manifest: &str) {
    let actual_package_id = package_manifest_id(manifest).unwrap_or(package_id);
    let package_dir =
        package_store_version_child(root, actual_package_id, package_manifest_version(manifest));
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(package_dir.join("package.yml"), manifest).unwrap();
    write_package_dir_alias(root, package_id, &package_dir);
}

pub fn write_package_manifest_in_dir(root: &Path, dir_name: &str, manifest: &str) {
    let package_id = package_manifest_id(manifest).unwrap_or(dir_name);
    let package_dir =
        package_store_version_child(root, package_id, package_manifest_version(manifest));
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(package_dir.join("package.yml"), manifest).unwrap();
    write_package_dir_alias(root, dir_name, &package_dir);
}

pub fn write_package_source(root: &Path, package_id: &str, relative_path: &str, source: &str) {
    write_file(
        package_store_child(root, package_id).join(relative_path),
        source,
    );
}

pub fn write_package_api_yml(root: &Path, package_id: &str, api_yml: &str) {
    write_file(
        package_store_child(root, package_id).join("api.yml"),
        api_yml,
    );
}

fn package_store_child(root: &Path, name: &str) -> PathBuf {
    if let Some(path) = package_dir_alias(root, name) {
        return path;
    }
    let package_id_dir = root
        .join(".skiff-packages")
        .join(package_store_dir_name(name));
    if let Ok(entries) = fs::read_dir(&package_id_dir) {
        let mut roots = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("package.yml").is_file())
            .collect::<Vec<_>>();
        roots.sort();
        if roots.len() == 1 {
            return roots.remove(0);
        }
    }
    package_id_dir.join("0.1.0")
}

fn package_store_version_child(root: &Path, package_id: &str, version: &str) -> PathBuf {
    root.join(".skiff-packages")
        .join(package_store_dir_name(package_id))
        .join(version)
}

fn package_store_dir_name(package_id: &str) -> String {
    skiff_compiler_core::id::PublicationId::parse(package_id)
        .map(|id| id.artifact_path())
        .unwrap_or_else(|_| package_id.replace('/', "~~").replace('.', "~"))
}

fn package_manifest_id(manifest: &str) -> Option<&str> {
    manifest_scalar_field(manifest, "id")
}

fn package_manifest_version(manifest: &str) -> &str {
    manifest_scalar_field(manifest, "version").unwrap_or("0.1.0")
}

fn manifest_scalar_field<'a>(manifest: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field}:");
    manifest.lines().find_map(|line| {
        let value = line.trim_start().strip_prefix(&prefix)?.trim();
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .or(Some(value))
    })
}

fn write_package_dir_alias(root: &Path, alias: &str, package_dir: &Path) {
    let alias_path = package_dir_alias_path(root, alias);
    fs::create_dir_all(alias_path.parent().unwrap()).unwrap();
    fs::write(alias_path, package_dir.display().to_string()).unwrap();
}

fn package_dir_alias(root: &Path, alias: &str) -> Option<PathBuf> {
    let alias_path = package_dir_alias_path(root, alias);
    fs::read_to_string(alias_path)
        .ok()
        .map(|value| PathBuf::from(value.trim()))
}

fn package_dir_alias_path(root: &Path, alias: &str) -> PathBuf {
    root.join(".skiff-packages")
        .join(".test-aliases")
        .join(alias.replace('/', "__"))
}

pub fn write_complex_cloud_package(root: &Path) {
    write_package_manifest_in_dir(
        root,
        "google.com/cloud",
        r#"
id: google.com/cloud
version: 0.1.0
"#,
    );
    write_package_api_yml(
        root,
        "google.com/cloud",
        r#"
storage:
  upload: cloud.storage.upload
"#,
    );
    write_package_source(
        root,
        "google.com/cloud",
        "cloud/storage.skiff",
        r#"
          function upload() -> string { return "ok" }
        "#,
    );
}

pub fn write_package_with_dependency_alias(root: &Path, alias: &str) {
    write_package_manifest(
        root,
        "example.com/facade",
        &format!(
            r#"
id: example.com/facade
version: 0.1.0
packages:
  - id: google.com/cloud
    version: 0.1.0
    alias: {alias}
"#
        ),
    );
    write_package_api_yml(
        root,
        "example.com/facade",
        r#"
facade: facade_impl.facade
"#,
    );
    write_package_source(
        root,
        "example.com/facade",
        "facade_impl.skiff",
        &format!(
            r#"
          import {alias}

          function facade() -> string {{ return {alias}.storage.upload() }}
"#
        ),
    );
}

pub fn write_package_source_with_friend_test(
    root: &Path,
    default_run: &str,
    test_name: &str,
    test_assertion: &str,
) {
    write_package_source(
        root,
        "example.com/util",
        "util_impl.skiff",
        &format!(
            r#"
          function ok() -> string {{
            return "ok"
          }}

          function helper() -> number {{ return 1 }}
        "#
        ),
    );
    write_package_api_yml(
        root,
        "example.com/util",
        r#"
ok: util_impl.ok
"#,
    );
    write_package_source(
        root,
        "example.com/util",
        "util.test.skiff",
        &format!(
            r#"
          function helper() -> number {{ return 1 }}

          test defaultRun {default_run}

          test "{test_name}" {{
            {test_assertion}
          }}
        "#
        ),
    );
}

pub struct PackageGraphBuilder<'a> {
    service: &'a ServiceProjectBuilder,
}

impl ServiceProjectBuilder {
    pub fn packages(&self) -> PackageGraphBuilder<'_> {
        PackageGraphBuilder { service: self }
    }
}

impl PackageGraphBuilder<'_> {
    pub fn add_package(self, package_id: &str, manifest: &str, sources: &[(&str, &str)]) -> Self {
        write_package_manifest(self.service.root(), package_id, manifest);
        for (relative_path, source) in sources {
            write_package_source(self.service.root(), package_id, relative_path, source);
        }
        self
    }

    pub fn add_package_in_dir(
        self,
        dir_name: &str,
        manifest: &str,
        sources: &[(&str, &str)],
    ) -> Self {
        write_package_manifest_in_dir(self.service.root(), dir_name, manifest);
        for (relative_path, source) in sources {
            write_package_source(self.service.root(), dir_name, relative_path, source);
        }
        self
    }
}

impl PackageProjectBuilder {
    pub fn new(name: &str) -> Self {
        let temp = TestDir::new("skiff-compiler", name);
        let package_dir = temp.path().join("pkg");
        fs::create_dir_all(&package_dir).unwrap();
        Self { temp, package_dir }
    }

    pub fn with_manifest(self, package_id: &str, api_yml: &str) -> Self {
        self.write_root_file(
            "package.yml",
            &format!(
                r#"
id: {package_id}
version: 1.0.0
"#
            ),
        )
        .write_root_file("api.yml", api_yml)
    }

    pub fn write_root_file(self, relative_path: &str, contents: &str) -> Self {
        self.add_root_file(relative_path, contents);
        self
    }

    pub fn write_source(self, relative_path: &str, contents: &str) -> Self {
        self.add_source(relative_path, contents);
        self
    }

    pub fn add_root_file(&self, relative_path: &str, contents: &str) {
        write_file(self.package_dir.join(relative_path), contents);
    }

    pub fn add_source(&self, relative_path: &str, contents: &str) {
        write_file(self.package_dir.join(relative_path), contents);
    }

    pub fn root(&self) -> &Path {
        &self.package_dir
    }

    pub fn path(&self, relative_path: &str) -> PathBuf {
        self.package_dir.join(relative_path)
    }

    pub fn temp_path(&self) -> &Path {
        self.temp.path()
    }
}

impl PackageTestFixture {
    pub fn math(name: &str) -> Self {
        Self {
            project: PackageProjectBuilder::new(name).with_manifest("example.com/math", ""),
        }
    }

    pub fn split(name: &str) -> Self {
        Self {
            project: PackageProjectBuilder::new(name).with_manifest("example.com/split", ""),
        }
    }

    pub fn main(name: &str, manifest: &str) -> Self {
        Self {
            project: PackageProjectBuilder::new(name).write_root_file("package.yml", manifest),
        }
    }

    pub fn source(self, relative_path: &str, contents: &str) -> Self {
        self.project.add_source(relative_path, contents);
        self
    }

    pub fn file(&self, relative_path: &str) -> PathBuf {
        self.project.path(relative_path)
    }

    pub fn root(&self) -> &Path {
        self.project.root()
    }

    pub fn test_file(&self, relative_path: &str) -> PathBuf {
        self.file(relative_path)
    }
}

pub fn write_file(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}
