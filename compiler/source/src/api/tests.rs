use std::path::PathBuf;

use super::*;
use crate::PublicationApiSeed;
use compiler_input_model::{PublicationApiEntry, PublicationApiPublicInstanceEntry};

fn source(path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
    CompilerSourceFile::parse(
        PathBuf::from(path),
        module_path.to_string(),
        false,
        false,
        text.to_string(),
        path,
    )
    .unwrap()
}

#[test]
fn builds_linkage_neutral_publication_api_surface() {
    let spec = PublicationApiSpec::from_entries(vec![
        PublicationApiEntry::for_source("chat.Chat", "chat", "Chat"),
        PublicationApiEntry::for_source("chat.ChatList", "chat", "ChatList"),
        PublicationApiEntry::for_source("chat.Events", "chat", "Events"),
        PublicationApiEntry::for_source("chat.start", "chat", "start"),
        PublicationApiEntry::for_source("chat.VERSION", "chat", "VERSION"),
    ]);
    let sources = [source(
        "chat.skiff",
        "chat",
        r#"
                    type Chat {}
                    alias ChatList = Array<Chat>
                    interface Events { function sent(id: string) -> string }
                    function start() -> string { return "" }
                    const VERSION: string = "1"
                "#,
    )];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

    assert_eq!(api.public_modules["chat"], "chat");
    assert_eq!(
        api.public_symbols["chat.Chat"].source_symbol.as_str(),
        "Chat"
    );
    assert_eq!(
        api.public_symbol_for_source_key(&SourceSymbolKey::new("chat", "Chat")),
        Some("chat.Chat")
    );
    assert!(api.is_public_schema_source_key(&SourceSymbolKey::new("chat", "Chat")));
    assert_eq!(
        api.api_source_modules(),
        BTreeSet::from(["chat".to_string()])
    );
    assert!(api.schema_types.contains_key("chat.ChatList"));
    assert!(api.schema_types.contains_key("chat.Events"));
    assert!(api.callables.contains_key("chat.start"));
    assert!(api.public_symbols.contains_key("chat.VERSION"));
}

#[test]
fn public_nominal_type_does_not_publish_its_impl_methods() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "Client", "client", "Client",
    )]);
    let sources = [source(
        "client.skiff",
        "client",
        r#"
                type Client {}
                impl Client {
                  function send(self: Client, input: string) -> string { return input }
                }
            "#,
    )];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

    assert!(api.public_symbols.contains_key("Client"));
    assert!(api.callables.is_empty());
}

#[test]
fn similarly_named_explicit_functions_remain_distinct_public_operations() {
    let spec = PublicationApiSpec::from_entries(vec![
        PublicationApiEntry::for_source("send", "api", "send"),
        PublicationApiEntry::for_source("admin.send", "api", "adminSend"),
    ]);
    let sources = [source(
        "api.skiff",
        "api",
        r#"
                function send(input: string) -> string { return input }
                function adminSend(input: string) -> string { return input }
            "#,
    )];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

    assert_eq!(api.callables.len(), 2);
    assert_eq!(api.callables["send"].source_symbol, "send");
    assert_eq!(api.callables["admin.send"].source_symbol, "adminSend");
}

#[test]
fn builds_public_instance_seed_from_explicit_api_leaf() {
    let spec = PublicationApiSpec::from_public_instances(vec![
        PublicationApiPublicInstanceEntry::for_source(
            "managedLlm",
            "root.llm.managedLlm",
            ["root.llm.ManagedLlm"],
        )
        .unwrap(),
    ]);
    let sources = [source(
        "llm.skiff",
        "llm",
        r#"
                interface ManagedLlm { function sendChat(input: string) -> string }
                type ManagedLlmImpl implements ManagedLlm {}
                const managedLlm: ManagedLlmImpl = ManagedLlmImpl {}
            "#,
    )];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();
    let seed = PublicationApiSeed::from_publication_api(&api);

    assert!(api.public_symbols.is_empty());
    assert_eq!(api.public_instances.len(), 1);
    let instance = &seed.public_instances["managedLlm"];
    assert_eq!(instance.source_module, "llm");
    assert_eq!(instance.source_symbol, "managedLlm");
    assert_eq!(instance.interfaces[0].source_module, "llm");
    assert_eq!(instance.interfaces[0].source_symbol, "ManagedLlm");
    assert!(seed
        .publication_public_instance_symbols
        .contains(&SourceSymbolKey::new("llm", "managedLlm")));
}

