use crate::error::RuntimeError;

#[derive(Debug)]
pub enum EvalStreamExecutionError<E> {
    Eval(RuntimeError),
    Callback(E),
}

pub type EvalStreamResult<T, E> = std::result::Result<T, EvalStreamExecutionError<E>>;

impl<E> EvalStreamExecutionError<EvalStreamExecutionError<E>> {
    pub fn flatten(self) -> EvalStreamExecutionError<E> {
        match self {
            EvalStreamExecutionError::Eval(error) => EvalStreamExecutionError::Eval(error),
            EvalStreamExecutionError::Callback(EvalStreamExecutionError::Eval(error)) => {
                EvalStreamExecutionError::Eval(error)
            }
            EvalStreamExecutionError::Callback(EvalStreamExecutionError::Callback(error)) => {
                EvalStreamExecutionError::Callback(error)
            }
        }
    }
}

pub fn map_eval_error<T, E, SourceError>(
    result: std::result::Result<T, SourceError>,
) -> EvalStreamResult<T, E>
where
    SourceError: Into<RuntimeError>,
{
    result.map_err(|error| EvalStreamExecutionError::Eval(error.into()))
}

pub fn map_callback_error<T, E>(result: std::result::Result<T, E>) -> EvalStreamResult<T, E> {
    result.map_err(EvalStreamExecutionError::Callback)
}
