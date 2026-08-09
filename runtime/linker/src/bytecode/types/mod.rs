mod interner;
mod substitution;
mod validation;

pub(super) use interner::TypeLinker;
#[cfg(test)]
pub(super) use substitution::substitute_type;