#[test]
fn re_exports_apply_aliases_and_path_prefix() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "Request",
        "types",
        "HttpRequest",
    )]);
    let sources = [source(
        "types.skiff",
        "types",
        r#"
                type HttpRequest {}
            "#,
    )];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

    assert!(api.schema_types.contains_key("Request"));
    assert_eq!(
        api.public_symbols["Request"].source_symbol.as_str(),
        "HttpRequest"
    );
    assert_eq!(
        api.public_symbols["Request"].source_module.as_str(),
        "types"
    );
}

#[test]
fn builds_publication_api_from_resolved_source_modules() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "crypto.hash",
        "crypto",
        "hash",
    )]);
    let sources = [source(
        "crypto.skiff",
        "std.crypto",
        r#"
                function hash() -> string { return "" }
            "#,
    )];

    let api = PublicationApi::build_from_publication_sources_with_resolved_modules(
        &spec,
        sources.iter(),
        |entry| format!("std.{}", entry.source_module_hint()),
    )
    .unwrap();

    assert_eq!(api.public_modules["crypto"], "std.crypto");
    assert_eq!(
        api.public_symbols["crypto.hash"].source_module,
        "std.crypto"
    );
}

#[test]
fn rejects_duplicate_final_public_paths() {
    let spec = PublicationApiSpec::from_entries(vec![
        PublicationApiEntry::for_source("chat.Chat", "model", "Chat"),
        PublicationApiEntry::for_source("chat.Chat", "model", "Chat"),
    ]);
    let sources = [source(
        "model.skiff",
        "model",
        r#"
                type Chat {}
            "#,
    )];

    let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
        .expect_err("same public path must not be re-exported twice")
        .to_string();

    assert!(
        error.contains("duplicate publication api symbol chat.Chat"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_duplicate_selected_source_type_declarations() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "ExampleService",
        "internal.example",
        "ExampleService",
    )]);
    let sources = [source(
        "internal/example.skiff",
        "internal.example",
        r#"
                type ExampleService {}
                type ExampleService {}
            "#,
    )];

    let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
        .expect_err("api.yml selector must not resolve to duplicate source declarations")
        .to_string();

    assert!(
            error.contains("api.yml selector internal.example.ExampleService resolves to multiple source declarations"),
            "unexpected error: {error}"
        );
    assert!(
        error.contains("duplicate publication api schema type ExampleService"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_cross_kind_selected_source_symbol_declarations() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "Collision",
        "model",
        "Collision",
    )]);
    let sources = [source(
        "model.skiff",
        "model",
        r#"
                type Collision {}
                alias Collision = string
            "#,
    )];

    let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
        .expect_err("api.yml selector must not resolve across duplicate source kinds")
        .to_string();

    assert!(
        error.contains("api.yml selector model.Collision resolves to multiple source declarations"),
        "unexpected error: {error}"
    );
}

#[test]
fn allows_same_source_symbol_name_in_different_modules() {
    let spec = PublicationApiSpec::from_entries(vec![
        PublicationApiEntry::for_source("left.Shared", "left", "Shared"),
        PublicationApiEntry::for_source("right.Shared", "right", "Shared"),
    ]);
    let sources = [
        source(
            "left.skiff",
            "left",
            r#"
                    type Shared {}
                "#,
        ),
        source(
            "right.skiff",
            "right",
            r#"
                    type Shared {}
                "#,
        ),
    ];

    let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

    assert_eq!(api.public_symbols["left.Shared"].source_module, "left");
    assert_eq!(api.public_symbols["right.Shared"].source_module, "right");
    assert!(api.schema_types.contains_key("left.Shared"));
    assert!(api.schema_types.contains_key("right.Shared"));
}

#[test]
fn rejects_missing_re_export_target() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "Missing", "types", "Missing",
    )]);
    let sources = [source("api.skiff", "api", "")];

    let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
        .expect_err("re-export target must exist")
        .to_string();

    assert!(
        error.contains("api.yml selector types.Missing not found"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_impl_method_selector() {
    let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
        "run",
        "model.Chat",
        "run",
    )]);
    let sources = [source(
        "model.skiff",
        "model",
        r#"
                type Chat {}
                impl Chat {
                  function run(self: Chat) -> string { return "" }
                }
            "#,
    )];

    let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
        .expect_err("impl methods cannot be api.yml selectors")
        .to_string();

    assert!(
        error.contains("points to an impl method"),
        "unexpected error: {error}"
    );
}
