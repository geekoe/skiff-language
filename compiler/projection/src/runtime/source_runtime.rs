use crate::{error::CompileError, error::ProjectionError};

pub fn compile_error_to_publication_error(error: CompileError) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: error.to_string(),
    }
}
