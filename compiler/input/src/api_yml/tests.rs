use super::*;

fn parse(text: &str) -> Result<PublicationApiSpec, PublicationApiYmlError> {
    parse_publication_api_yml(text, Path::new("api.yml"))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "skiff-api-yml-{name}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn missing_and_blank_are_rejected_while_empty_mapping_is_empty_api() {
    let temp = temp_dir("missing");
    assert!(read_publication_api_yml(&temp)
        .unwrap_err()
        .to_string()
        .contains("api.yml is required"));
    let _ = std::fs::remove_dir_all(temp);

    assert!(parse("")
        .unwrap_err()
        .to_string()
        .contains("must not be empty"));
    assert!(parse("   \n")
        .unwrap_err()
        .to_string()
        .contains("must not be empty"));
    assert!(parse("{}\n").unwrap().is_empty());
}

#[test]
fn flattens_nested_public_paths_and_splits_selector() {
    let spec = parse(
        r#"
decode: decode.decode
LlmRequest: types.LlmRequest
http:
  Request: http.HttpRequest
  sse: http.sse
"#,
    )
    .unwrap();

    let entries = spec.entries().collect::<Vec<_>>();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].public_path_string(), "decode");
    assert_eq!(entries[0].source_module_hint(), "decode");
    assert_eq!(entries[0].source_symbol(), "decode");
    assert_eq!(entries[2].public_path_string(), "http.Request");
    assert_eq!(entries[2].source_module_hint(), "http");
    assert_eq!(entries[2].source_symbol(), "HttpRequest");
}

#[test]
fn parses_scalar_function_leaf() {
    let spec = parse("echo: api.echo\n").unwrap();
    let entries = spec.entries().collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].public_path_string(), "echo");
    assert_eq!(entries[0].source_selector.as_dotted(), "api.echo");
}

#[test]
fn parses_public_instance_leaf_with_root_selectors() {
    let spec = parse(
        r#"
managedLlm:
  const: root.llm.managedLlm
  interfaces:
    - root.llm.ManagedLlm
"#,
    )
    .unwrap();

    assert!(spec.entries().next().is_none());
    let public_instances = spec.public_instances().collect::<Vec<_>>();
    assert_eq!(public_instances.len(), 1);
    assert_eq!(public_instances[0].public_path_string(), "managedLlm");
    assert_eq!(public_instances[0].source_module_hint(), "llm");
    assert_eq!(public_instances[0].source_symbol(), "managedLlm");
    assert_eq!(
        public_instances[0].interface_selectors[0].module_path,
        "llm"
    );
    assert_eq!(
        public_instances[0].interface_selectors[0].symbol,
        "ManagedLlm"
    );
}

#[test]
fn rejects_invalid_shapes() {
    for (name, yaml, expected) in [
            ("root-list", "[]", "root must be a mapping"),
            (
                "nested-empty",
                "functions: {}",
                "cannot be an empty mapping",
            ),
            (
                "mixed-nested-empty",
                "unused: {}\nrun: api.run\n",
                "cannot be an empty mapping",
            ),
            ("numeric-key", "1: types.User", "key under <root>"),
            (
                "dotted-key",
                "http.Request: http.HttpRequest",
                "dotted public keys are not supported",
            ),
            ("non-string-leaf", "User: 1", "must map to a string"),
            ("short-selector", "User: User", "module.path.Symbol"),
            ("root-selector", "User: root.types.User", "root. prefix"),
            (
                "legacy-function-marker-only",
                "echo:\n  serviceCall: true\n",
                "must use a scalar string source selector",
            ),
            (
                "legacy-function-source-only",
                "echo:\n  source: api.echo\n",
                "must use a scalar string source selector",
            ),
            (
                "legacy-function-source-and-false-marker",
                "echo:\n  source: api.echo\n  serviceCall: false\n",
                "must use a scalar string source selector",
            ),
            (
                "legacy-function-source-and-marker",
                "echo:\n  source: api.echo\n  serviceCall: true\n  route: /echo\n",
                "must use a scalar string source selector",
            ),
            (
                "instance-missing-interface",
                "managedLlm:\n  const: root.llm.managedLlm\n",
                "missing interfaces",
            ),
            (
                "instance-empty-interfaces",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: []\n",
                "interfaces cannot be empty",
            ),
            (
                "instance-extra-field",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: [root.llm.ManagedLlm]\n  route: /llm\n",
                "unsupported field route",
            ),
            (
                "instance-false-marker",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: [root.llm.ManagedLlm]\n  serviceCall: false\n",
                "unsupported field serviceCall",
            ),
            (
                "instance-duplicate-marker",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: [root.llm.ManagedLlm]\n  serviceCall: true\n  serviceCall: true\n",
                "unsupported field serviceCall",
            ),
        ] {
            let error = parse(yaml).unwrap_err().to_string();
            assert!(
                error.contains(expected),
                "unexpected error for {name}: {error}"
            );
        }
}

#[test]
fn rejects_duplicate_flattened_public_path() {
    let error = parse(
        r#"
User: types.User
User: other.User
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("duplicate api.yml public path User"),
        "unexpected error: {error}"
    );
}
