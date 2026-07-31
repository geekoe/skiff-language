use super::*;
use crate::{
    ManifestOwner, ManifestProvenance, PublicationApiPublicInstanceEntry, PublicationApiSpec,
    PublicationManifest,
};
use skiff_compiler_core::id::PublicationId;

#[test]
fn alias_empty_api_root_exposes_symbols_directly_under_alias() {
    let aliases = BTreeMap::from([("llm".to_string(), vec![String::new()])]);
    let resolver = PackageExportResolver::new(&aliases);

    let symbol = resolver
        .resolve_package_symbol_path("llm.chat")
        .expect("alias symbol should resolve");

    assert_eq!(symbol.dependency_ref, "llm");
    assert_eq!(symbol.symbol_path, "chat");
}

#[test]
fn alias_public_path_requires_explicit_public_path_segment() {
    let aliases = BTreeMap::from([("llm".to_string(), vec!["llm".to_string()])]);
    let resolver = PackageExportResolver::new(&aliases);

    let shorthand = resolver
        .resolve_package_symbol_path("llm.chat")
        .expect("alias root should still be recognized");
    let explicit = resolver
        .resolve_package_symbol_path("llm.llm.chat")
        .expect("explicit public path should resolve");

    assert_eq!(shorthand.symbol_path, "chat");
    assert_eq!(explicit.symbol_path, "llm.chat");
}

#[test]
fn default_std_root_keeps_canonical_std_symbol_prefix() {
    let aliases = BTreeMap::new();
    let resolver = PackageExportResolver::new(&aliases);

    let symbol = resolver
        .resolve_package_symbol_path("std.websocket.WebSocketConnectRequest")
        .expect("std symbol should resolve through default root");

    assert_eq!(symbol.dependency_ref, "std");
    assert_eq!(symbol.symbol_path, "std.websocket.WebSocketConnectRequest");
}

#[test]
fn alias_bindings_include_public_instance_roots() {
    let dependency = PackageDependency {
        id: "example.com/llm".to_string(),
        version: "0.1.0".to_string(),
        alias: Some("llm".to_string()),
        top_level_alias: None,
    };
    let manifest = PackageManifest::new(
        PublicationManifest::new(
            PublicationId::parse("example.com/llm").unwrap(),
            "0.1.0".to_string(),
            PublicationApiSpec::from_public_instances(vec![
                PublicationApiPublicInstanceEntry::for_source(
                    "managedLlm",
                    "root.llm.managedLlm",
                    ["root.llm.ManagedLlm"],
                )
                .unwrap(),
            ]),
            Vec::new(),
            ManifestProvenance::synthetic("package.yml", ManifestOwner::UserOrBuiltinPackage),
        ),
        Vec::new(),
    );
    let available = BTreeMap::from([(
        package_manifest_key(&dependency.id, &dependency.version),
        manifest,
    )]);

    let aliases = PackageExportResolver::alias_bindings(&[dependency], &available);
    let resolver = PackageExportResolver::new(&aliases);
    let symbol = resolver
        .resolve_package_symbol_path("llm.managedLlm.sendChat")
        .expect("public instance alias root should resolve");

    assert_eq!(aliases.get("llm"), Some(&vec!["managedLlm".to_string()]));
    assert_eq!(symbol.dependency_ref, "llm");
    assert_eq!(symbol.symbol_path, "managedLlm.sendChat");
}
