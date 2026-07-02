use std::collections::BTreeSet;

use skiff_compiler_source::prelude_registry::{
    prelude_identity, prelude_registry, prelude_schema_identity,
};

mod projection_model;

pub(crate) fn projection_prelude_context() -> skiff_compiler_projection::prelude::PreludeProjection
{
    let registry = prelude_registry();
    let mut package_schema_import_required = BTreeSet::new();
    for name in registry
        .type_symbols()
        .keys()
        .chain(registry.type_symbols().values())
    {
        if registry.package_schema_type_requires_import(name) {
            package_schema_import_required.insert(name.clone());
            if let Some(symbol) = registry.known_type_symbol(name) {
                package_schema_import_required.insert(symbol);
            }
        }
    }

    let bare_raw_http_envelope_types = ["HttpRequest", "HttpResponse"]
        .into_iter()
        .filter(|name| registry.is_bare_raw_http_envelope_type(name))
        .map(str::to_string)
        .collect();

    skiff_compiler_projection::prelude::PreludeProjection::new(
        skiff_compiler_projection::prelude::PreludeProjectionParts {
            identity: prelude_identity(),
            schema_identity: prelude_schema_identity(),
            types: registry.prelude_types().to_vec(),
            roots: registry.prelude_roots().to_vec(),
            type_declarations: registry
                .declared_types()
                .map(|ty| projection_model::lower_prelude_type_decl(registry, ty))
                .collect(),
            type_symbols: registry.type_symbols().clone(),
            schema_stable_types: registry.schema_stable_types().iter().cloned().collect(),
            package_schema_import_required,
            bare_raw_http_envelope_types,
        },
    )
}
