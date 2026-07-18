mod common;

use std::fs;

use common::{artifacts::module_artifact, package_project::compile_package_project, TestDir};
use skiff_syntax::parser::parse_source;

#[test]
fn dotted_and_non_identifier_imports_are_rejected_by_source_syntax() {
    for import in [
        "import connect.mongo",
        "import std.mongo",
        "import skiff.run/foo",
        "import std.anything",
    ] {
        let error = parse_source(&format!(
            "{import}\nfunction rejected() -> number {{ return 1 }}\n"
        ))
        .expect_err("qualified imports should be rejected")
        .to_string();
        assert!(
            error.contains("import name must be a single ASCII identifier"),
            "unexpected parse error for {import}: {error}"
        );
    }
}

#[test]
fn ordinary_find_one_member_stays_plain_file_ir() {
    let temp = TestDir::new("skiff-compiler", "plain-find-one");
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/plain-find-one\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("repo.skiff"),
        r#"
type User { id: string, name: string }
type Repo {}

impl Repo {
  function findOne(id: string) -> User? { return null }
}

function findUser(repo: Repo, id: string) -> User? {
  return repo.findOne(id)
}
"#,
    )
    .unwrap();

    let project = compile_package_project(temp.path()).expect("package should compile");
    let value = module_artifact(&project.package, "repo").value();
    let text = serde_json::to_string_pretty(&value).unwrap();

    assert!(!text.contains(r#""target": "connect.mongo.findOne""#));
    assert!(!text.contains(r#""providerCapability": "connect.mongo/v1""#));
}

#[test]
fn removed_provider_roots_fail_before_package_artifact_projection() {
    let cases = [
        (
            "connect-without-import",
            r#"
type User { id: string, name: string }
function findUser(id: string) -> User? {
  const db = connect.mongo.Target("cluster-a", "app")
  const users = db.Collection<User>("user")
  return users.findOne({ id: id })
}
"#,
            "connect.mongo provider wrapper has been removed",
        ),
        (
            "connect-with-import",
            r#"
import connect
type User { id: string, name: string }
function findUser(id: string) -> User? {
  const db = connect.mongo.Target("cluster-a", "app")
  const users = db.Collection<User>("user")
  return users.findOne({ id: id })
}
"#,
            "connect.mongo provider wrapper has been removed",
        ),
        (
            "std-mongo-root",
            r#"
function rejected() -> number {
  const target = std.mongo.Target("cluster-a", "app")
  return 1
}
"#,
            "std.mongo is not permitted as a std module root",
        ),
        (
            "unknown-root",
            r#"
function rejected() -> number {
  const helper = missing.helper
  return 1
}
"#,
            "unresolved root missing in expression missing.helper",
        ),
        (
            "provider-primitive",
            r#"
function findUser() -> {} {
  return __providerCallFindOne({}, {})
}
"#,
            "internal provider-call primitive __providerCallFindOne",
        ),
    ];

    for (name, source, expected) in cases {
        let temp = TestDir::new("skiff-compiler", name);
        fs::write(
            temp.path().join("package.yml"),
            format!("id: example.com/{name}\nversion: 1.0.0\n"),
        )
        .unwrap();
        fs::write(temp.path().join("repo.skiff"), source).unwrap();

        let error = compile_package_project(temp.path())
            .expect_err("removed provider syntax should fail package compilation")
            .to_string();
        assert!(
            error.contains(expected),
            "unexpected compile error for {name}: {error}"
        );
    }
}
