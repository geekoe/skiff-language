mod common;
use common::artifacts::{
    assert_publish_error_contains, assert_service_package_id, build_temp_service_publication,
    package_assembly, service_assembly_value, service_package,
};
use skiff_compiler::test_support::project_fixtures::{
    write_package_api_yml, write_package_manifest, write_package_source, ServiceProjectBuilder,
};

#[test]
fn connect_mongo_dotted_import_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "connect-mongo-import",
        "import connect.mongo",
        "return {}",
    );

    assert_publish_error_contains(
        temp.root(),
        &["import name must be a single ASCII identifier"],
    );
}

#[test]
fn std_root_projection_without_import_mongo_target_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "disallow-std-mongo-root-expression",
        "",
        r#"
          const _std_target = std.mongo.Target("cluster-a", "app")
          return {}
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "std.mongo is not permitted as a std module root",
            "allowed std module roots are",
        ],
    );
}

#[test]
fn connect_mongo_target_call_is_rejected_as_removed_source_surface() {
    let temp = ServiceProjectBuilder::package_model(
        "connect-mongo-target-removed",
        "import connect",
        r#"
          const db = connect.mongo.Target("cluster-a", "app")
          const users = db.Collection<User>("user")
          return users.findOne({ id: "u1" })
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["connect.mongo provider wrapper has been removed"],
    );
}

#[test]
fn service_source_cannot_call_internal_provider_primitives_directly() {
    let temp = ServiceProjectBuilder::package_model(
        "direct-provider-primitive",
        "",
        r#"
          return __providerCallFindOne({}, { id: "u1" })
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["internal provider-call primitive", "__providerCallFindOne"],
    );
}

#[test]
fn ordinary_package_sources_cannot_hide_connect_provider_calls() {
    let temp = ServiceProjectBuilder::package_model(
        "ordinary-package-connect-use",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/repo", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/repo",
        r#"
id: example.com/repo
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/repo",
        r#"
loadUser: repo.loadUser
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/repo",
        "repo.skiff",
        r#"
          import connect

          function loadUser() -> {} {
            const db = connect.mongo.Target("cluster-a", "app")
            const users = db.Collection<{}>("user")
            return users.findOne({})
          }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "package example.com/repo",
            "connect.mongo provider wrapper has been removed",
        ],
    );
}

#[test]
fn package_source_cannot_call_internal_provider_primitives_directly() {
    let temp = ServiceProjectBuilder::package_model(
        "ordinary-package-direct-provider-primitive",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/repo", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/repo",
        r#"
id: example.com/repo
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/repo",
        r#"
loadUser: repo.loadUser
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/repo",
        "repo.skiff",
        r#"
          function loadUser() -> {} {
            return __providerCallFindOne({}, {})
          }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &[
            "package example.com/repo",
            "internal provider-call primitive __providerCallFindOne",
        ],
    );
}

#[test]
fn package_sources_cannot_shadow_std_or_connect_roots() {
    let temp =
        ServiceProjectBuilder::package_model("package-reserved-root", "import app", "return {}");
    temp.add_service_package_dependency("example.com/bad", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/bad",
        r#"
id: example.com/bad
version: 0.1.0
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/bad",
        r#"
run: bad.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/bad",
        "bad.skiff",
        r#"
          type connect {}

          function run() -> {} {
            const std = {}
            return std
          }
        "#,
    );

    assert_publish_error_contains(
        temp.root(),
        &["package example.com/bad", "reserved prelude name"],
    );
}

#[test]
fn ordinary_package_import_is_recorded_without_provider_requirements() {
    let temp = ServiceProjectBuilder::package_model("ordinary-package", "import app", "return {}");
    temp.add_service_package_dependency("example.com/chat", Some("app"));
    temp.packages().add_package(
        "example.com/chat",
        r#"
id: example.com/chat
version: 0.1.0
"#,
        &[(
            "chat_impl.skiff",
            r#"
          function run() -> string {
            return "ok"
          }
        "#,
        )],
    );
    write_package_api_yml(
        temp.root(),
        "example.com/chat",
        r#"
run: chat_impl.run
"#,
    );

    let published = build_temp_service_publication(temp.root());
    assert_service_package_id(&published, "example.com/chat");
    assert_eq!(
        service_package(&published, "example.com/chat")["version"],
        "0.1.0"
    );
    assert!(service_assembly_value(&published)
        .get("providerRequirements")
        .is_none());
    assert!(service_package(&published, "example.com/chat")
        .get("providerRequirements")
        .is_none());
}

