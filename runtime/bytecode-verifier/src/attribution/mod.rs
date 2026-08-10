mod source;
mod statements;

pub(crate) use source::{prove_source_attribution, SourceAttributionFacts};
pub(crate) use statements::prove_statement_attribution;
pub use statements::{VerifiedStatementEvent, VerifiedStatementSchedule};
