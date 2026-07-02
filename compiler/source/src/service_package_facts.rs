use crate::{
    shared::ast::TypeRef,
    shared::ast_utils::{source_referenced_dotted_root_imports, AstVisitor},
    shared::prelude_registry::prelude_registry,
    shared::type_expr::TypeExpr,
    source_graph::CompilerSourceFile,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceSourcePackageFacts {
    pub imports: Vec<Vec<String>>,
    pub references_std_package_types: bool,
}

pub fn service_source_package_facts_from_compiler_sources(
    user_production_sources: &[CompilerSourceFile],
) -> ServiceSourcePackageFacts {
    ServiceSourcePackageFacts {
        imports: collect_service_publication_import_paths(user_production_sources),
        references_std_package_types: service_sources_reference_std_package_types(
            user_production_sources,
        ),
    }
}

fn collect_service_publication_import_paths(sources: &[CompilerSourceFile]) -> Vec<Vec<String>> {
    let mut imports = Vec::new();
    for source in sources {
        let ext_imports = source_referenced_dotted_root_imports(&source.ast, "ext");
        let references_std = !source_referenced_dotted_root_imports(&source.ast, "std").is_empty();
        imports.extend(source.ast.imports.iter().map(|import| import.path.clone()));
        imports.extend(ext_imports);
        if references_std {
            imports.push(vec!["std".to_string()]);
        }
    }
    imports
}

fn service_sources_reference_std_package_types(sources: &[CompilerSourceFile]) -> bool {
    sources.iter().any(|source| {
        let mut collector = StdPackageTypeReferenceCollector::default();
        collector.visit_source(&source.ast);
        collector.references_std_package
    })
}

#[derive(Default)]
struct StdPackageTypeReferenceCollector {
    references_std_package: bool,
}

impl StdPackageTypeReferenceCollector {
    fn visit_source(&mut self, ast: &crate::shared::ast::SourceFile) {
        for ty in &ast.types {
            for implemented in &ty.implements {
                self.visit_type_ref(implemented);
            }
            if let Some(alias) = &ty.alias {
                self.visit_type_ref(alias);
            }
            for field in &ty.fields {
                self.visit_type_ref(&field.ty);
            }
        }
        for alias in &ast.aliases {
            self.visit_type_ref(&alias.target_type);
        }
        for interface in &ast.interfaces {
            for operation in &interface.operations {
                self.visit_operation(operation);
            }
        }
        for operation in &ast.function_signatures {
            self.visit_operation(operation);
        }
        for function in &ast.functions {
            if let Some(implicit_self) = &function.implicit_self {
                self.visit_type_ref(implicit_self);
            }
            for param in &function.params {
                self.visit_type_ref(&param.ty);
            }
            self.visit_type_ref(&function.return_type);
            self.visit_block(&function.body);
        }
        for constant in &ast.consts {
            if let Some(ty) = &constant.ty {
                self.visit_type_ref(ty);
            }
            self.visit_expr(&constant.value);
        }
        for implementation in &ast.impls {
            for method in &implementation.methods {
                self.visit_operation(method);
            }
            for method in &implementation.method_bodies {
                if let Some(implicit_self) = &method.implicit_self {
                    self.visit_type_ref(implicit_self);
                }
                for param in &method.params {
                    self.visit_type_ref(&param.ty);
                }
                self.visit_type_ref(&method.return_type);
                self.visit_block(&method.body);
            }
        }
    }

    fn visit_operation(&mut self, operation: &crate::shared::ast::InterfaceOperation) {
        if let Some(implicit_self) = &operation.implicit_self {
            self.visit_type_ref(implicit_self);
        }
        for param in &operation.params {
            self.visit_type_ref(&param.ty);
        }
        self.visit_type_ref(&operation.return_type);
    }
}

impl AstVisitor for StdPackageTypeReferenceCollector {
    fn visit_type_ref(&mut self, ty: &TypeRef) {
        if self.references_std_package {
            return;
        }
        let registry = prelude_registry();
        TypeExpr::parse_lossy(&ty.name).for_each_named(|name| {
            if self.references_std_package {
                return;
            }
            let Some(symbol) = registry.known_type_symbol(name) else {
                return;
            };
            if symbol.starts_with("std.") && !registry.is_native_type_name(&symbol) {
                self.references_std_package = true;
            }
        });
    }
}
