use skiff_compiler_core::source_role::PublicationSourceRole;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPublicationSource {
    pub source_path: String,
    pub module_path: String,
    pub role: PublicationSourceRole,
    pub source_ast_hash: Option<String>,
}
