use serde_json::json;
use skiff_runtime_linked_program::ExecutableAddr;

use crate::error::{attach_diagnostic_frame, RuntimeError};

use super::EvalRuntimeProgram;

impl EvalRuntimeProgram {
    pub fn attach_request_diagnostic_frame(
        &self,
        error: RuntimeError,
        operation: &str,
        target: &str,
        build_id: &str,
        addr: &ExecutableAddr,
    ) -> RuntimeError {
        match self.projection().resolve_executable(addr) {
            Ok(resolved) => attach_diagnostic_frame(
                error,
                json!({
                    "sourceId": null,
                    "operation": operation,
                    "target": target,
                    "buildId": build_id,
                    "runtimeProgram": true,
                    "unit": addr.unit.to_string(),
                    "file": addr.file.to_string(),
                    "executable": addr.executable,
                    "fileIrIdentity": resolved.file.file_ir_identity.as_str(),
                    "modulePath": resolved.file.module_path.as_str(),
                    "symbol": resolved.executable.symbol.as_str(),
                }),
            ),
            Err(resolve_error) => attach_diagnostic_frame(
                error,
                json!({
                    "sourceId": null,
                    "operation": operation,
                    "target": target,
                    "buildId": build_id,
                    "runtimeProgram": true,
                    "unit": addr.unit.to_string(),
                    "file": addr.file.to_string(),
                    "executable": addr.executable,
                    "diagnosticFrameError": resolve_error.to_string(),
                }),
            ),
        }
    }
}
