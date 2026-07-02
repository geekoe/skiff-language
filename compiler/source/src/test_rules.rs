use crate::{shared::ast::SourceFile, shared::error::CompileError};

pub fn validate_no_test_declarations_in_production_source(
    path: &str,
    ast: &SourceFile,
) -> Result<(), CompileError> {
    let mut violations = Vec::new();
    if !ast.tests.is_empty() {
        violations.push("production source cannot contain test declarations".to_string());
    }
    if ast.test_default_run.is_some() {
        violations.push("production source cannot contain test defaultRun directive".to_string());
    }
    if violations.is_empty() {
        return Ok(());
    }
    Err(CompileError::Semantic(format!(
        "{} in {path}",
        violations.join("; ")
    )))
}
