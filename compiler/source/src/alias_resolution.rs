use std::collections::{BTreeMap, BTreeSet};

use crate::{
    shared::ast::{
        AliasDecl, Expr, InterfaceDecl, InterfaceOperation, Pattern, SourceFile, TypeDecl, TypeRef,
    },
    shared::ast_utils::{walk_expr_mut, walk_pattern_mut, AstVisitorMut},
    shared::type_expr::{FunctionTypeParam, RecordTypeField, TypeExpr},
    shared::type_syntax::{generic_parts, split_top_level, string_literal},
};

pub fn collect_source_alias_violations(path: &str, ast: &SourceFile, violations: &mut Vec<String>) {
    let mut ast = ast.clone();
    validate_and_expand_source_aliases(path, &mut ast, violations);
}

pub fn validate_and_expand_source_aliases(
    path: &str,
    ast: &mut SourceFile,
    violations: &mut Vec<String>,
) {
    let aliases = source_aliases(ast);
    validate_and_expand_source_aliases_with_targets(path, ast, &aliases, violations);
}

pub fn validate_and_expand_source_aliases_with_targets(
    path: &str,
    ast: &mut SourceFile,
    aliases: &BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    validate_and_expand_aliases(path, aliases, violations, |resolver| {
        resolver.expand_source_file(ast);
    });
}

pub fn qualify_alias_type_name(
    raw: &str,
    local_aliases: &BTreeSet<String>,
    qualified_aliases: &BTreeSet<String>,
    qualify_local_alias: &impl Fn(&str) -> String,
) -> String {
    qualify_alias_type_name_seen(raw, local_aliases, qualified_aliases, qualify_local_alias)
}

pub fn validate_and_expand_contract_aliases_with_targets(
    context: &str,
    types: &mut BTreeMap<String, TypeDecl>,
    aliases: &mut BTreeMap<String, AliasDecl>,
    interfaces: &mut [InterfaceDecl],
    alias_targets: &BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    validate_and_expand_aliases(context, alias_targets, violations, |resolver| {
        for ty in types.values_mut() {
            resolver.expand_type_decl(ty);
        }
        for alias in aliases.values_mut() {
            resolver.expand_type_ref(&mut alias.target_type);
        }
        for interface in interfaces {
            resolver.expand_interface(interface);
        }
    });
}

pub fn collect_alias_cycle_violations(
    context: &str,
    aliases: &BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    let mut state = BTreeMap::<String, VisitState>::new();
    let mut stack = Vec::new();
    let mut reported = BTreeSet::new();
    for name in aliases.keys() {
        visit_alias(
            context,
            name,
            aliases,
            &mut state,
            &mut stack,
            &mut reported,
            violations,
        );
    }
}

fn validate_and_expand_aliases(
    context: &str,
    aliases: &BTreeMap<String, String>,
    violations: &mut Vec<String>,
    expand: impl FnOnce(&mut AliasResolver<'_, '_>),
) {
    let initial_violation_count = violations.len();
    collect_alias_cycle_violations(context, aliases, violations);
    if violations.len() != initial_violation_count {
        return;
    }

    let mut resolver = AliasResolver::new(context, aliases, violations);
    expand(&mut resolver);
}

fn source_aliases(ast: &SourceFile) -> BTreeMap<String, String> {
    ast.aliases
        .iter()
        .map(|alias| (alias.name.clone(), alias.target_type.name.clone()))
        .collect()
}

fn qualify_alias_type_name_seen(
    raw: &str,
    local_aliases: &BTreeSet<String>,
    qualified_aliases: &BTreeSet<String>,
    qualify_local_alias: &impl Fn(&str) -> String,
) -> String {
    TypeExpr::parse_lossy(raw)
        .map_named_types(|name| {
            if local_aliases.contains(name) {
                qualify_local_alias(name)
            } else if qualified_aliases.contains(name) {
                name.to_string()
            } else {
                name.to_string()
            }
        })
        .to_type_string()
}

struct AliasResolver<'a, 'b> {
    context: &'a str,
    aliases: &'a BTreeMap<String, String>,
    violations: &'b mut Vec<String>,
    type_param_scopes: Vec<BTreeSet<String>>,
    reported_generic_alias_uses: BTreeSet<String>,
}

impl<'a, 'b> AliasResolver<'a, 'b> {
    fn new(
        context: &'a str,
        aliases: &'a BTreeMap<String, String>,
        violations: &'b mut Vec<String>,
    ) -> Self {
        Self {
            context,
            aliases,
            violations,
            type_param_scopes: Vec::new(),
            reported_generic_alias_uses: BTreeSet::new(),
        }
    }

