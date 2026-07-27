mod create;
mod read;
mod replace;
mod store;
mod update;

use skiff_runtime_capability_context::DbCapabilityError;

use crate::ServiceDbError;

fn capability_error(error: ServiceDbError) -> DbCapabilityError {
    DbCapabilityError::opaque(error)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreparedRuntimeTestKind {
    FindOneByKey,
    FindOneByQuery,
    FindMany,
    Create,
    Update,
    Replace,
}

#[cfg(test)]
pub(crate) enum PreparedRuntimeTestOutcome {
    Optional(Option<mongodb::bson::Document>),
    Many(Vec<mongodb::bson::Document>),
    Create,
}

#[cfg(test)]
pub(crate) type PreparedRuntimeTestWait = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = crate::Result<PreparedRuntimeTestOutcome>>
            + Send
            + 'static,
    >,
>;

#[cfg(test)]
pub(crate) trait PreparedRuntimeTestDriver: Send + Sync {
    fn wait(&self, kind: PreparedRuntimeTestKind) -> PreparedRuntimeTestWait;
}
