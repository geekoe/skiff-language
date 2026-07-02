pub fn is_valid_dotted_module_path(path: &str) -> bool {
    !path.is_empty() && path.split('.').all(is_valid_module_segment)
}

fn is_valid_module_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn package_public_path(package_id: &str, export_path: &str) -> String {
    crate::package_export_resolver::package_public_path(package_id, export_path)
}

#[allow(dead_code)]
pub fn public_symbol_path(prefix: &str, symbol: &str) -> String {
    if prefix.is_empty() {
        symbol.to_string()
    } else {
        format!("{prefix}.{symbol}")
    }
}
