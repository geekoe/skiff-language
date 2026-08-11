use crate::RuntimeDiagnostic;

mod sealed {
    pub trait Sealed {}
}

/// Marker for diagnostic errors that also carry a generated Skiff projection.
///
/// This capability is sealed: external error types cannot opt themselves into
/// projection merely by choosing a diagnostic code.
///
/// ```compile_fail
/// use std::{borrow::Cow, fmt};
/// use skiff_runtime_request_contract::{
///     DiagnosticCode, ProjectableDiagnostic, RuntimeDiagnostic,
/// };
///
/// #[derive(Debug)]
/// struct ExternalError;
///
/// impl fmt::Display for ExternalError {
///     fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
///         formatter.write_str("external error")
///     }
/// }
///
/// impl std::error::Error for ExternalError {}
///
/// impl RuntimeDiagnostic for ExternalError {
///     fn diagnostic_code(&self) -> DiagnosticCode {
///         DiagnosticCode::new("external.error").expect("valid code")
///     }
///
///     fn diagnostic_message(&self) -> Cow<'_, str> {
///         Cow::Borrowed("external error")
///     }
/// }
///
/// impl ProjectableDiagnostic for ExternalError {}
/// ```
///
/// A diagnostic-only error also cannot satisfy a projectable bound:
///
/// ```compile_fail
/// use std::{borrow::Cow, fmt};
/// use skiff_runtime_request_contract::{
///     DiagnosticCode, ProjectableDiagnostic, RuntimeDiagnostic,
/// };
///
/// #[derive(Debug)]
/// struct DiagnosticOnly;
///
/// impl fmt::Display for DiagnosticOnly {
///     fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
///         formatter.write_str("diagnostic only")
///     }
/// }
///
/// impl std::error::Error for DiagnosticOnly {}
///
/// impl RuntimeDiagnostic for DiagnosticOnly {
///     fn diagnostic_code(&self) -> DiagnosticCode {
///         DiagnosticCode::new("diagnostic.only").expect("valid code")
///     }
///
///     fn diagnostic_message(&self) -> Cow<'_, str> {
///         Cow::Borrowed("diagnostic only")
///     }
/// }
///
/// fn needs_projection(_error: &dyn ProjectableDiagnostic) {}
///
/// needs_projection(&DiagnosticOnly);
/// ```
pub trait ProjectableDiagnostic: RuntimeDiagnostic + sealed::Sealed {}
