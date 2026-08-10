mod interner;
mod normalization;
mod substitution;
mod validation;

pub(super) use interner::TypeLinker;
#[cfg(test)]
pub(super) use normalization::normalize_type;
#[cfg(test)]
pub(super) use substitution::substitute_type;
