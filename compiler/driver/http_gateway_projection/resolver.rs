use skiff_artifact_model::{
    OperationCallableKind, PackageArtifact, PackageCallableId, PackageCallableSignature,
    PackageLocalAbiSymbol,
};
use skiff_compiler_input::SourceSymbolSelector;

#[derive(Debug)]
pub(super) struct ResolvedCallable {
    pub selector: String,
    pub callable_id: PackageCallableId,
    pub signature: PackageCallableSignature,
}

pub(super) struct ExactCallableResolver<'a> {
    implementation: &'a PackageArtifact,
}

impl<'a> ExactCallableResolver<'a> {
    pub fn new(implementation: &'a PackageArtifact) -> Self {
        Self { implementation }
    }

    pub fn resolve(&self, raw: &str) -> Result<ResolvedCallable, String> {
        let selector = SourceSymbolSelector::parse(raw)
            .map_err(|message| format!("invalid current-package source selector: {message}"))?;
        let source_path = format!("{}.{}", selector.module_path, selector.symbol);
        if source_path != raw {
            return Err(format!(
                "selector must be the exact canonical source path {source_path}"
            ));
        }
        let symbol = self
            .implementation
            .package_local_abi
            .implementation_symbols
            .get(&source_path)
            .ok_or_else(|| {
                format!("implementationSymbols has no exact top-level callable {source_path}")
            })?;
        let PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } = symbol
        else {
            return Err(format!(
                "implementation symbol {source_path} is not a top-level function"
            ));
        };
        let link = self
            .implementation
            .callable_links
            .get(callable_id)
            .ok_or_else(|| format!("callable {callable_id} has no exact callableLinks entry"))?;
        if &link.callable_id != callable_id {
            return Err(format!(
                "callableLinks key {callable_id} disagrees with nested id {}",
                link.callable_id
            ));
        }
        if link.target.callable_abi_id != callable_id.as_str() {
            return Err(format!(
                "callable {callable_id} target ABI id {} does not match",
                link.target.callable_abi_id
            ));
        }
        if link.target.callable_kind != OperationCallableKind::InternalFunction {
            return Err(format!(
                "callable {callable_id} target kind {:?} is not InternalFunction",
                link.target.callable_kind
            ));
        }
        if link.target.file_ref.module_path != selector.module_path {
            return Err(format!(
                "callable {callable_id} target module {} does not match selector module {}",
                link.target.file_ref.module_path, selector.module_path
            ));
        }
        if !self.implementation.files.iter().any(|file| {
            file.file_ir_identity == link.target.file_ref.file_ir_identity
                && file.module_path == link.target.file_ref.module_path
                && file.source_ast_hash == link.target.file_ref.source_ast_hash
        }) {
            return Err(format!(
                "callable {callable_id} target does not name an exact implementation file"
            ));
        }
        if !self
            .implementation
            .callable_semantic_facts
            .contains_key(callable_id)
        {
            return Err(format!(
                "callable {callable_id} has no exact callableSemanticFacts entry"
            ));
        }
        Ok(ResolvedCallable {
            selector: source_path,
            callable_id: callable_id.clone(),
            signature: signature.clone(),
        })
    }
}
