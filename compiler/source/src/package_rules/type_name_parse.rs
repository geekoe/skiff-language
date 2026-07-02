pub(super) fn qualified_package_std_type_parts(ty: &str) -> Option<(&str, &str)> {
    let rest = ty.strip_prefix("std.")?;
    let (root, bare) = rest.split_once('.')?;
    if root.is_empty() || bare.is_empty() || bare.contains('.') {
        return None;
    }
    Some((root, bare))
}
