use std::path::{Path, PathBuf};

use crate::{parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile};

use super::*;

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
fn resolves_qualified_type_key_for_dotted_module_paths() {
    let sources = [
        source(
            "domain/shared.skiff",
            "domain.shared",
            "type Address { street: string }\n",
        ),
        source(
            "domain/orders.skiff",
            "domain.orders",
            "type Order { shipping: domain.shared.Address }\n",
        ),
    ];
    let parsed_sources = parse_publication_sources(Path::new("."), &sources).unwrap();
    let type_index = ServiceTypeIndex::build(&parsed_sources);

    let qualified = type_index
        .resolve_from_module("domain.orders", "domain.shared.Address")
        .unwrap();
    assert_eq!(
        qualified.source_key,
        SourceSymbolKey::new("domain.shared", "Address")
    );
    assert!(type_index
        .resolve_from_module("domain.orders", "Address")
        .is_none());
}