    fn expand_source_file(&mut self, ast: &mut SourceFile) {
        for ty in &mut ast.types {
            self.expand_type_decl(ty);
        }
        for alias in &mut ast.aliases {
            self.expand_type_ref(&mut alias.target_type);
        }
        for interface in &mut ast.interfaces {
            self.expand_interface(interface);
        }
        for operation in &mut ast.function_signatures {
            self.expand_operation(operation);
        }
        for function in &mut ast.functions {
            self.with_type_params(&function.type_params, |resolver| {
                resolver.expand_operation_parts(
                    &mut function.params,
                    &mut function.return_type,
                    function.implicit_self.as_mut(),
                );
                resolver.expand_block(&mut function.body);
            });
        }
        for implementation in &mut ast.impls {
            let implementation_type_params = generic_type_params(&implementation.target);
            self.with_type_params(&implementation_type_params, |resolver| {
                implementation.target =
                    resolver.expand_nominal_type_name(&implementation.target, &mut Vec::new());
                for method in &mut implementation.methods {
                    resolver.expand_operation(method);
                }
                for method in &mut implementation.method_bodies {
                    resolver.with_type_params(&method.type_params, |resolver| {
                        resolver.expand_operation_parts(
                            &mut method.params,
                            &mut method.return_type,
                            method.implicit_self.as_mut(),
                        );
                        resolver.expand_block(&mut method.body);
                    });
                }
            });
        }
        for constant in &mut ast.consts {
            if let Some(ty) = &mut constant.ty {
                self.expand_type_ref(ty);
            }
            self.expand_expr(&mut constant.value);
        }
        for test in &mut ast.tests {
            self.expand_block(&mut test.body);
        }
    }

    fn expand_type_decl(&mut self, ty: &mut TypeDecl) {
        if let Some(alias) = ty.alias.as_mut() {
            self.expand_type_ref(alias);
        }
        for implements in &mut ty.implements {
            self.expand_type_ref(implements);
        }
        for field in &mut ty.fields {
            self.expand_type_ref(&mut field.ty);
        }
    }

    fn expand_interface(&mut self, interface: &mut InterfaceDecl) {
        for operation in &mut interface.operations {
            self.expand_operation(operation);
        }
    }

    fn expand_operation(&mut self, operation: &mut InterfaceOperation) {
        self.with_type_params(&operation.type_params, |resolver| {
            resolver.expand_operation_parts(
                &mut operation.params,
                &mut operation.return_type,
                operation.implicit_self.as_mut(),
            );
        });
    }

    fn expand_operation_parts(
        &mut self,
        params: &mut [crate::shared::ast::Param],
        return_type: &mut TypeRef,
        implicit_self: Option<&mut TypeRef>,
    ) {
        for param in params {
            self.expand_type_ref(&mut param.ty);
        }
        self.expand_type_ref(return_type);
        if let Some(implicit_self) = implicit_self {
            self.expand_type_ref(implicit_self);
        }
    }

    fn expand_block(&mut self, block: &mut crate::shared::ast::Block) {
        self.visit_block(block);
    }

    fn expand_expr(&mut self, expr: &mut Expr) {
        self.visit_expr(expr);
    }

