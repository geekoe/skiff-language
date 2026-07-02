use super::*;
use skiff_runtime_eval::error::RuntimeErrorPayload;
#[cfg(test)]
use skiff_runtime_eval::error::WirePayload;

pub(super) trait IntoEvalResult<T> {
    fn into_eval_result(self) -> Result<T>;
}

impl<T> IntoEvalResult<T> for root_error::Result<T> {
    fn into_eval_result(self) -> Result<T> {
        self.map_err(root_error_into_eval)
    }
}

pub(crate) fn root_error_into_eval(error: root_error::RuntimeError) -> RuntimeError {
    match error {
        root_error::RuntimeError::ExternalErrorPayload {
            code,
            message,
            status,
            details,
        } => RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code,
            message,
            status,
            details,
        }),
        root_error::RuntimeError::Diagnosed(diagnosed) => root_diagnosed_into_eval(diagnosed),
        root_error::RuntimeError::Opaque(error) => RuntimeError::from_wire_payload(error),
        error => RuntimeError::Opaque(Box::new(error)),
    }
}

fn root_diagnosed_into_eval(diagnosed: root_error::Diagnosed) -> RuntimeError {
    match diagnosed.try_into_runtime_parts() {
        Ok((inner, frames)) => {
            let mut error = root_error_into_eval(inner);
            for frame in frames.into_iter().rev() {
                error = match frame {
                    root_error::DiagnosticFrame::Source { source_id, frame } => {
                        error.with_source(source_id, *frame)
                    }
                    root_error::DiagnosticFrame::Diagnostic { frame } => {
                        error.with_diagnostic_frame(*frame)
                    }
                };
            }
            error
        }
        Err(diagnosed) => RuntimeError::Opaque(Box::new(diagnosed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use skiff_runtime_eval::error::TypeIdentity;

    #[test]
    fn diagnosed_root_error_replays_eval_diagnostic_wrappers() {
        let source_frame = json!({
            "assemblyId": 1,
            "sourceId": 7,
            "source": { "path": "package/main.skiff" }
        });
        let diagnostic_frame = json!({
            "operation": "request.dispatch",
            "sourceId": 7
        });
        let error = root_error::RuntimeError::Opaque(Box::new(
            skiff_runtime_boundary::error::RuntimeError::db_decode(
                "std.db",
                "db value missing key field id",
            ),
        ))
        .with_diagnostic_frame(diagnostic_frame.clone())
        .with_source(7, source_frame.clone());

        let eval_error = root_error_into_eval(error);

        match &eval_error {
            RuntimeError::WithDiagnosticFrame { frame, error } => {
                assert_eq!(**frame, diagnostic_frame);
                match error.as_ref() {
                    RuntimeError::WithSource {
                        source_id,
                        frame,
                        error,
                    } => {
                        assert_eq!(*source_id, 7);
                        assert_eq!(**frame, source_frame);
                        assert!(matches!(error.as_ref(), RuntimeError::Opaque(_)));
                        let payload = error.payload();
                        assert_eq!(payload.code, "std.db.DecodeError");
                        assert_eq!(payload.message, "db value missing key field id");
                        assert_eq!(
                            WirePayload::catch_projection(error.as_ref()),
                            Some((
                                TypeIdentity::builtin("std.db.DecodeError"),
                                json!({
                                    "target": "std.db",
                                    "message": "db value missing key field id",
                                }),
                            ))
                        );
                    }
                    error => panic!("expected inner eval source wrapper, got {error:?}"),
                }
            }
            error => panic!("expected outer eval diagnostic wrapper, got {error:?}"),
        }

        let source = eval_error
            .diagnostic_source()
            .expect("replayed source frame should provide diagnostic source");
        assert_eq!(source.assembly_id, Some(1));
        assert_eq!(source.source_id, 7);

        let payload = eval_error.payload();
        let details = payload.details.expect("diagnostic details should exist");
        assert_eq!(details["frames"][0], diagnostic_frame);
        assert_eq!(details["frames"][1], source_frame);
    }

    #[test]
    fn root_opaque_capability_error_enters_eval_as_catchable_opaque() {
        let capability_error =
            skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
                target: "svc.account".to_string(),
            };
        let expected_payload = capability_error.payload();
        let expected_catch_projection = capability_error.catch_projection();
        let root_error = root_error::RuntimeError::from(capability_error);
        assert!(matches!(root_error, root_error::RuntimeError::Opaque(_)));

        let eval_error = root_error_into_eval(root_error);

        assert!(matches!(eval_error, RuntimeError::Opaque(_)));
        assert_eq!(eval_error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&eval_error),
            expected_catch_projection
        );
    }

    #[test]
    fn root_standard_error_enters_eval_as_catchable_opaque() {
        let root_error = root_error::RuntimeError::file_error("std.file not found");

        let eval_error = root_error_into_eval(root_error);

        assert!(matches!(eval_error, RuntimeError::Opaque(_)));
        assert_eq!(
            WirePayload::catch_projection(&eval_error),
            Some((
                TypeIdentity::builtin("std.file.FileError"),
                json!({ "message": "std.file not found" }),
            ))
        );
    }
}
