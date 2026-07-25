use skiff_artifact_model::{
    FileIrUnit, NominalTypeRefBaseIr, PackageImplementationLinks, PackageRefIr, PackageSymbolRef,
    TypeDeclIr, TypeRefIr,
};

#[derive(Clone, Debug)]
pub struct PackageTypeSource {
    pub package_id: String,
    pub dependency_refs: Vec<String>,
    pub implementation_links: PackageImplementationLinks,
    pub file_ir_units: Vec<FileIrUnit>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NominalTypeKey {
    pub module_path: String,
    pub name: String,
}

impl NominalTypeKey {
    pub fn new(module_path: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedNominalType<'a> {
    pub key: NominalTypeKey,
    pub declaration: &'a TypeDeclIr,
}

impl<'a> ResolvedNominalType<'a> {
    pub fn new(module_path: impl Into<String>, declaration: &'a TypeDeclIr) -> Self {
        Self {
            key: NominalTypeKey::new(module_path, declaration.name.clone()),
            declaration,
        }
    }
}

pub trait NominalTypeResolver {
    fn resolve<'a>(
        &'a self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<ResolvedNominalType<'a>>;
}

#[derive(Clone, Copy, Debug)]
pub struct ArtifactNominalTypeSource<'a> {
    file_ir_units: &'a [FileIrUnit],
    package_sources: &'a [PackageTypeSource],
}

impl<'a> ArtifactNominalTypeSource<'a> {
    pub fn new(file_ir_units: &'a [FileIrUnit], package_sources: &'a [PackageTypeSource]) -> Self {
        Self {
            file_ir_units,
            package_sources,
        }
    }

    pub fn unit_by_module_path(&self, module_path: &str) -> Option<&'a FileIrUnit> {
        self.file_ir_units
            .iter()
            .find(|unit| unit.module_path == module_path)
            .or_else(|| {
                self.package_sources
                    .iter()
                    .flat_map(|package| package.file_ir_units.iter())
                    .find(|unit| unit.module_path == module_path)
            })
    }

    pub fn package_source_for_ref(
        &self,
        package_ref: &PackageRefIr,
    ) -> Option<&'a PackageTypeSource> {
        let package_key = match package_ref {
            PackageRefIr::PackageId { package_id } => package_id,
            PackageRefIr::Dependency { dependency_ref } => dependency_ref,
        };
        self.package_sources.iter().find(|source| {
            source.package_id == *package_key
                || source
                    .dependency_refs
                    .iter()
                    .any(|dependency_ref| dependency_ref == package_key)
        })
    }

    pub fn resolve_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Option<ResolvedNominalType<'a>> {
        self.resolve_package_symbol_parts(symbol)
            .map(|(module_path, declaration)| {
                ResolvedNominalType::new(module_path.to_string(), declaration)
            })
    }

    pub fn resolve_package_symbol_parts(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Option<(&'a str, &'a TypeDeclIr)> {
        let package = self.package_source_for_ref(&symbol.package)?;
        let export = package
            .implementation_links
            .types
            .get(&symbol.symbol_path)?;
        let unit = package
            .file_ir_units
            .iter()
            .find(|unit| unit.file_ir_identity == export.file.file_ir_identity)
            .or_else(|| {
                package
                    .file_ir_units
                    .iter()
                    .find(|unit| unit.module_path == export.file.module_path)
            })?;
        unit.type_table
            .get(export.type_index as usize)
            .map(|declaration| (unit.module_path.as_str(), declaration))
    }

    pub fn resolve_symbol_in_module(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<ResolvedNominalType<'a>> {
        self.resolve_symbol_in_module_parts(module_path, symbol)
            .map(|(module_path, declaration)| {
                ResolvedNominalType::new(module_path.to_string(), declaration)
            })
    }

    pub fn resolve_symbol_in_module_parts(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<(&'a str, &'a TypeDeclIr)> {
        let unit = self.unit_by_module_path(module_path)?;
        let declaration = unit.declarations.types.get(symbol)?;
        unit.type_table
            .get(declaration.type_index as usize)
            .map(|declaration| (unit.module_path.as_str(), declaration))
    }

    pub fn resolve_type_ref(
        &self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<ResolvedNominalType<'a>> {
        self.resolve_type_ref_parts(current_module, ty)
            .map(|(module_path, declaration)| {
                ResolvedNominalType::new(module_path.to_string(), declaration)
            })
    }

    pub fn resolve_type_ref_parts(
        &self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<(&'a str, &'a TypeDeclIr)> {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let unit = self.unit_by_module_path(current_module)?;
                unit.type_table
                    .get(*type_index as usize)
                    .map(|declaration| (unit.module_path.as_str(), declaration))
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let unit = self.unit_by_module_path(module_path)?;
                unit.type_table
                    .get(*type_index as usize)
                    .map(|declaration| (unit.module_path.as_str(), declaration))
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                self.resolve_symbol_in_module_parts(&symbol.module_path, &symbol.symbol)
            }
            TypeRefIr::PackageSymbol { symbol } => self.resolve_package_symbol_parts(symbol),
            TypeRefIr::AppliedNominal { base, .. } => match base {
                NominalTypeRefBaseIr::LocalType { type_index } => {
                    let unit = self.unit_by_module_path(current_module)?;
                    unit.type_table
                        .get(*type_index as usize)
                        .map(|declaration| (unit.module_path.as_str(), declaration))
                }
                NominalTypeRefBaseIr::PublicationType {
                    module_path,
                    type_index,
                } => {
                    let unit = self.unit_by_module_path(module_path)?;
                    unit.type_table
                        .get(*type_index as usize)
                        .map(|declaration| (unit.module_path.as_str(), declaration))
                }
                NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                    self.resolve_symbol_in_module_parts(&symbol.module_path, &symbol.symbol)
                }
                NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                    self.resolve_package_symbol_parts(symbol)
                }
                NominalTypeRefBaseIr::PackageSchema { .. } => None,
            },
            TypeRefIr::Builtin { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => None,
        }
    }
}

impl NominalTypeResolver for ArtifactNominalTypeSource<'_> {
    fn resolve<'a>(
        &'a self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<ResolvedNominalType<'a>> {
        self.resolve_type_ref(current_module, ty)
    }
}

pub fn type_decl_for_symbol_in_unit<'a>(
    unit: &'a FileIrUnit,
    symbol_name: &str,
) -> Option<&'a TypeDeclIr> {
    unit.declarations
        .types
        .get(symbol_name)
        .and_then(|declaration| unit.type_table.get(declaration.type_index as usize))
        .or_else(|| unit.type_table.iter().find(|decl| decl.name == symbol_name))
}

pub fn is_nominal_type_ref(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::AppliedNominal { .. }
            | TypeRefIr::DbObjectSymbol { .. }
    )
}
