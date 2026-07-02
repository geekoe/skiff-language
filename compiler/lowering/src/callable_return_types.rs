use std::collections::{BTreeMap, BTreeSet};

use skiff_compiler_source::semantic::impl_method_declaration_name;
use skiff_syntax::{
    ast::{FunctionDecl, InterfaceOperation, SourceFile},
    type_syntax::generic_parts,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallableReturnType {
    pub module_path: String,
    pub return_type: String,
    pub type_params: Vec<String>,
    pub(super) local_type_names: BTreeSet<String>,
}

pub fn extend_callable_return_types_for_source(
    return_types: &mut BTreeMap<String, CallableReturnType>,
    module_path: &str,
    ast: &SourceFile,
) {
    let local_type_names = callable_local_type_names(ast);
    for function in &ast.functions {
        insert_function_callable_return_type(
            return_types,
            module_path,
            &function.name,
            function,
            &[],
            &local_type_names,
        );
    }
    for implementation in &ast.impls {
        let inherited = callable_generic_type_params(&implementation.target);
        for method in &implementation.methods {
            let declaration_name =
                impl_method_declaration_name(&implementation.target, &method.name);
            insert_operation_callable_return_type(
                return_types,
                module_path,
                &declaration_name,
                method,
                &inherited,
                &local_type_names,
            );
            insert_operation_callable_return_type(
                return_types,
                module_path,
                &method.name,
                method,
                &inherited,
                &local_type_names,
            );
        }
        for method in &implementation.method_bodies {
            let declaration_name =
                impl_method_declaration_name(&implementation.target, &method.name);
            insert_function_callable_return_type(
                return_types,
                module_path,
                &declaration_name,
                method,
                &inherited,
                &local_type_names,
            );
            insert_function_callable_return_type(
                return_types,
                module_path,
                &method.name,
                method,
                &inherited,
                &local_type_names,
            );
        }
    }
}

fn callable_local_type_names(ast: &SourceFile) -> BTreeSet<String> {
    ast.types
        .iter()
        .map(|ty| ty.name.clone())
        .chain(ast.aliases.iter().map(|alias| alias.name.clone()))
        .collect()
}

fn insert_function_callable_return_type(
    return_types: &mut BTreeMap<String, CallableReturnType>,
    module_path: &str,
    declaration_name: &str,
    function: &FunctionDecl,
    inherited_type_params: &[String],
    local_type_names: &BTreeSet<String>,
) {
    let type_params = inherited_type_params
        .iter()
        .chain(&function.type_params)
        .cloned()
        .collect::<Vec<_>>();
    insert_callable_return_type(
        return_types,
        module_path,
        declaration_name,
        function.return_type.name.clone(),
        type_params,
        local_type_names,
    );
}

fn insert_operation_callable_return_type(
    return_types: &mut BTreeMap<String, CallableReturnType>,
    module_path: &str,
    declaration_name: &str,
    operation: &InterfaceOperation,
    inherited_type_params: &[String],
    local_type_names: &BTreeSet<String>,
) {
    let type_params = inherited_type_params
        .iter()
        .chain(&operation.type_params)
        .cloned()
        .collect::<Vec<_>>();
    insert_callable_return_type(
        return_types,
        module_path,
        declaration_name,
        operation.return_type.name.clone(),
        type_params,
        local_type_names,
    );
}

fn insert_callable_return_type(
    return_types: &mut BTreeMap<String, CallableReturnType>,
    module_path: &str,
    declaration_name: &str,
    return_type: String,
    type_params: Vec<String>,
    local_type_names: &BTreeSet<String>,
) {
    let signature = CallableReturnType {
        module_path: module_path.to_string(),
        return_type,
        type_params,
        local_type_names: local_type_names.clone(),
    };
    return_types
        .entry(declaration_name.to_string())
        .or_insert(signature.clone());
    return_types
        .entry(format!("{module_path}.{declaration_name}"))
        .or_insert(signature);
}

fn callable_generic_type_params(name: &str) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim())
                .filter(|arg| {
                    !arg.is_empty()
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
