use super::*;

pub(super) fn validate_type_decl_discriminator(
    name: &str,
    ty: &str,
    discriminator: Option<&str>,
    location: SourceLocation,
) -> Result<()> {
    let union = split_top_level(ty.trim(), '|');
    if union.len() <= 1 {
        if discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "type {name} discriminator can only be used with anonymous record union branches"
                ),
                location,
            ));
        }
        return Ok(());
    }

    let anonymous_record_branches = union
        .iter()
        .filter_map(|part| parser_record_type_fields(part.trim()))
        .collect::<Vec<_>>();
    if anonymous_record_branches.is_empty() {
        if discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "type {name} discriminator can only be used with anonymous record union branches"
                ),
                location,
            ));
        }
        return Ok(());
    }

    let Some(discriminator) = discriminator else {
        return Err(CompileError::syntax(
            format!(
                "named union type {name} uses anonymous record branches; add discriminator \"tag\" to the type declaration"
            ),
            location,
        ));
    };

    let mut values = BTreeSet::new();
    for fields in anonymous_record_branches {
        let Some(value) = discriminator_record_branch_value(&fields, discriminator) else {
            return Err(CompileError::syntax(
                format!(
                    "anonymous record union branch in {name} must declare {discriminator} as a string literal"
                ),
                location,
            ));
        };
        if !values.insert(value.clone()) {
            return Err(CompileError::syntax(
                format!(
                    "anonymous record union branch {discriminator} \"{value}\" in {name} must be unique"
                ),
                location,
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_actor_declarations(
    actors: &[ActorDecl],
    types: &[TypeDecl],
    dbs: &[DbDecl],
) -> Result<()> {
    let type_by_name = types
        .iter()
        .map(|declaration| (declaration.name.as_str(), declaration))
        .collect::<BTreeMap<_, _>>();
    let db_type_names = dbs
        .iter()
        .map(|declaration| declaration.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut actor_names = BTreeSet::new();
    for actor in actors {
        if !actor_names.insert(actor.name.as_str()) {
            return Err(CompileError::syntax(
                format!("duplicated actor declaration {}", actor.name),
                actor.span.start,
            ));
        }
        let attached = type_by_name.get(actor.name.as_str()).ok_or_else(|| {
            CompileError::syntax(
                format!(
                    "actor {} requires a same-file type declaration of the same name",
                    actor.name
                ),
                actor.span.start,
            )
        })?;
        if !attached.type_params.is_empty() {
            return Err(CompileError::syntax(
                format!(
                    "actor {} must attach to a non-generic type declaration",
                    actor.name
                ),
                actor.span.start,
            ));
        }
        if attached.alias.is_some() || attached.discriminator.is_some() {
            return Err(CompileError::syntax(
                format!(
                    "actor {} must attach to a concrete record type declaration",
                    actor.name
                ),
                actor.span.start,
            ));
        }
        if !attached
            .fields
            .iter()
            .any(|field| field.name == actor.key_field)
        {
            return Err(CompileError::syntax(
                format!(
                    "actor {} key({}) must name a field of the attached type {}",
                    actor.name, actor.key_field, actor.name
                ),
                actor.span.start,
            ));
        }
        if db_type_names.contains(actor.name.as_str()) {
            return Err(CompileError::syntax(
                format!(
                    "type {} cannot attach both db object and actor declarations",
                    actor.name
                ),
                actor.span.start,
            ));
        }
    }
    Ok(())
}

fn discriminator_record_branch_value(
    fields: &[(String, String)],
    discriminator: &str,
) -> Option<String> {
    fields.iter().find_map(|(field_name, field_type)| {
        (field_name == discriminator)
            .then(|| string_literal(field_type))
            .flatten()
    })
}

fn parser_record_type_fields(ty: &str) -> Option<Vec<(String, String)>> {
    record_type_fields(ty).map(|fields| {
        fields
            .into_iter()
            .map(|field| (field.name.to_string(), field.ty.to_string()))
            .collect()
    })
}
