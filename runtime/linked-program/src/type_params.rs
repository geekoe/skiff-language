use crate::linked::{LinkedExecutable, LinkedTypeRef};

pub fn executable_type_param_names(executable: &LinkedExecutable) -> Vec<String> {
    let mut names = Vec::new();
    for name in &executable.type_params {
        push_unique_type_param(&mut names, name);
    }
    for param in &executable.params {
        collect_type_ref_type_params(&param.ty, &mut names);
    }
    if let Some(ty) = &executable.return_type {
        collect_type_ref_type_params(ty, &mut names);
    }
    if let Some(ty) = &executable.self_type {
        collect_type_ref_type_params(ty, &mut names);
    }
    names
}

fn collect_type_ref_type_params(type_ref: &LinkedTypeRef, names: &mut Vec<String>) {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => push_unique_type_param(names, name),
        LinkedTypeRef::Native { args, .. } => {
            for arg in args {
                collect_type_ref_type_params(arg, names);
            }
        }
        LinkedTypeRef::AppliedNominal { arguments, .. } => {
            for argument in arguments {
                collect_type_ref_type_params(argument, names);
            }
        }
        LinkedTypeRef::Record { fields } => {
            for field in fields.values() {
                collect_type_ref_type_params(field, names);
            }
        }
        LinkedTypeRef::Union { items } => {
            for item in items {
                collect_type_ref_type_params(item, names);
            }
        }
        LinkedTypeRef::Nullable { inner } => collect_type_ref_type_params(inner, names),
        LinkedTypeRef::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_type_params(arg, names);
            }
        }
        LinkedTypeRef::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_type_ref_type_params(&param.ty, names);
            }
            collect_type_ref_type_params(return_type, names);
        }
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => {}
    }
}

fn push_unique_type_param(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|item| item == name) {
        names.push(name.to_string());
    }
}

#[cfg(test)]
mod tests;
