use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::{
        ast::{Expr, FunctionDecl, InterfaceOperation, Pattern, TypeRef},
        ast_utils::{walk_expr, walk_pattern, AstVisitor},
        type_expr::TypeExpr,
    },
    SourceDependencyAnalysisInput,
};

pub(crate) fn validate_contract_type_uses(
    parsed_sources: &[ParsedCompilerSource],
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<(), String> {
    let mut validator = ContractTypeUseValidator {
        dependency_analysis,
        violations: BTreeSet::new(),
    };
    for parsed in parsed_sources {
        let ast = parsed.ast();
        for ty in &ast.types {
            if let Some(alias) = &ty.alias {
                validator.visit_type_ref(alias);
            }
            for implemented in &ty.implements {
                validator.visit_type_ref(implemented);
            }
            for field in &ty.fields {
                validator.visit_type_ref(&field.ty);
            }
        }
        for alias in &ast.aliases {
            validator.visit_type_ref(&alias.target_type);
        }
        for interface in &ast.interfaces {
            for operation in &interface.operations {
                validator.visit_operation(operation);
            }
        }
        for implementation in &ast.impls {
            validator.validate_type_text(&implementation.target);
            for method in &implementation.methods {
                validator.visit_operation(method);
            }
            for method in &implementation.method_bodies {
                validator.visit_function(method);
            }
        }
        for function in &ast.functions {
            validator.visit_function(function);
        }
        for signature in &ast.function_signatures {
            validator.visit_operation(signature);
        }
        for constant in &ast.consts {
            if let Some(ty) = &constant.ty {
                validator.visit_type_ref(ty);
            }
            validator.visit_expr(&constant.value);
        }
        for db in &ast.dbs {
            for index in &db.indexes {
                if let Some(predicate) = &index.where_expr {
                    validator.visit_expr(predicate);
                }
            }
        }
        for test in &ast.tests {
            validator.visit_block(&test.body);
        }
    }
    if validator.violations.is_empty() {
        return Ok(());
    }
    Err(validator
        .violations
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n- "))
}

struct ContractTypeUseValidator<'a> {
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    violations: BTreeSet<String>,
}

impl ContractTypeUseValidator<'_> {
    fn visit_function(&mut self, function: &FunctionDecl) {
        for parameter in &function.params {
            self.visit_type_ref(&parameter.ty);
        }
        self.visit_type_ref(&function.return_type);
        if let Some(receiver) = &function.implicit_self {
            self.visit_type_ref(receiver);
        }
        self.visit_block(&function.body);
    }

    fn visit_operation(&mut self, operation: &InterfaceOperation) {
        for parameter in &operation.params {
            self.visit_type_ref(&parameter.ty);
        }
        self.visit_type_ref(&operation.return_type);
        if let Some(receiver) = &operation.implicit_self {
            self.visit_type_ref(receiver);
        }
    }

    fn validate_type_text(&mut self, text: &str) {
        self.validate_type_expr(&TypeExpr::parse(text));
    }

    fn validate_type_expr(&mut self, expr: &TypeExpr) {
        match expr {
            TypeExpr::Named { name, args } => {
                if let Some((alias, stable_key)) = name.split_once('.') {
                    if self.dependency_analysis.contract_requirement(alias).is_ok() {
                        if stable_key.is_empty() {
                            self.violations.insert(format!(
                                "contract dependency type `{name}` has no stable type key"
                            ));
                        } else if let Err(error) = self
                            .dependency_analysis
                            .public_contract_type_id_by_stable_key(alias, stable_key)
                        {
                            self.violations.insert(error.to_string());
                        }
                    }
                }
                for argument in args {
                    self.validate_type_expr(argument);
                }
            }
            TypeExpr::Nullable(inner) | TypeExpr::AnyInterface { interface: inner } => {
                self.validate_type_expr(inner);
            }
            TypeExpr::Union(items) => {
                for item in items {
                    self.validate_type_expr(item);
                }
            }
            TypeExpr::Record(fields) => {
                for field in fields {
                    self.validate_type_expr(&field.ty);
                }
            }
            TypeExpr::Function {
                params,
                return_type,
            } => {
                for parameter in params {
                    self.validate_type_expr(&parameter.ty);
                }
                self.validate_type_expr(return_type);
            }
            TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => {}
        }
    }
}

impl AstVisitor for ContractTypeUseValidator<'_> {
    fn visit_pattern(&mut self, pattern: &Pattern) {
        if let Pattern::Nominal { name, .. } = pattern {
            self.validate_type_text(name);
        }
        walk_pattern(self, pattern);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let Expr::Record { type_name, .. } = expr {
            self.validate_type_text(type_name);
        }
        walk_expr(self, expr);
    }

    fn visit_type_ref(&mut self, ty: &TypeRef) {
        self.validate_type_text(&ty.name);
    }
}
use std::collections::BTreeSet;
