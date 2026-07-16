use std::convert::Infallible;

use skiff_artifact_model::{FileIrUnit, LiteralIr, TypeRefIr};
use skiff_compiler_core::type_closure::{
    ArtifactNominalTypeSource, NoTypeClosureGuards, NominalTypeKey, NominalTypeResolver,
    PackageTypeSource, RepresentationIndirectionGuards, ResolvedNominalType, TypeClosureControl,
    TypeClosurePolicy, TypeClosureTrace, TypeClosureVisit, TypeClosureWalker,
};
use skiff_compiler_core::type_ref::{TypeRefVisit, TypeRefVisitPath};

use crate::type_closure_diagnostics::type_closure_trace_suffix;

use super::{
    contract_index_type_decl_for_type_ref, static_type_ref_boundary_policy, BoundaryKind,
    BoundaryTypePolicyDecision, ContractProjection, ContractProjectionIndex,
};

pub(super) fn recursive_type_violations(
    index: &ContractProjectionIndex<'_>,
    projection: &ContractProjection,
) -> Vec<String> {
    let package_sources = Vec::new();
    let resolver = BoundaryNominalTypeResolver::new(index, &package_sources);
    let guards = RepresentationIndirectionGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut violations = Vec::new();

    for ty in projection.types.values() {
        let Some(declaration) =
            index.type_decl_by_module_local_name(&ty.source_module, &ty.source_name)
        else {
            continue;
        };
        let root = ResolvedNominalType::new(ty.source_module.clone(), declaration);
        let cycles = root_cycle_guards(&walker, &root);
        if matches!(
            declaration.descriptor,
            skiff_artifact_model::TypeDescriptorIr::Alias { .. }
                | skiff_artifact_model::TypeDescriptorIr::Union { .. }
        ) && !cycles.is_empty()
        {
            violations.push(format!(
                "recursive representation or union type {} is not supported in service boundary schema",
                ty.public_name
            ));
        }
        if matches!(
            declaration.descriptor,
            skiff_artifact_model::TypeDescriptorIr::Record { .. }
        ) {
            violations.extend(cycles.into_iter().map(|guarded| {
                let guard = if guarded { "guarded " } else { "" };
                format!(
                    "{guard}recursive record type {} is not supported in service boundary schema until runtime schema definitions are published",
                    ty.public_name
                )
            }));
        }
    }

    for alias in projection.aliases.values() {
        let Some(declaration) =
            index.type_decl_by_module_local_name(&alias.source_module, &alias.source_name)
        else {
            continue;
        };
        let root = ResolvedNominalType::new(alias.source_module.clone(), declaration);
        if !root_cycle_guards(&walker, &root).is_empty() {
            violations.push(format!(
                "recursive representation or union type {} is not supported in service boundary schema",
                alias.public_name
            ));
        }
    }

    violations
}

fn root_cycle_guards<R: NominalTypeResolver>(
    walker: &TypeClosureWalker<'_, R, RepresentationIndirectionGuards>,
    root: &ResolvedNominalType<'_>,
) -> Vec<bool> {
    let mut policy = RootCyclePolicy {
        root: root.key.clone(),
        guarded: Vec::new(),
    };
    let result = walker.walk_declaration(root, &mut policy);
    match result {
        Ok(()) => policy.guarded,
        Err(failure) => match failure.error {},
    }
}

struct RootCyclePolicy {
    root: NominalTypeKey,
    guarded: Vec<bool>,
}

impl TypeClosurePolicy for RootCyclePolicy {
    type Error = Infallible;

