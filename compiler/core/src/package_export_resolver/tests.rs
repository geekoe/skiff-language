use super::*;

#[test]
fn resolves_dependency_source_type_path_without_rewriting_it_as_public() {
    let roots = BTreeMap::from([("widget".to_string(), vec!["api".to_string()])]);
    let resolved = PackageExportResolver::new(&roots)
        .resolve_package_symbol_path("widget/internal.codec.Private")
        .unwrap();
    assert_eq!(resolved.dependency_ref, "widget");
    assert_eq!(resolved.symbol_path, "internal.codec.Private");
}