#[test]
fn package_and_service_assemblies_omit_removed_metadata_fields() {
    let temp =
        ServiceProjectBuilder::package_model("removed-package-metadata", "import app", "return {}");
    temp.add_service_package_dependency("example.com/search", Some("app"));
    temp.packages().add_package(
        "example.com/search",
        r#"
id: example.com/search
version: 0.1.0
"#,
        &[(
            "search_impl.skiff",
            r#"
          function query() -> string { return "ok" }
          function write() -> string { return "ok" }
        "#,
        )],
    );
    write_package_api_yml(
        temp.root(),
        "example.com/search",
        r#"
query: search_impl.query
write: search_impl.write
"#,
    );

    let published = build_temp_service_publication(temp.root());
    let service_assembly = service_assembly_value(&published);
    let service_package = service_package(&published, "example.com/search");
    let package_assembly = &package_assembly(&published, "example.com/search").value;

    assert!(service_assembly.get("providerRequirements").is_none());
    assert!(service_assembly.get("transportSelection").is_none());
    assert!(service_assembly.get("effectSummaries").is_none());
    assert!(service_package.get("providerRequirements").is_none());
    assert!(service_package.get("transports").is_none());
    assert!(package_assembly.get("providerRequirements").is_none());
    assert!(package_assembly.get("transports").is_none());
    assert!(package_assembly.get("publicEffects").is_none());
}

#[test]
fn package_providers_field_is_rejected() {
    let temp =
        ServiceProjectBuilder::package_model("provider-field-removed", "import app", "return {}");
    temp.add_service_package_dependency("example.com/queue", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/queue",
        r#"
id: example.com/queue
version: 0.1.0
providers:
  - capability: example.com/queue/v1
    transports: [legacy]
    targets:
      - example.com/queue.publish
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/queue",
        r#"
publish: queue.publish
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/queue",
        "queue.skiff",
        r#"
          function publish() -> string { return "ok" }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["unknown field `providers`"]);
}

#[test]
fn package_transports_field_is_rejected() {
    let temp =
        ServiceProjectBuilder::package_model("transport-field-removed", "import app", "return {}");
    temp.add_service_package_dependency("example.com/queue", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/queue",
        r#"
id: example.com/queue
version: 0.1.0
transports: [legacy]
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/queue",
        r#"
publish: queue.publish
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/queue",
        "queue.skiff",
        r#"
          function publish() -> string { return "ok" }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["unknown field `transports`"]);
}

#[test]
fn service_assembly_omits_removed_effect_summaries() {
    let temp = ServiceProjectBuilder::package_model(
        "builtin-connect-no-effect-summaries",
        "",
        "return {}",
    );
    let published = build_temp_service_publication(temp.root());
    assert!(published
        .artifacts
        .service_assembly
        .value
        .get("effectSummaries")
        .is_none());
}

#[test]
fn package_public_effects_field_is_rejected() {
    let temp = ServiceProjectBuilder::package_model(
        "public-effects-field-removed",
        "import app",
        "return {}",
    );
    temp.add_service_package_dependency("example.com/effects", Some("app"));
    write_package_manifest(
        temp.root(),
        "example.com/effects",
        r#"
id: example.com/effects
version: 0.1.0
publicEffects:
  example.com/effects.outer:
    target: example.com/effects.outer
    nestedEffects:
      - target: example.com/effects.middle
        source: explicit
  example.com/effects.middle:
    target: example.com/effects.middle
    nestedEffects:
      - target: example.com/effects.inner
        source: explicit
  example.com/effects.inner:
    target: example.com/effects.inner
    effect: external.read
"#,
    );
    write_package_api_yml(
        temp.root(),
        "example.com/effects",
        r#"
run: effects.run
"#,
    );
    write_package_source(
        temp.root(),
        "example.com/effects",
        "effects.skiff",
        r#"
          function run() -> string {
            return "ok"
          }
        "#,
    );

    assert_publish_error_contains(temp.root(), &["unknown field `publicEffects`"]);
}

#[test]
fn reserved_std_and_connect_names_are_rejected_for_declarations_and_local_bindings() {
    let temp = ServiceProjectBuilder::package_model("redeclare-connect", "", "return {}");
    temp.add_source(
        "internal/redeclare.skiff",
        r#"
            type connect {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["type connect", "reserved prelude name"]);

    let temp = ServiceProjectBuilder::package_model(
        "local-std",
        "",
        r#"
            const std = 1
            return {}
        "#,
    );
    assert_publish_error_contains(temp.root(), &["local binding std", "reserved prelude name"]);

    let temp = ServiceProjectBuilder::package_model(
        "pattern-connect",
        "",
        r#"
            match input {
              connect => {
                return {}
              }
            }
            return {}
        "#,
    );
    assert_publish_error_contains(
        temp.root(),
        &["pattern binding connect", "reserved prelude name"],
    );
}
