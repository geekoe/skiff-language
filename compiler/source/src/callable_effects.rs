use std::collections::BTreeMap;

use skiff_artifact_model::CallableEffectSummary;

use crate::{
    parsed_sources::ParsedCompilerSource, semantic::impl_method_declaration_name, SourceSymbolKey,
};

/// Owner-local callable effect facts. Source symbols are retained only inside
/// source compilation; the compiled handoff resolves them to executable keys.
#[derive(Debug, Clone, Default)]
pub struct SourceCallableEffectFacts {
    operations: BTreeMap<SourceSymbolKey, CallableEffectSummary>,
}

impl SourceCallableEffectFacts {
    pub fn analysis_pending(parsed_sources: &[ParsedCompilerSource]) -> Self {
        let mut operations = BTreeMap::new();
        for parsed in parsed_sources {
            let module_path = parsed.source().module_path.as_str();
            for function in &parsed.ast().functions {
                operations.insert(
                    SourceSymbolKey::new(module_path, &function.name),
                    CallableEffectSummary::analysis_pending(),
                );
            }
            for implementation in &parsed.ast().impls {
                for method in &implementation.method_bodies {
                    operations.insert(
                        SourceSymbolKey::new(
                            module_path,
                            impl_method_declaration_name(&implementation.target, &method.name),
                        ),
                        CallableEffectSummary::analysis_pending(),
                    );
                }
            }
        }
        Self { operations }
    }

    pub fn operations(&self) -> &BTreeMap<SourceSymbolKey, CallableEffectSummary> {
        &self.operations
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::{parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile};

    use super::*;

    #[test]
    fn every_source_callable_is_explicitly_analysis_pending() {
        let source = CompilerSourceFile::parse(
            PathBuf::from("api.skiff"),
            "api".to_string(),
            true,
            false,
            r#"
                type Worker {}

                impl Worker {
                    function handle(self: Worker) -> void {}
                }

                function run() -> void {}
            "#
            .to_string(),
            "api.skiff",
        )
        .expect("source parses");
        let parsed = parse_publication_sources(Path::new("/tmp/effect-source"), &[source])
            .expect("source facts build");
        let facts = SourceCallableEffectFacts::analysis_pending(&parsed);

        for symbol in ["run", "Worker.handle"] {
            assert_eq!(
                facts.operations().get(&SourceSymbolKey::new("api", symbol)),
                Some(&CallableEffectSummary::analysis_pending())
            );
        }
        assert_eq!(facts.operations().len(), 2);
    }
}
