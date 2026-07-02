use skiff_compiler::test_support::compile_source_file_ir_artifact_for_test as compile_source_file_ir_artifact;

#[test]
fn connect_mongo_dotted_import_is_rejected_at_parse_time() {
    let source = r#"
        import connect.mongo

        function rejected() -> number {
          return 1
        }
    "#;

    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err()
            .to_string();
    assert!(err.contains("import name must be a single ASCII identifier"));
}

#[test]
fn ordinary_member_find_one_file_ir_does_not_carry_connect_mongo_provider_metadata() {
    let source = r#"
        type User {
          id: string,
          name: string,
        }

        type Repo {}

        impl Repo {
          function findOne(self: Repo, id: string) -> User? {
            return null
          }
        }

        function findUser(repo: Repo, id: string) -> User? {
          return repo.findOne(id)
        }
    "#;

    let artifact =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .expect("source should compile to a file artifact");
    let artifact_text = serde_json::to_string_pretty(&artifact.value()).unwrap();

    assert!(
        !artifact_text.contains(r#""target": "connect.mongo.findOne""#),
        "ordinary member findOne must not lower to connect.mongo provider metadata:\n{artifact_text}"
    );
    assert!(
        !artifact_text.contains(r#""providerCapability": "connect.mongo/v1""#),
        "ordinary member findOne must not carry connect.mongo provider capability:\n{artifact_text}"
    );
}

#[test]
fn connect_mongo_target_call_without_import_is_rejected_at_compile_time() {
    let source = r#"
        type User {
          id: string,
          name: string,
        }

        function findUser(id: string) -> User? {
          const db = connect.mongo.Target("cluster-a", "app")
          const users = db.Collection<User>("user")
          return users.findOne({ id: id })
        }
    "#;

    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err()
            .to_string();
    assert!(
        err.contains("connect.mongo provider wrapper has been removed"),
        "unexpected compile error: {err}"
    );
}

#[test]
fn connect_mongo_target_call_with_import_is_rejected_at_compile_time() {
    let source = r#"
        import connect

        type User {
          id: string,
          name: string,
        }

        function findUser(id: string) -> User? {
          const db = connect.mongo.Target("cluster-a", "app")
          const users = db.Collection<User>("user")
          return users.findOne({ id: id })
        }
    "#;

    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err()
            .to_string();
    assert!(
        err.contains("connect.mongo provider wrapper has been removed"),
        "unexpected compile error: {err}"
    );
}

#[test]
fn std_root_projection_gate_rejects_disallowed_root_expression_at_compile_time() {
    let source = r#"
        function rejected() -> number {
          const target = std.mongo.Target("cluster-a", "app")
          return 1
        }
    "#;

    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err();

    let err = err.to_string();
    assert!(
        err.contains("std.mongo is not permitted as a std module root"),
        "unexpected compile error: {err}"
    );
}

#[test]
fn single_file_artifact_rejects_invalid_std_imports_at_compile_time() {
    for import in [
        "import std.mongo",
        "import skiff.run/foo",
        "import std.anything",
    ] {
        let source = format!(
            r#"
            {import}

            function rejected() -> number {{
              return 1
            }}
        "#
        );

        let err = compile_source_file_ir_artifact(
            &source,
            "internal/repo.skiff",
            "internal.repo",
            "service",
        )
        .unwrap_err();
        let err = err.to_string();
        assert!(
            err.contains("import name must be a single ASCII identifier"),
            "unexpected compile error for {import}: {err}"
        );
    }
}

#[test]
fn unknown_root_requires_explicit_import_at_compile_time() {
    let source = r#"
        function rejected() -> number {
          const helper = missing.helper
          return 1
        }
    "#;
    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err()
            .to_string();
    assert!(
        err.contains("unresolved root missing in expression missing.helper"),
        "unexpected compile error: {err}"
    );
}

#[test]
fn service_source_internal_provider_primitive_is_rejected_at_compile_time() {
    let source = r#"
        function findUser() -> {} {
          return __providerCallFindOne({}, {})
        }
    "#;

    let err =
        compile_source_file_ir_artifact(source, "internal/repo.skiff", "internal.repo", "service")
            .unwrap_err()
            .to_string();
    assert!(
        err.contains("internal provider-call primitive __providerCallFindOne"),
        "unexpected compile error: {err}"
    );
}