    fn nominal_cycle(
        &mut self,
        visit: TypeClosureVisit<'_>,
        resolved: &ResolvedNominalType<'_>,
    ) -> Result<(), Self::Error> {
        if resolved.key == self.root {
            self.guarded.push(visit.guarded);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundaryTypeRefClosureViolation {
    trace: TypeClosureTrace,
    pub message: String,
}

impl BoundaryTypeRefClosureViolation {
    pub fn trace_suffix(&self) -> String {
        type_closure_trace_suffix(&self.trace)
    }
}

pub(crate) struct BoundaryTypeRefClosureValidator<'a, 'p> {
    index: &'a ContractProjectionIndex<'a>,
    package_sources: &'p [PackageTypeSource],
}

impl<'a, 'p> BoundaryTypeRefClosureValidator<'a, 'p> {
    pub fn new(
        index: &'a ContractProjectionIndex<'a>,
        package_sources: &'p [PackageTypeSource],
    ) -> Self {
        Self {
            index,
            package_sources,
        }
    }

    pub fn validate_type_ref_closure(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
    ) -> Vec<BoundaryTypeRefClosureViolation> {
        let resolver = BoundaryNominalTypeResolver::new(self.index, self.package_sources);
        let guards = NoTypeClosureGuards;
        let walker = TypeClosureWalker::new(&resolver, &guards);
        let mut policy = BoundaryClosurePolicy {
            boundary_kind,
            violations: Vec::new(),
        };
        let result = walker.walk(module_path, ty, &mut policy);
        match result {
            Ok(()) => policy.violations,
            Err(failure) => match failure.error {},
        }
    }

    pub fn display_type_ref(&self, module_path: &str, ty: &TypeRefIr) -> String {
        match ty {
            TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
            TypeRefIr::Native { name, args } => format!(
                "{name}<{}>",
                args.iter()
                    .map(|arg| self.display_type_ref(module_path, arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeRefIr::LocalType { type_index } => self
                .unit_by_module_path(module_path)
                .and_then(|unit| unit.type_table.get(*type_index as usize))
                .map(|decl| decl.name.clone())
                .unwrap_or_else(|| format!("<missing:{module_path}:{type_index}>")),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self
                .unit_by_module_path(module_path)
                .and_then(|unit| unit.type_table.get(*type_index as usize))
                .map(|decl| decl.name.clone())
                .unwrap_or_else(|| format!("<missing publication type:{module_path}>")),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let source_module = self
                    .index
                    .source_module_for_reference_module(&symbol.module_path);
                let module_path = if source_module.is_empty() {
                    symbol.module_path.as_str()
                } else {
                    source_module
                };
                if module_path.is_empty() {
                    symbol.symbol.clone()
                } else {
                    format!("{module_path}.{}", symbol.symbol)
                }
            }
            TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
            TypeRefIr::Record { fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, ty)| format!("{name}: {}", self.display_type_ref(module_path, ty)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{fields}}}")
            }
            TypeRefIr::Union { items } => items
                .iter()
                .map(|item| self.display_type_ref(module_path, item))
                .collect::<Vec<_>>()
                .join(" | "),
            TypeRefIr::Nullable { inner } => {
                format!("{}?", self.display_type_ref(module_path, inner))
            }
            TypeRefIr::Literal { value } => match value {
                LiteralIr::Null => "null".to_string(),
                LiteralIr::Bool { value } => value.to_string(),
                LiteralIr::Number { value } => value.to_string(),
                LiteralIr::String { value } => format!("\"{value}\""),
            },
            TypeRefIr::TypeParam { name } => name.clone(),
            TypeRefIr::AnyInterface { interface } => {
                let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                    .map_or_else(
                        |_| interface.interface_abi_id.clone(),
                        |ty| self.display_type_ref(module_path, &ty),
                    );
                if interface.canonical_type_args.is_empty() {
                    format!("any {interface_name}")
                } else {
                    format!(
                        "any {interface_name}<{}>",
                        interface
                            .canonical_type_args
                            .iter()
                            .map(|arg| self.display_type_ref(module_path, arg))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                let params = params
                    .iter()
                    .map(|param| {
                        format!(
                            "{}: {}",
                            param.name,
                            self.display_type_ref(module_path, &param.ty)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "fn({params}) -> {}",
                    self.display_type_ref(module_path, return_type)
                )
            }
        }
    }

    fn unit_by_module_path(&self, module_path: &str) -> Option<&FileIrUnit> {
        self.index.unit_by_module_path(module_path).or_else(|| {
            self.package_sources
                .iter()
                .flat_map(|package| package.file_ir_units.iter())
                .find(|unit| unit.module_path == module_path)
        })
    }
}

struct BoundaryNominalTypeResolver<'a> {
    index: &'a ContractProjectionIndex<'a>,
    package_source: ArtifactNominalTypeSource<'a>,
}

impl<'a> BoundaryNominalTypeResolver<'a> {
    fn new(
        index: &'a ContractProjectionIndex<'a>,
        package_sources: &'a [PackageTypeSource],
    ) -> Self {
        Self {
            index,
            package_source: ArtifactNominalTypeSource::new(&[], package_sources),
        }
    }
}

impl NominalTypeResolver for BoundaryNominalTypeResolver<'_> {
    fn resolve<'a>(
        &'a self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<ResolvedNominalType<'a>> {
        contract_index_type_decl_for_type_ref(self.index, current_module, ty)
            .map(|(module_path, declaration)| {
                ResolvedNominalType::new(module_path.to_string(), declaration)
            })
            .or_else(|| self.package_source.resolve(current_module, ty))
    }
}

struct BoundaryClosurePolicy {
    boundary_kind: BoundaryKind,
    violations: Vec<BoundaryTypeRefClosureViolation>,
}

impl TypeClosurePolicy for BoundaryClosurePolicy {
    type Error = Infallible;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        let decision = static_type_ref_boundary_policy(
            self.boundary_kind,
            TypeRefVisit {
                ty: visit.ty,
                path: TypeRefVisitPath::empty(),
            },
        );
        match decision {
            BoundaryTypePolicyDecision::Accept => Ok(TypeClosureControl::Continue),
            BoundaryTypePolicyDecision::Reject(message) => {
                self.violations.push(BoundaryTypeRefClosureViolation {
                    trace: visit.trace.clone(),
                    message,
                });
                Ok(TypeClosureControl::Prune)
            }
        }
    }
}
