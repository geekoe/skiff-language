use skiff_compiler_projection_input::ProjectionSourceSymbolKey;

pub fn projection_source_symbol_text(source: &ProjectionSourceSymbolKey) -> String {
    format!("{}.{}", source.module_path(), source.symbol())
}