    fn expand_nominal_type_parts(&mut self, type_name: &mut String, type_args: &mut Vec<TypeRef>) {
        for type_arg in type_args.iter_mut() {
            self.expand_type_ref(type_arg);
        }
        let raw = if type_args.is_empty() {
            type_name.clone()
        } else {
            format!(
                "{}<{}>",
                type_name,
                type_args
                    .iter()
                    .map(|ty| ty.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let expanded = self.expand_nominal_type_name(&raw, &mut Vec::new());
        if let Some(parts) = generic_parts(&expanded) {
            *type_name = parts.root.trim().to_string();
            *type_args = parts
                .args
                .iter()
                .map(|arg| TypeRef {
                    name: arg.trim().to_string(),
                })
                .collect();
        } else {
            *type_name = expanded;
            type_args.clear();
        }
    }

    fn expand_type_ref(&mut self, ty: &mut TypeRef) {
        ty.name = self.expand_type_name(&ty.name);
    }

    fn expand_type_name(&mut self, raw: &str) -> String {
        self.expand_type_name_seen(raw, &mut Vec::new())
    }

    fn expand_nominal_type_name(&mut self, raw: &str, seen: &mut Vec<String>) -> String {
        let expanded = self.expand_type_name_seen(raw, seen);
        split_top_level(&expanded, '|')
            .into_iter()
            .next()
            .unwrap_or(expanded.as_str())
            .trim()
            .to_string()
    }

    fn expand_type_name_seen(&mut self, raw: &str, seen: &mut Vec<String>) -> String {
        let name = raw.trim();
        if name.is_empty() || string_literal(name).is_some() {
            return name.to_string();
        }
        if self.is_type_param(name) {
            return name.to_string();
        }

        if let Some(inner) = name.strip_suffix('?') {
            let expanded = self.expand_type_name_seen(inner, seen);
            return nullable_type_name(&expanded);
        }

        self.expand_type_expr(&TypeExpr::parse_lossy(name), seen)
            .to_type_string()
    }

    fn expand_type_expr(&mut self, ty: &TypeExpr, seen: &mut Vec<String>) -> TypeExpr {
        match ty {
            TypeExpr::Named { name, args } if args.is_empty() => {
                if self.is_type_param(name) {
                    return ty.clone();
                }
                let Some(target) = self.aliases.get(name) else {
                    return ty.clone();
                };
                if seen.iter().any(|entry| entry == name) {
                    return TypeExpr::parse_lossy(target);
                }
                seen.push(name.to_string());
                let expanded = self.expand_type_name_seen(target, seen);
                seen.pop();
                TypeExpr::parse_lossy(&expanded)
            }
            TypeExpr::Named { name, args } => {
                let updated_args = args
                    .iter()
                    .map(|arg| self.expand_type_expr(arg, seen))
                    .collect::<Vec<_>>();

                if self.aliases.contains_key(name) && !self.is_type_param(name) {
                    let reference = TypeExpr::Named {
                        name: name.clone(),
                        args: args.clone(),
                    };
                    self.report_generic_alias_use(name, &reference.to_type_string());
                }

                TypeExpr::Named {
                    name: name.clone(),
                    args: updated_args,
                }
            }
            TypeExpr::Nullable(inner) => {
                TypeExpr::Nullable(Box::new(self.expand_type_expr(inner, seen)))
            }
            TypeExpr::Union(parts) => TypeExpr::Union(
                parts
                    .iter()
                    .map(|part| self.expand_type_expr(part, seen))
                    .collect(),
            ),
            TypeExpr::Record(fields) => TypeExpr::Record(
                fields
                    .iter()
                    .map(|field| RecordTypeField {
                        name: field.name.clone(),
                        ty: self.expand_type_expr(&field.ty, seen),
                    })
                    .collect(),
            ),
            TypeExpr::Function {
                params,
                return_type,
            } => TypeExpr::Function {
                params: params
                    .iter()
                    .map(|param| FunctionTypeParam {
                        name: param.name.clone(),
                        ty: self.expand_type_expr(&param.ty, seen),
                    })
                    .collect(),
                return_type: Box::new(self.expand_type_expr(return_type, seen)),
            },
            _ => ty.clone(),
        }
    }

    fn report_generic_alias_use(&mut self, alias: &str, reference: &str) {
        let key = format!("{alias}\0{reference}");
        if self.reported_generic_alias_uses.insert(key) {
            self.violations.push(format!(
                "{}: alias {alias} does not accept type arguments in type reference {reference}",
                self.context
            ));
        }
    }

    fn with_type_params(&mut self, params: &[String], f: impl FnOnce(&mut Self)) {
        if params.is_empty() {
            f(self);
            return;
        }
        self.type_param_scopes
            .push(params.iter().cloned().collect::<BTreeSet<_>>());
        f(self);
        self.type_param_scopes.pop();
    }

    fn is_type_param(&self, name: &str) -> bool {
        self.type_param_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
    }
}

impl AstVisitorMut for AliasResolver<'_, '_> {
    fn visit_expr(&mut self, expr: &mut Expr) {
        if let Expr::Record {
            type_name,
            type_args,
            fields,
        } = expr
        {
            self.expand_nominal_type_parts(type_name, type_args);
            for (_, value) in fields {
                self.visit_expr(value);
            }
            return;
        }
        walk_expr_mut(self, expr);
    }

    fn visit_pattern(&mut self, pattern: &mut Pattern) {
        if let Pattern::Nominal {
            name,
            type_args,
            fields,
        } = pattern
        {
            self.expand_nominal_type_parts(name, type_args);
            for field in fields {
                if let Some(pattern) = &mut field.pattern {
                    self.visit_pattern(pattern);
                }
            }
            return;
        }
        walk_pattern_mut(self, pattern);
    }

    fn visit_type_ref(&mut self, ty: &mut TypeRef) {
        self.expand_type_ref(ty);
    }
}

fn nullable_type_name(inner: &str) -> String {
    let mut parts = split_top_level(inner, '|')
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        if !parts.iter().any(|part| part == "null") {
            parts.push("null".to_string());
        }
        return parts.join(" | ");
    }
    format!("{inner}?")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn visit_alias(
    context: &str,
    name: &str,
    aliases: &BTreeMap<String, String>,
    state: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
    reported: &mut BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    match state.get(name).copied() {
        Some(VisitState::Done) => return,
        Some(VisitState::Visiting) => {
            if let Some(start) = stack.iter().position(|entry| entry == name) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(name.to_string());
                let key = cycle.join(" -> ");
                if reported.insert(key.clone()) {
                    violations.push(format!(
                        "{context}: recursive alias cycle {key} is not supported"
                    ));
                }
            }
            return;
        }
        None => {}
    }

    let Some(target) = aliases.get(name) else {
        return;
    };
    state.insert(name.to_string(), VisitState::Visiting);
    stack.push(name.to_string());

    for reference in alias_type_references(target) {
        if aliases.contains_key(&reference) {
            visit_alias(
                context, &reference, aliases, state, stack, reported, violations,
            );
        }
    }

    stack.pop();
    state.insert(name.to_string(), VisitState::Done);
}

fn alias_type_references(raw: &str) -> Vec<String> {
    let mut refs = Vec::new();
    TypeExpr::parse_lossy(raw).for_each_named(|ty| refs.push(ty.to_string()));
    refs.sort();
    refs.dedup();
    refs
}

fn generic_type_params(name: &str) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .filter(|arg| {
                    !arg.is_empty()
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(|arg| (*arg).to_string())
                .collect()
        })
        .unwrap_or_default()
}
