mod facts;
mod proof;

pub use facts::{VerifiedCallableEffects, VerifiedFunctionEffects};
pub(crate) use proof::prove_effect_and_no_pending;

#[cfg(test)]
mod tests;
