/// Resolves the exact synchronous static intrinsic owned by the bytecode
/// compiler. Admission and emission share this table so a native call cannot
/// be admitted under one authority and encoded under another.
pub(super) fn static_intrinsic_canonical_key(target: &str) -> Option<&'static str> {
    match target {
        "Array.empty" | "core.array.empty" => Some("core.array.empty"),
        "Map.empty" | "core.map.empty" => Some("core.map.empty"),
        "core.bytes.fromUtf8" => Some("core.bytes.fromUtf8"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::static_intrinsic_canonical_key;

    #[test]
    fn bytes_from_utf8_requires_its_canonical_native_binding() {
        assert_eq!(
            static_intrinsic_canonical_key("core.bytes.fromUtf8"),
            Some("core.bytes.fromUtf8")
        );
        for unowned in ["std.bytes.fromUtf8", "bytes.fromUtf8", "core.bytes.fromHex"] {
            assert_eq!(static_intrinsic_canonical_key(unowned), None);
        }
    }
}
