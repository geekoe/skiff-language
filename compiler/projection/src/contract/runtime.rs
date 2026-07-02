use std::collections::{BTreeMap, BTreeSet};

use crate::{
    contract_schema::descriptor::RuntimeTypeDescriptorIr, projection_source_symbol_text,
    runtime_manifest_model::JsonSchema,
};
use skiff_artifact_model::InterfaceInstantiationRef;
use skiff_artifact_model::{LiteralIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr};
use skiff_compiler_core::{
    prelude_registry::compiler_owned_type_symbol,
    prelude_registry::PRELUDE_REGISTRY_ID,
    type_syntax::{generic_parts, record_type_fields, split_top_level, string_literal},
};
use skiff_compiler_projection_input::ProjectionSourceSymbolKey;

use crate::prelude::PreludeProjection;

use super::{
    ContractAliasProjection, ContractLiteralKey, ContractNamedTypeKey, ContractPackageRefKey,
    ContractProjection, ContractProjectionIndex, ContractProjectionTypeBinding,
    ContractTypeDescriptorProjection, ContractTypeKey, ContractTypeProjection,
};

impl ContractProjection {
    pub fn schema_for_type_key(&self, key: &ContractTypeKey) -> JsonSchema {
        self.schema_for_type_key_inner(key)
    }

    pub fn runtime_descriptor_for_type_key(
        &self,
        key: &ContractTypeKey,
    ) -> RuntimeTypeDescriptorIr {
        self.runtime_descriptor_for_type_key_inner(key)
    }

    pub fn schema_for_source_type_ref(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> JsonSchema {
        let mut seen = BTreeSet::new();
        self.schema_for_source_type_ref_inner(index, module_path, ty, &mut seen)
    }

    pub fn runtime_descriptor_for_source_type_ref(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> RuntimeTypeDescriptorIr {
        let mut seen = BTreeSet::new();
        self.runtime_descriptor_for_source_type_ref_inner(index, module_path, ty, &mut seen)
    }

    fn schema_for_type_key_inner(&self, key: &ContractTypeKey) -> JsonSchema {
        match key {
            ContractTypeKey::Builtin { name, args } => self.schema_for_builtin(name, args),
            ContractTypeKey::Named(name) => self.schema_for_named_key(name),
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => self
                .schema_for_standard_library_package_symbol(package, symbol_path)
                .unwrap_or_else(JsonSchema::any),
            ContractTypeKey::AnyInterface { .. } => boundary_rejected_type_schema(),
            ContractTypeKey::DbObjectSymbol { .. } => JsonSchema::any(),
            ContractTypeKey::Record { fields } => self.schema_object_for_fields(fields),
            ContractTypeKey::Union { items } => self.schema_for_union(items),
            ContractTypeKey::Nullable { inner } => self.schema_for_type_key_inner(inner).nullable(),
            ContractTypeKey::Literal(literal) => self.schema_for_literal(literal),
            ContractTypeKey::TypeParam { .. } | ContractTypeKey::Function { .. } => {
                JsonSchema::any()
            }
        }
    }

    fn schema_for_source_type_ref_inner(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        match ty {
            TypeRefIr::Native { name, args } => {
                self.schema_for_source_builtin(index, module_path, name, args, seen)
            }
            TypeRefIr::LocalType { type_index } => index
                .type_binding_by_module_type_index(module_path, *type_index)
                .map(|binding| self.schema_for_source_type_binding(index, binding, seen))
                .unwrap_or_else(JsonSchema::any),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => index
                .type_binding_by_module_type_index(module_path, *type_index)
                .map(|binding| self.schema_for_source_type_binding(index, binding, seen))
                .unwrap_or_else(JsonSchema::any),
            TypeRefIr::ServiceSymbol { symbol } => {
                if let Some(source_key) =
                    index.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                {
                    if let Some(binding) = index.type_binding_by_source_key(&source_key) {
                        return self.schema_for_source_type_binding(index, binding, seen);
                    }
                }
                let source_module = index.source_module_for_reference_module(&symbol.module_path);
                if let Some(binding) =
                    index.type_binding_by_module_local_name(source_module, &symbol.symbol)
                {
                    return self.schema_for_source_type_binding(index, binding, seen);
                }
                self.schema_for_source_type_key_fallback(index, module_path, ty, seen)
            }
            TypeRefIr::PackageSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
                self.schema_for_source_type_key_fallback(index, module_path, ty, seen)
            }
            TypeRefIr::Record { fields } => {
                self.schema_object_for_source_fields(index, module_path, fields, seen)
            }
            TypeRefIr::Union { items } => {
                self.schema_for_source_union(index, module_path, items, seen)
            }
            TypeRefIr::Nullable { inner } => self
                .schema_for_source_type_ref_inner(index, module_path, inner, seen)
                .nullable(),
            TypeRefIr::Literal { value } => self.schema_for_source_literal(value),
            TypeRefIr::AnyInterface { .. } => boundary_rejected_type_schema(),
            TypeRefIr::TypeParam { .. } | TypeRefIr::Function { .. } => JsonSchema::any(),
        }
    }

    fn schema_for_source_type_key_fallback(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        match index.canonical_type_ref(module_path, ty) {
            Ok(key) => self.schema_for_source_type_key_inner(index, &key, seen),
            Err(_) => JsonSchema::any(),
        }
    }

    fn schema_for_source_type_key_inner(
        &self,
        index: &ContractProjectionIndex<'_>,
        key: &ContractTypeKey,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        match key {
            ContractTypeKey::Builtin { name, args } => {
                let arg_schemas = args
                    .iter()
                    .map(|arg| self.schema_for_source_type_key_inner(index, arg, seen))
                    .collect::<Vec<_>>();
                self.schema_for_builtin_schemas(name, args, &arg_schemas)
            }
            ContractTypeKey::Named(ContractNamedTypeKey::Public { symbol }) => {
                self.schema_for_public_symbol(symbol)
            }
            ContractTypeKey::Named(ContractNamedTypeKey::Source { source }) => {
                let source_symbol = projection_source_symbol_text(source);
                index
                    .type_binding_by_source_key(source)
                    .map(|binding| self.schema_for_source_type_binding(index, binding, seen))
                    .or_else(|| self.schema_for_standard_library_type(&source_symbol, seen))
                    .unwrap_or_else(|| JsonSchema::reference(&source_symbol))
            }
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => self
                .schema_for_standard_library_package_symbol(package, symbol_path)
                .unwrap_or_else(|| self.schema_for_type_key_inner(key)),
            ContractTypeKey::AnyInterface { .. } => boundary_rejected_type_schema(),
            ContractTypeKey::DbObjectSymbol { .. } => self.schema_for_type_key_inner(key),
            ContractTypeKey::Record { fields } => {
                let mut properties = BTreeMap::new();
                let mut required = Vec::new();
                for (name, ty) in fields {
                    let field_schema = self.schema_for_source_type_key_inner(index, ty, seen);
                    if !field_schema.is_nullable() {
                        required.push(name.clone());
                    }
                    properties.insert(name.clone(), field_schema);
                }
                JsonSchema::object(properties, required, false)
            }
            ContractTypeKey::Union { items } => {
                if let Some(schema) = self.string_literal_union_schema(items) {
                    return schema;
                }
                if is_transport_message_union(items) {
                    return self.standard_library_connection_message_schema(seen);
                }
                JsonSchema::one_of(
                    items
                        .iter()
                        .map(|item| self.schema_for_source_type_key_inner(index, item, seen))
                        .collect(),
                )
            }
            ContractTypeKey::Nullable { inner } => self
                .schema_for_source_type_key_inner(index, inner, seen)
                .nullable(),
            ContractTypeKey::Literal(literal) => self.schema_for_literal(literal),
            ContractTypeKey::TypeParam { .. } | ContractTypeKey::Function { .. } => {
                JsonSchema::any()
            }
        }
    }

    fn schema_for_source_builtin(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        name: &str,
        args: &[TypeRefIr],
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        let arg_schemas = args
            .iter()
            .map(|arg| self.schema_for_source_type_ref_inner(index, module_path, arg, seen))
            .collect::<Vec<_>>();
        let arg_keys = args
            .iter()
            .map(|arg| index.canonical_type_ref(module_path, arg))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        self.schema_for_builtin_schemas(name, &arg_keys, &arg_schemas)
    }

    fn schema_for_builtin_schemas(
        &self,
        name: &str,
        arg_keys: &[ContractTypeKey],
        arg_schemas: &[JsonSchema],
    ) -> JsonSchema {
        match (name, arg_keys, arg_schemas) {
            ("string", [], []) => JsonSchema::string(),
            ("integer", [], []) => JsonSchema::typed("integer"),
            ("number", [], []) => JsonSchema::typed("number"),
            ("bool" | "boolean", [], []) => JsonSchema::typed("boolean"),
            ("null" | "void", [], []) => JsonSchema::typed("null"),
            ("Date", [], []) => date_time_schema(),
            ("bytes", [], []) | ("Json", [], []) | ("JsonObject", [], []) => self
                .schema_for_standard_library_type(name, &mut BTreeSet::new())
                .unwrap_or_else(JsonSchema::any),
            ("Array", [_], [inner]) => JsonSchema::array(inner.clone()),
            ("Map", [key, _], [_, value]) => self.schema_for_map_schema(key, value.clone()),
            (root, [_], [context]) if is_connect_result_generic_type(root) => self
                .schema_for_generic_standard_library_type(root, &[context.clone()])
                .unwrap_or_else(JsonSchema::any),
            (root, [_], [context]) if is_websocket_connection_generic_type(root) => {
                self.websocket_connection_schema_for_schema(context.clone())
            }
            (root, [_], [context]) if is_websocket_receive_event_generic_type(root) => {
                self.websocket_receive_event_schema_for_schema(context.clone())
            }
            (name, [], []) => self
                .schema_for_standard_library_type(name, &mut BTreeSet::new())
                .unwrap_or_else(JsonSchema::any),
            _ => JsonSchema::any(),
        }
    }

    fn schema_for_map_schema(&self, key: &ContractTypeKey, value: JsonSchema) -> JsonSchema {
        if !self.is_schema_map_key_type(key) {
            return JsonSchema::any();
        }
        let mut schema = JsonSchema::map(value);
        if let Some(symbol) = self.map_key_symbol(key) {
            schema = schema
                .with_x_skiff_map_key_symbol(symbol)
                .with_x_skiff_map_key_schema(self.schema_for_map_key_metadata(key));
        }
        schema
    }

    fn schema_for_source_type_binding(
        &self,
        index: &ContractProjectionIndex<'_>,
        binding: ContractProjectionTypeBinding<'_>,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        let source_key = ProjectionSourceSymbolKey::new(binding.module_path, binding.local_name);
        let source_symbol = projection_source_symbol_text(&source_key);
        if let Some(public_symbol) = self.public_schema_symbol_for_source(index, &source_key) {
            return self.schema_for_public_symbol(&public_symbol);
        }
        if !seen.insert(source_symbol.clone()) {
            return JsonSchema::reference(&source_symbol);
        }

        let schema = match binding.descriptor() {
            TypeDescriptorIr::Record { fields } => self
                .schema_object_for_source_fields(index, binding.module_path, fields, seen)
                .with_x_skiff_symbol(source_symbol.clone()),
            TypeDescriptorIr::Alias { target } => {
                let mut schema = self
                    .schema_for_source_type_ref_with_union_discriminator(
                        index,
                        binding.module_path,
                        target,
                        binding.discriminator(),
                        seen,
                    )
                    .with_x_skiff_symbol(source_symbol.clone())
                    .with_x_skiff_alias(source_type_ref_display_text(
                        index,
                        binding.module_path,
                        target,
                    ));
                if let Some(discriminator) = binding.discriminator() {
                    schema = schema.with_x_skiff_union_discriminator(discriminator.to_string());
                }
                schema
            }
            TypeDescriptorIr::Union { variants } => {
                let mut schema = self
                    .schema_for_source_union_with_discriminator(
                        index,
                        binding.module_path,
                        variants,
                        binding.discriminator(),
                        seen,
                    )
                    .with_x_skiff_symbol(source_symbol.clone());
                if let Some(discriminator) = binding.discriminator() {
                    schema = schema.with_x_skiff_union_discriminator(discriminator.to_string());
                }
                schema
            }
            TypeDescriptorIr::Native { .. } => {
                JsonSchema::any().with_x_skiff_symbol(source_symbol.clone())
            }
        };
        seen.remove(&source_symbol);
        schema
    }

    fn schema_object_for_source_fields(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();
        for (name, ty) in fields {
            let field_schema = self.schema_for_source_type_ref_inner(index, module_path, ty, seen);
            if !field_schema.is_nullable() {
                required.push(name.clone());
            }
            properties.insert(name.clone(), field_schema);
        }
        JsonSchema::object(properties, required, false)
    }

    fn schema_for_source_union(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        items: &[TypeRefIr],
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        if let Some(schema) = source_string_literal_union_schema(items) {
            return schema;
        }
        if self.source_transport_message_union(index, module_path, items) {
            return self.standard_library_connection_message_schema(seen);
        }
        JsonSchema::one_of(
            items
                .iter()
                .map(|item| self.schema_for_source_type_ref_inner(index, module_path, item, seen))
                .collect(),
        )
    }

    fn schema_for_source_type_ref_with_union_discriminator(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
        discriminator: Option<&str>,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        let Some(discriminator) = discriminator else {
            return self.schema_for_source_type_ref_inner(index, module_path, ty, seen);
        };
        match ty {
            TypeRefIr::Nullable { inner } => self
                .schema_for_source_type_ref_with_union_discriminator(
                    index,
                    module_path,
                    inner,
                    Some(discriminator),
                    seen,
                )
                .nullable(),
            TypeRefIr::Union { items } => self.schema_for_source_union_with_discriminator(
                index,
                module_path,
                items,
                Some(discriminator),
                seen,
            ),
            _ => self.schema_for_source_type_ref_inner(index, module_path, ty, seen),
        }
    }

    fn schema_for_source_union_with_discriminator(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        items: &[TypeRefIr],
        discriminator: Option<&str>,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        JsonSchema::one_of(
            items
                .iter()
                .map(|item| {
                    let mut schema =
                        self.schema_for_source_type_ref_inner(index, module_path, item, seen);
                    if let Some(tag) = discriminator
                        .and_then(|field| source_record_discriminator_value(item, field))
                    {
                        schema = schema.with_x_skiff_union_branch(tag);
                    }
                    schema
                })
                .collect(),
        )
    }

    fn source_transport_message_union(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        items: &[TypeRefIr],
    ) -> bool {
        let Ok(keys) = items
            .iter()
            .map(|item| index.canonical_type_ref(module_path, item))
            .collect::<Result<Vec<_>, _>>()
        else {
            return false;
        };
        is_transport_message_union(&keys)
    }

    fn schema_for_source_literal(&self, literal: &LiteralIr) -> JsonSchema {
        match literal {
            LiteralIr::Null => JsonSchema::typed("null"),
            LiteralIr::Bool { .. } => JsonSchema::typed("boolean"),
            LiteralIr::Number { .. } => JsonSchema::typed("number"),
            LiteralIr::String { value } => JsonSchema::string_enum(vec![value.clone()]),
        }
    }

    fn websocket_connection_schema_for_schema(&self, context: JsonSchema) -> JsonSchema {
        let mut properties = BTreeMap::new();
        properties.insert("id".to_string(), JsonSchema::string());
        properties.insert(
            "businessIdentity".to_string(),
            JsonSchema::string().nullable(),
        );
        properties.insert("context".to_string(), context);
        JsonSchema::object(
            properties,
            vec![
                "id".to_string(),
                "businessIdentity".to_string(),
                "context".to_string(),
            ],
            false,
        )
    }

    fn websocket_receive_event_schema_for_schema(&self, context: JsonSchema) -> JsonSchema {
        let mut properties = BTreeMap::new();
        properties.insert(
            "connection".to_string(),
            self.websocket_connection_schema_for_schema(context),
        );
        properties.insert(
            "message".to_string(),
            self.standard_library_connection_message_schema(&mut BTreeSet::new()),
        );
        JsonSchema::object(
            properties,
            vec!["connection".to_string(), "message".to_string()],
            false,
        )
    }

    fn schema_for_builtin(&self, name: &str, args: &[ContractTypeKey]) -> JsonSchema {
        match (name, args) {
            ("string", []) => JsonSchema::string(),
            ("integer", []) => JsonSchema::typed("integer"),
            ("number", []) => JsonSchema::typed("number"),
            ("bool" | "boolean", []) => JsonSchema::typed("boolean"),
            ("null" | "void", []) => JsonSchema::typed("null"),
            ("Date", []) => date_time_schema(),
            ("bytes", []) | ("Json", []) | ("JsonObject", []) => self
                .schema_for_standard_library_type(name, &mut BTreeSet::new())
                .unwrap_or_else(JsonSchema::any),
            ("Array", [inner]) => JsonSchema::array(self.schema_for_type_key_inner(inner)),
            ("Map", [key, value]) => self.schema_for_map(key, value),
            (root, [context]) if is_connect_result_generic_type(root) => self
                .schema_for_generic_standard_library_type(
                    root,
                    &[self.schema_for_type_key_inner(context)],
                )
                .unwrap_or_else(JsonSchema::any),
            (root, [context]) if is_websocket_connection_generic_type(root) => {
                self.websocket_connection_schema(context)
            }
            (root, [context]) if is_websocket_receive_event_generic_type(root) => {
                self.websocket_receive_event_schema(context)
            }
            (name, []) => self
                .schema_for_standard_library_type(name, &mut BTreeSet::new())
                .unwrap_or_else(JsonSchema::any),
            _ => JsonSchema::any(),
        }
    }

    fn schema_for_named_key(&self, name: &ContractNamedTypeKey) -> JsonSchema {
        match name {
            ContractNamedTypeKey::Public { symbol } => self.schema_for_public_symbol(symbol),
            ContractNamedTypeKey::Source { source } => {
                let symbol = projection_source_symbol_text(source);
                if self.types.contains_key(&symbol) || self.aliases.contains_key(&symbol) {
                    return self.schema_for_public_symbol(&symbol);
                }
                self.schema_for_standard_library_type(&symbol, &mut BTreeSet::new())
                    .unwrap_or_else(JsonSchema::any)
            }
        }
    }

    fn schema_for_public_symbol(&self, symbol: &str) -> JsonSchema {
        if let Some(ty) = self.types.get(symbol) {
            let schema = match &ty.descriptor {
                ContractTypeDescriptorProjection::Record { fields } => {
                    self.schema_object_for_fields(fields)
                }
                ContractTypeDescriptorProjection::Union { variants } => {
                    self.schema_for_union_with_discriminator(variants, ty.discriminator.as_deref())
                }
                ContractTypeDescriptorProjection::Native { .. } => JsonSchema::any(),
            };
            let mut schema = schema.with_x_skiff_symbol(symbol.to_string());
            if let Some(discriminator) = &ty.discriminator {
                schema = schema.with_x_skiff_union_discriminator(discriminator.clone());
            }
            return schema;
        }

        if let Some(alias) = self.aliases.get(symbol) {
            return self.schema_for_alias_projection(symbol, alias);
        }

        if let Some(schema) = self.schema_for_standard_library_type(symbol, &mut BTreeSet::new()) {
            return schema;
        }

        JsonSchema::reference(symbol)
    }

    fn schema_for_alias_projection(
        &self,
        symbol: &str,
        alias: &ContractAliasProjection,
    ) -> JsonSchema {
        if alias.transparent {
            return self.schema_for_type_key_with_discriminator(
                &alias.target,
                alias.discriminator.as_deref(),
            );
        }
        let mut schema = self
            .schema_for_type_key_with_discriminator(&alias.target, alias.discriminator.as_deref());
        schema = schema
            .with_x_skiff_symbol(symbol.to_string())
            .with_x_skiff_alias(contract_type_key_display_text(&alias.target));
        if let Some(discriminator) = &alias.discriminator {
            schema = schema.with_x_skiff_union_discriminator(discriminator.clone());
        }
        schema
    }

    fn schema_for_type_key_with_discriminator(
        &self,
        key: &ContractTypeKey,
        discriminator: Option<&str>,
    ) -> JsonSchema {
        let Some(discriminator) = discriminator else {
            return self.schema_for_type_key_inner(key);
        };
        match key {
            ContractTypeKey::Nullable { inner } => self
                .schema_for_type_key_with_discriminator(inner, Some(discriminator))
                .nullable(),
            ContractTypeKey::Union { items } => {
                self.schema_for_union_with_discriminator(items, Some(discriminator))
            }
            _ => self.schema_for_type_key_inner(key),
        }
    }

    fn schema_for_union_with_discriminator(
        &self,
        items: &[ContractTypeKey],
        discriminator: Option<&str>,
    ) -> JsonSchema {
        JsonSchema::one_of(
            items
                .iter()
                .map(|item| {
                    let mut schema = self.schema_for_type_key_inner(item);
                    if let Some(tag) = discriminator
                        .and_then(|field| contract_record_discriminator_value(item, field))
                    {
                        schema = schema.with_x_skiff_union_branch(tag);
                    }
                    if let Some(required) = discriminator
                        .and_then(|field| self.contract_record_required_fields(item, field))
                    {
                        schema = schema.with_required(required);
                    }
                    schema
                })
                .collect(),
        )
    }

    fn contract_record_required_fields(
        &self,
        key: &ContractTypeKey,
        discriminator: &str,
    ) -> Option<Vec<String>> {
        let ContractTypeKey::Record { fields } = key else {
            return None;
        };
        let mut required = Vec::new();
        if let Some(field) = fields.get(discriminator) {
            if !self.schema_for_type_key_inner(field).is_nullable() {
                required.push(discriminator.to_string());
            }
        }
        for (name, ty) in fields {
            if name == discriminator || self.schema_for_type_key_inner(ty).is_nullable() {
                continue;
            }
            required.push(name.clone());
        }
        (!required.is_empty()).then_some(required)
    }

    fn schema_for_standard_library_package_symbol(
        &self,
        package: &ContractPackageRefKey,
        symbol_path: &str,
    ) -> Option<JsonSchema> {
        let symbol = standard_library_package_symbol(self.prelude(), package, symbol_path)?;
        self.schema_for_standard_library_type(&symbol, &mut BTreeSet::new())
    }

    fn schema_for_standard_library_type(
        &self,
        ty: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        let prelude = self.prelude();
        let ty = ty.trim();
        if let Some(inner) = ty.strip_suffix('?') {
            return Some(
                self.schema_for_standard_library_type(inner, seen)?
                    .nullable(),
            );
        }
        let union = split_top_level(ty, '|');
        if union.len() > 1 {
            return Some(JsonSchema::one_of(
                union
                    .iter()
                    .map(|part| self.schema_for_standard_library_type(part, seen))
                    .collect::<Option<Vec<_>>>()?,
            ));
        }
        if let Some(parts) = generic_parts(ty) {
            match (parts.root, parts.args.as_slice()) {
                ("Array", [inner]) => {
                    return Some(JsonSchema::array(
                        self.schema_for_standard_library_type(inner, seen)?,
                    ));
                }
                ("Map", [_key, value]) => {
                    return Some(JsonSchema::map(
                        self.schema_for_standard_library_type(value, seen)?,
                    ));
                }
                _ => {}
            }
        }
        if let Some(fields) = record_type_fields(ty) {
            return Some(self.standard_library_record_schema(
                None,
                fields.iter().map(|field| (field.name, field.ty)).collect(),
                seen,
            )?);
        }
        if let Some(value) = string_literal(ty) {
            return Some(JsonSchema::string_enum(vec![value]));
        }
        match ty {
            "string" => return Some(JsonSchema::string()),
            "integer" => return Some(JsonSchema::typed("integer")),
            "number" => return Some(JsonSchema::typed("number")),
            "bool" | "boolean" => return Some(JsonSchema::typed("boolean")),
            "null" | "void" => return Some(JsonSchema::typed("null")),
            "Date" => return Some(date_time_schema()),
            _ => {}
        }

        let symbol = prelude.known_type_symbol(ty).or_else(|| {
            prelude
                .lowered_type_decl(ty)
                .map(|_| prelude.type_symbol(ty))
        })?;
        match symbol.as_str() {
            "std.bytes.bytes" => {
                return Some(with_prelude_schema_metadata(
                    JsonSchema::string().with_content_encoding("base64"),
                    &symbol,
                    prelude.schema_identity(),
                ));
            }
            "std.json.Json" => {
                return Some(with_prelude_schema_metadata(
                    JsonSchema::typed("json"),
                    &symbol,
                    prelude.schema_identity(),
                ));
            }
            "std.json.JsonObject" => {
                return Some(with_prelude_schema_metadata(
                    JsonSchema::typed("object").with_additional_properties(true),
                    &symbol,
                    prelude.schema_identity(),
                ));
            }
            _ => {}
        }

        let decl = prelude.lowered_type_decl(&symbol)?;
        if !seen.insert(symbol.clone()) {
            return Some(with_prelude_schema_metadata(
                JsonSchema::reference(&symbol),
                &symbol,
                prelude.schema_identity(),
            ));
        }

        let schema = self.standard_library_decl_schema(&symbol, &decl, &BTreeMap::new(), seen)?;
        seen.remove(&symbol);
        Some(schema)
    }

    fn schema_for_generic_standard_library_type(
        &self,
        name: &str,
        args: &[JsonSchema],
    ) -> Option<JsonSchema> {
        let prelude = self.prelude();
        let symbol = prelude.known_type_symbol(name)?;
        let decl = prelude.lowered_type_decl(&symbol)?;
        if decl.type_params.len() != args.len() {
            return None;
        }
        let substitutions = decl
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        self.standard_library_decl_schema(&symbol, &decl, &substitutions, &mut BTreeSet::new())
    }

    fn standard_library_connection_message_schema(
        &self,
        seen: &mut BTreeSet<String>,
    ) -> JsonSchema {
        self.schema_for_standard_library_type("std.websocket.ConnectionMessage", seen)
            .unwrap_or_else(JsonSchema::any)
    }

    fn standard_library_decl_schema(
        &self,
        symbol: &str,
        decl: &TypeDeclIr,
        substitutions: &BTreeMap<String, JsonSchema>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        match &decl.descriptor {
            TypeDescriptorIr::Record { fields } => self.standard_library_record_schema_from_ir(
                Some(symbol),
                fields,
                substitutions,
                seen,
            ),
            TypeDescriptorIr::Alias { target } => {
                let mut schema =
                    self.standard_library_type_ref_schema(target, substitutions, seen)?;
                schema =
                    with_prelude_schema_metadata(schema, symbol, self.prelude().schema_identity())
                        .with_x_skiff_alias(standard_library_type_ref_display_text(target));
                if let Some(discriminator) = &decl.discriminator {
                    schema = schema.with_x_skiff_union_discriminator(discriminator.clone());
                }
                Some(schema)
            }
            TypeDescriptorIr::Union { variants } => {
                let mut schema = with_prelude_schema_metadata(
                    self.standard_library_union_schema_from_ir(
                        variants,
                        decl.discriminator.as_deref(),
                        substitutions,
                        seen,
                    )?,
                    symbol,
                    self.prelude().schema_identity(),
                );
                if let Some(discriminator) = &decl.discriminator {
                    schema = schema.with_x_skiff_union_discriminator(discriminator.clone());
                }
                Some(schema)
            }
            TypeDescriptorIr::Native { .. } => None,
        }
    }

    fn standard_library_type_ref_schema(
        &self,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, JsonSchema>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        match ty {
            TypeRefIr::Native { name, args } => {
                let arg_schemas = args
                    .iter()
                    .map(|arg| self.standard_library_type_ref_schema(arg, substitutions, seen))
                    .collect::<Option<Vec<_>>>()?;
                match name.as_str() {
                    "string" => Some(JsonSchema::string()),
                    "integer" => Some(JsonSchema::typed("integer")),
                    "number" => Some(JsonSchema::typed("number")),
                    "bool" | "boolean" => Some(JsonSchema::typed("boolean")),
                    "null" | "void" => Some(JsonSchema::typed("null")),
                    "Date" => Some(date_time_schema()),
                    "Array" if arg_schemas.len() == 1 => {
                        Some(JsonSchema::array(arg_schemas[0].clone()))
                    }
                    "Map" if arg_schemas.len() == 2 => {
                        Some(JsonSchema::map(arg_schemas[1].clone()))
                    }
                    _ if arg_schemas.is_empty() => {
                        self.schema_for_standard_library_type(name, seen)
                    }
                    _ => self.schema_for_generic_standard_library_type(name, &arg_schemas),
                }
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let symbol_path = symbol.symbol_path.as_str();
                if symbol_path == "std.bytes.bytes" {
                    return Some(with_prelude_schema_metadata(
                        JsonSchema::string().with_content_encoding("base64"),
                        symbol_path,
                        self.prelude().schema_identity(),
                    ));
                }
                self.schema_for_standard_library_type(symbol_path, seen)
            }
            TypeRefIr::Record { fields } => {
                self.standard_library_record_schema_from_ir(None, fields, substitutions, seen)
            }
            TypeRefIr::Union { items } => {
                self.standard_library_union_schema_from_ir(items, None, substitutions, seen)
            }
            TypeRefIr::Nullable { inner } => Some(
                self.standard_library_type_ref_schema(inner, substitutions, seen)?
                    .nullable(),
            ),
            TypeRefIr::Literal { value } => Some(self.schema_for_source_literal(value)),
            TypeRefIr::TypeParam { name } => substitutions.get(name).cloned(),
            TypeRefIr::AnyInterface { .. } => Some(boundary_rejected_type_schema()),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Function { .. } => Some(JsonSchema::any()),
        }
    }

    fn standard_library_union_schema_from_ir(
        &self,
        items: &[TypeRefIr],
        discriminator: Option<&str>,
        substitutions: &BTreeMap<String, JsonSchema>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        if let Some(schema) = source_string_literal_union_schema(items) {
            return Some(schema);
        }
        Some(JsonSchema::one_of(
            items
                .iter()
                .map(|item| {
                    let mut schema =
                        self.standard_library_type_ref_schema(item, substitutions, seen)?;
                    if let Some(tag) = discriminator
                        .and_then(|field| source_record_discriminator_value(item, field))
                    {
                        schema = schema.with_x_skiff_union_branch(tag);
                    }
                    Some(schema)
                })
                .collect::<Option<Vec<_>>>()?,
        ))
    }

    fn standard_library_record_schema_from_ir(
        &self,
        symbol: Option<&str>,
        fields: &BTreeMap<String, TypeRefIr>,
        substitutions: &BTreeMap<String, JsonSchema>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();
        for (field_name, field_type) in fields {
            let field_schema =
                self.standard_library_type_ref_schema(field_type, substitutions, seen)?;
            if !field_schema.is_nullable() {
                required.push(field_name.to_string());
            }
            properties.insert(field_name.to_string(), field_schema);
        }
        let schema = JsonSchema::object(properties, required, false);
        Some(match symbol {
            Some(symbol) => {
                with_prelude_schema_metadata(schema, symbol, self.prelude().schema_identity())
            }
            None => schema,
        })
    }

    fn standard_library_alias_schema(
        &self,
        symbol: &str,
        alias: &str,
        discriminator: Option<&str>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        let union = split_top_level(alias, '|');
        if union.len() > 1 {
            let mut schema = with_prelude_schema_metadata(
                JsonSchema::one_of(
                    union
                        .iter()
                        .map(|part| {
                            self.standard_library_union_branch_schema(
                                symbol,
                                part,
                                discriminator,
                                seen,
                            )
                        })
                        .collect::<Option<Vec<_>>>()?,
                ),
                symbol,
                self.prelude().schema_identity(),
            );
            if let Some(discriminator) = discriminator {
                schema = schema.with_x_skiff_union_discriminator(discriminator.to_string());
            }
            return Some(schema);
        }

        if let Some(fields) = record_type_fields(alias) {
            return self.standard_library_record_schema(
                Some(symbol),
                fields.iter().map(|field| (field.name, field.ty)).collect(),
                seen,
            );
        }

        Some(
            with_prelude_schema_metadata(
                self.schema_for_standard_library_type(alias, seen)?,
                symbol,
                self.prelude().schema_identity(),
            )
            .with_x_skiff_alias(alias.to_string()),
        )
    }

    fn standard_library_union_branch_schema(
        &self,
        symbol: &str,
        branch: &str,
        discriminator: Option<&str>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        let Some(fields) = record_type_fields(branch) else {
            return self.schema_for_standard_library_type(branch, seen);
        };
        let tag = discriminator.and_then(|field| {
            fields.iter().find_map(|record_field| {
                (record_field.name == field)
                    .then(|| string_literal(record_field.ty))
                    .flatten()
            })
        });
        let mut schema = self.standard_library_record_schema(
            Some(symbol),
            fields.iter().map(|field| (field.name, field.ty)).collect(),
            seen,
        )?;
        if let Some(tag) = tag {
            schema = schema.with_x_skiff_union_branch(tag);
        }
        Some(schema)
    }

    fn standard_library_record_schema(
        &self,
        symbol: Option<&str>,
        fields: Vec<(&str, &str)>,
        seen: &mut BTreeSet<String>,
    ) -> Option<JsonSchema> {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();
        for (field_name, field_type) in fields {
            let field_schema = self.schema_for_standard_library_type(field_type, seen)?;
            if !field_schema.is_nullable() {
                required.push(field_name.to_string());
            }
            properties.insert(field_name.to_string(), field_schema);
        }
        let schema = JsonSchema::object(properties, required, false);
        Some(match symbol {
            Some(symbol) => {
                with_prelude_schema_metadata(schema, symbol, self.prelude().schema_identity())
            }
            None => schema,
        })
    }

    fn schema_for_union(&self, items: &[ContractTypeKey]) -> JsonSchema {
        if let Some(schema) = self.string_literal_union_schema(items) {
            return schema;
        }
        if is_transport_message_union(items) {
            return self.standard_library_connection_message_schema(&mut BTreeSet::new());
        }
        JsonSchema::one_of(
            items
                .iter()
                .map(|item| self.schema_for_type_key_inner(item))
                .collect(),
        )
    }

    fn string_literal_union_schema(&self, items: &[ContractTypeKey]) -> Option<JsonSchema> {
        let values = items
            .iter()
            .map(|item| match item {
                ContractTypeKey::Literal(ContractLiteralKey::String(value)) => Some(value.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(JsonSchema::string_enum(values))
    }

    fn schema_for_literal(&self, literal: &ContractLiteralKey) -> JsonSchema {
        match literal {
            ContractLiteralKey::Null => JsonSchema::typed("null"),
            ContractLiteralKey::Bool(_) => JsonSchema::typed("boolean"),
            ContractLiteralKey::Number(_) => JsonSchema::typed("number"),
            ContractLiteralKey::String(value) => JsonSchema::string_enum(vec![value.clone()]),
        }
    }

    fn schema_object_for_fields(&self, fields: &BTreeMap<String, ContractTypeKey>) -> JsonSchema {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();
        for (name, ty) in fields {
            let field_schema = self.schema_for_type_key_inner(ty);
            if !field_schema.is_nullable() {
                required.push(name.clone());
            }
            properties.insert(name.clone(), field_schema);
        }
        JsonSchema::object(properties, required, false)
    }

    fn schema_for_map(&self, key: &ContractTypeKey, value: &ContractTypeKey) -> JsonSchema {
        if !self.is_schema_map_key_type(key) {
            return JsonSchema::any();
        }
        let mut schema = JsonSchema::map(self.schema_for_type_key_inner(value));
        if let Some(symbol) = self.map_key_symbol(key) {
            schema = schema
                .with_x_skiff_map_key_symbol(symbol)
                .with_x_skiff_map_key_schema(self.schema_for_map_key_metadata(key));
        }
        schema
    }

    fn is_schema_map_key_type(&self, key: &ContractTypeKey) -> bool {
        if matches!(key, ContractTypeKey::Builtin { name, args } if args.is_empty() && name == "string")
        {
            return true;
        }
        self.map_key_representation_target(key).is_some()
    }

    fn schema_for_map_key_metadata(&self, key: &ContractTypeKey) -> JsonSchema {
        let Some(symbol) = self.map_key_symbol(key) else {
            return self.schema_for_type_key_inner(key);
        };
        let mut schema = self
            .map_key_representation_target(key)
            .unwrap_or_else(|| self.schema_for_type_key_inner(key));
        schema = schema.with_x_skiff_symbol(symbol.clone());
        if let Some(alias) = self.aliases.get(&symbol) {
            schema = schema.with_x_skiff_alias(contract_type_key_display_text(&alias.target));
        }
        schema
    }

    fn map_key_symbol(&self, key: &ContractTypeKey) -> Option<String> {
        match key {
            ContractTypeKey::Named(ContractNamedTypeKey::Public { symbol }) => Some(symbol.clone()),
            ContractTypeKey::Named(ContractNamedTypeKey::Source { source }) => {
                Some(projection_source_symbol_text(source))
            }
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => standard_library_package_symbol(self.prelude(), package, symbol_path),
            _ => None,
        }
    }

    fn map_key_representation_target(&self, key: &ContractTypeKey) -> Option<JsonSchema> {
        match key {
            ContractTypeKey::Named(name) => {
                let symbol = name.canonical_symbol();
                if let Some(alias) = self.aliases.get(&symbol) {
                    return self.map_key_representation_target(&alias.target);
                }
                if let Some(ContractTypeProjection {
                    descriptor: ContractTypeDescriptorProjection::Union { variants },
                    ..
                }) = self.types.get(&symbol)
                {
                    return string_map_key_union_schema(variants);
                }
                None
            }
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => {
                let symbol = standard_library_package_symbol(self.prelude(), package, symbol_path)?;
                self.schema_for_standard_library_type(&symbol, &mut BTreeSet::new())
                    .filter(schema_is_string_like)
            }
            ContractTypeKey::Builtin { name, args } if args.is_empty() && name == "string" => {
                Some(JsonSchema::string())
            }
            ContractTypeKey::Literal(ContractLiteralKey::String(value)) => {
                Some(JsonSchema::string_enum(vec![value.clone()]))
            }
            ContractTypeKey::Union { items } => string_map_key_union_schema(items),
            _ => None,
        }
    }

    fn websocket_connection_schema(&self, context: &ContractTypeKey) -> JsonSchema {
        let mut properties = BTreeMap::new();
        properties.insert("id".to_string(), JsonSchema::string());
        properties.insert(
            "businessIdentity".to_string(),
            JsonSchema::string().nullable(),
        );
        properties.insert(
            "context".to_string(),
            self.schema_for_type_key_inner(context),
        );
        JsonSchema::object(
            properties,
            vec![
                "id".to_string(),
                "businessIdentity".to_string(),
                "context".to_string(),
            ],
            false,
        )
    }

    fn websocket_receive_event_schema(&self, context: &ContractTypeKey) -> JsonSchema {
        let mut properties = BTreeMap::new();
        properties.insert(
            "connection".to_string(),
            self.websocket_connection_schema(context),
        );
        properties.insert(
            "message".to_string(),
            self.standard_library_connection_message_schema(&mut BTreeSet::new()),
        );
        JsonSchema::object(
            properties,
            vec!["connection".to_string(), "message".to_string()],
            false,
        )
    }

    fn runtime_descriptor_for_type_key_inner(
        &self,
        key: &ContractTypeKey,
    ) -> RuntimeTypeDescriptorIr {
        match key {
            ContractTypeKey::Builtin { name, args } => {
                self.runtime_descriptor_for_builtin_key(name, args)
            }
            ContractTypeKey::Named(name) => {
                let symbol = name.canonical_symbol_ref();
                self.runtime_descriptor_for_named_symbol(symbol.as_ref())
            }
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => {
                let symbol = standard_library_package_symbol(self.prelude(), package, symbol_path)
                    .unwrap_or_else(|| symbol_path.clone());
                self.runtime_descriptor_for_standard_library_symbol(&symbol)
                    .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(symbol))
            }
            ContractTypeKey::AnyInterface { .. } => boundary_rejected_runtime_type_descriptor(),
            ContractTypeKey::DbObjectSymbol {
                module_path,
                symbol,
            } => RuntimeTypeDescriptorIr::named(format!("{module_path}.{symbol}")),
            ContractTypeKey::Record { fields } => RuntimeTypeDescriptorIr::Record {
                fields: self.runtime_descriptor_field_map(fields),
            },
            ContractTypeKey::Union { items } => RuntimeTypeDescriptorIr::Union {
                items: items
                    .iter()
                    .map(|item| self.runtime_descriptor_for_type_key_inner(item))
                    .collect(),
            },
            ContractTypeKey::Nullable { inner } => {
                RuntimeTypeDescriptorIr::nullable(self.runtime_descriptor_for_type_key_inner(inner))
            }
            ContractTypeKey::Literal(ContractLiteralKey::String(value)) => {
                RuntimeTypeDescriptorIr::literal_string(value)
            }
            ContractTypeKey::Literal(ContractLiteralKey::Null) => {
                RuntimeTypeDescriptorIr::named("null")
            }
            ContractTypeKey::Literal(ContractLiteralKey::Bool(value)) => {
                RuntimeTypeDescriptorIr::named(value.to_string())
            }
            ContractTypeKey::Literal(ContractLiteralKey::Number(value)) => {
                RuntimeTypeDescriptorIr::named(value)
            }
            ContractTypeKey::TypeParam { name } => RuntimeTypeDescriptorIr::named(name),
            ContractTypeKey::Function { .. } => RuntimeTypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
        }
    }

    fn runtime_descriptor_for_source_type_ref_inner(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> RuntimeTypeDescriptorIr {
        match ty {
            TypeRefIr::Native { name, args } => {
                self.runtime_descriptor_for_source_builtin(index, module_path, name, args, seen)
            }
            TypeRefIr::LocalType { type_index } => index
                .type_binding_by_module_type_index(module_path, *type_index)
                .map(|binding| {
                    self.runtime_descriptor_for_source_type_binding(index, binding, seen)
                })
                .unwrap_or_else(|| {
                    RuntimeTypeDescriptorIr::named(format!(
                        "{module_path}.__invalid_local_type_{type_index}"
                    ))
                }),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => index
                .type_binding_by_module_type_index(module_path, *type_index)
                .map(|binding| {
                    self.runtime_descriptor_for_source_type_binding(index, binding, seen)
                })
                .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(format!("root.{module_path}"))),
            TypeRefIr::ServiceSymbol { symbol } => {
                if let Some(source_key) =
                    index.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                {
                    if let Some(binding) = index.type_binding_by_source_key(&source_key) {
                        return self
                            .runtime_descriptor_for_source_type_binding(index, binding, seen);
                    }
                }
                let source_module = index.source_module_for_reference_module(&symbol.module_path);
                if let Some(binding) =
                    index.type_binding_by_module_local_name(source_module, &symbol.symbol)
                {
                    return self.runtime_descriptor_for_source_type_binding(index, binding, seen);
                }
                self.runtime_descriptor_for_source_type_key_fallback(index, module_path, ty, seen)
            }
            TypeRefIr::PackageSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
                self.runtime_descriptor_for_source_type_key_fallback(index, module_path, ty, seen)
            }
            TypeRefIr::Record { fields } => RuntimeTypeDescriptorIr::Record {
                fields: self.runtime_descriptor_for_source_fields(index, module_path, fields, seen),
            },
            TypeRefIr::Union { items } => RuntimeTypeDescriptorIr::Union {
                items: items
                    .iter()
                    .map(|item| {
                        self.runtime_descriptor_for_source_type_ref_inner(
                            index,
                            module_path,
                            item,
                            seen,
                        )
                    })
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => RuntimeTypeDescriptorIr::nullable(
                self.runtime_descriptor_for_source_type_ref_inner(index, module_path, inner, seen),
            ),
            TypeRefIr::Literal { value } => RuntimeTypeDescriptorIr::Literal {
                value: value.clone(),
            },
            TypeRefIr::AnyInterface { .. } => boundary_rejected_runtime_type_descriptor(),
            TypeRefIr::TypeParam { name } => RuntimeTypeDescriptorIr::named(name),
            TypeRefIr::Function { .. } => RuntimeTypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
        }
    }

    fn runtime_descriptor_for_source_type_key_fallback(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> RuntimeTypeDescriptorIr {
        match index.canonical_type_ref(module_path, ty) {
            Ok(key) => self.runtime_descriptor_for_source_type_key_inner(index, &key, seen),
            Err(_) => {
                RuntimeTypeDescriptorIr::named(source_type_ref_display_text(index, module_path, ty))
            }
        }
    }

    fn runtime_descriptor_for_source_type_key_inner(
        &self,
        index: &ContractProjectionIndex<'_>,
        key: &ContractTypeKey,
        seen: &mut BTreeSet<String>,
    ) -> RuntimeTypeDescriptorIr {
        match key {
            ContractTypeKey::Builtin { name, args } => RuntimeTypeDescriptorIr::Builtin {
                name: name.clone(),
                args: (!args.is_empty()).then(|| {
                    args.iter()
                        .map(|arg| {
                            self.runtime_descriptor_for_source_type_key_inner(index, arg, seen)
                        })
                        .collect()
                }),
                fields: None,
            },
            ContractTypeKey::Named(ContractNamedTypeKey::Public { symbol }) => {
                self.runtime_descriptor_for_named_symbol(symbol)
            }
            ContractTypeKey::Named(ContractNamedTypeKey::Source { source }) => index
                .type_binding_by_source_key(source)
                .map(|binding| {
                    self.runtime_descriptor_for_source_type_binding(index, binding, seen)
                })
                .unwrap_or_else(|| {
                    RuntimeTypeDescriptorIr::named(projection_source_symbol_text(source))
                }),
            ContractTypeKey::PackageSymbol {
                package,
                symbol_path,
                ..
            } => {
                let symbol = standard_library_package_symbol(self.prelude(), package, symbol_path)
                    .unwrap_or_else(|| symbol_path.clone());
                self.runtime_descriptor_for_standard_library_symbol(&symbol)
                    .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(symbol))
            }
            ContractTypeKey::AnyInterface { .. } => boundary_rejected_runtime_type_descriptor(),
            ContractTypeKey::DbObjectSymbol {
                module_path,
                symbol,
            } => RuntimeTypeDescriptorIr::named(format!("{module_path}.{symbol}")),
            ContractTypeKey::Record { fields } => RuntimeTypeDescriptorIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            self.runtime_descriptor_for_source_type_key_inner(index, ty, seen),
                        )
                    })
                    .collect(),
            },
            ContractTypeKey::Union { items } => RuntimeTypeDescriptorIr::Union {
                items: items
                    .iter()
                    .map(|item| {
                        self.runtime_descriptor_for_source_type_key_inner(index, item, seen)
                    })
                    .collect(),
            },
            ContractTypeKey::Nullable { inner } => RuntimeTypeDescriptorIr::nullable(
                self.runtime_descriptor_for_source_type_key_inner(index, inner, seen),
            ),
            ContractTypeKey::Literal(ContractLiteralKey::String(value)) => {
                RuntimeTypeDescriptorIr::literal_string(value)
            }
            ContractTypeKey::Literal(ContractLiteralKey::Null) => {
                RuntimeTypeDescriptorIr::named("null")
            }
            ContractTypeKey::Literal(ContractLiteralKey::Bool(value)) => {
                RuntimeTypeDescriptorIr::named(value.to_string())
            }
            ContractTypeKey::Literal(ContractLiteralKey::Number(value)) => {
                RuntimeTypeDescriptorIr::named(value)
            }
            ContractTypeKey::TypeParam { name } => RuntimeTypeDescriptorIr::named(name),
            ContractTypeKey::Function { .. } => RuntimeTypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
        }
    }

    fn runtime_descriptor_for_source_builtin(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        name: &str,
        args: &[TypeRefIr],
        seen: &mut BTreeSet<String>,
    ) -> RuntimeTypeDescriptorIr {
        match (name, args) {
            ("Array", [inner]) => RuntimeTypeDescriptorIr::Builtin {
                name: "Array".to_string(),
                args: Some(vec![self.runtime_descriptor_for_source_type_ref_inner(
                    index,
                    module_path,
                    inner,
                    seen,
                )]),
                fields: None,
            },
            ("Map", [key, value]) => RuntimeTypeDescriptorIr::Builtin {
                name: "Map".to_string(),
                args: Some(vec![
                    self.runtime_descriptor_for_source_type_ref_inner(
                        index,
                        module_path,
                        key,
                        seen,
                    ),
                    self.runtime_descriptor_for_source_type_ref_inner(
                        index,
                        module_path,
                        value,
                        seen,
                    ),
                ]),
                fields: None,
            },
            (root, [context]) if is_connect_result_generic_type(root) => self
                .runtime_descriptor_for_generic_standard_library_type(
                    root,
                    &[self.runtime_descriptor_for_source_type_ref_inner(
                        index,
                        module_path,
                        context,
                        seen,
                    )],
                )
                .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(root)),
            (root, [context]) if is_websocket_connection_generic_type(root) => self
                .websocket_connection_runtime_descriptor(
                    self.runtime_descriptor_for_source_type_ref_inner(
                        index,
                        module_path,
                        context,
                        seen,
                    ),
                ),
            (root, [context]) if is_websocket_receive_event_generic_type(root) => self
                .websocket_receive_event_runtime_descriptor(
                    self.runtime_descriptor_for_source_type_ref_inner(
                        index,
                        module_path,
                        context,
                        seen,
                    ),
                ),
            (name, []) => self
                .runtime_descriptor_for_standard_library_symbol(name)
                .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(name)),
            _ => RuntimeTypeDescriptorIr::named(name),
        }
    }

    fn runtime_descriptor_for_source_type_binding(
        &self,
        index: &ContractProjectionIndex<'_>,
        binding: ContractProjectionTypeBinding<'_>,
        seen: &mut BTreeSet<String>,
    ) -> RuntimeTypeDescriptorIr {
        let source_key = ProjectionSourceSymbolKey::new(binding.module_path, binding.local_name);
        let source_symbol = projection_source_symbol_text(&source_key);
        if let Some(public_symbol) = self.public_schema_symbol_for_source(index, &source_key) {
            return self.runtime_descriptor_for_named_symbol(&public_symbol);
        }
        if !seen.insert(source_symbol.clone()) {
            return RuntimeTypeDescriptorIr::named(source_symbol);
        }

        let descriptor = match binding.descriptor() {
            TypeDescriptorIr::Record { fields } => RuntimeTypeDescriptorIr::named_with_fields(
                &source_symbol,
                self.runtime_descriptor_for_source_fields(index, binding.module_path, fields, seen),
            ),
            TypeDescriptorIr::Alias { target } => RuntimeTypeDescriptorIr::Representation {
                name: source_symbol.clone(),
                representation: Box::new(self.runtime_descriptor_for_source_type_ref_inner(
                    index,
                    binding.module_path,
                    target,
                    seen,
                )),
                discriminator: binding.discriminator().map(str::to_string),
            },
            TypeDescriptorIr::Union { variants } => RuntimeTypeDescriptorIr::Representation {
                name: source_symbol.clone(),
                representation: Box::new(RuntimeTypeDescriptorIr::Union {
                    items: variants
                        .iter()
                        .map(|variant| {
                            self.runtime_descriptor_for_source_type_ref_inner(
                                index,
                                binding.module_path,
                                variant,
                                seen,
                            )
                        })
                        .collect(),
                }),
                discriminator: binding.discriminator().map(str::to_string),
            },
            TypeDescriptorIr::Native { .. } => RuntimeTypeDescriptorIr::named(&source_symbol),
        };
        seen.remove(&source_symbol);
        descriptor
    }

    fn runtime_descriptor_for_source_fields(
        &self,
        index: &ContractProjectionIndex<'_>,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        seen: &mut BTreeSet<String>,
    ) -> BTreeMap<String, RuntimeTypeDescriptorIr> {
        fields
            .iter()
            .map(|(name, ty)| {
                (
                    name.clone(),
                    self.runtime_descriptor_for_source_type_ref_inner(index, module_path, ty, seen),
                )
            })
            .collect()
    }

    fn runtime_descriptor_for_builtin_key(
        &self,
        name: &str,
        args: &[ContractTypeKey],
    ) -> RuntimeTypeDescriptorIr {
        match (name, args) {
            ("Array", [inner]) => RuntimeTypeDescriptorIr::Builtin {
                name: "Array".to_string(),
                args: Some(vec![self.runtime_descriptor_for_type_key_inner(inner)]),
                fields: None,
            },
            ("Map", [key, value]) => RuntimeTypeDescriptorIr::Builtin {
                name: "Map".to_string(),
                args: Some(vec![
                    self.runtime_descriptor_for_type_key_inner(key),
                    self.runtime_descriptor_for_type_key_inner(value),
                ]),
                fields: None,
            },
            (root, [context]) if is_connect_result_generic_type(root) => self
                .runtime_descriptor_for_generic_standard_library_type(
                    root,
                    &[self.runtime_descriptor_for_type_key_inner(context)],
                )
                .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(root)),
            (root, [context]) if is_websocket_connection_generic_type(root) => self
                .websocket_connection_runtime_descriptor(
                    self.runtime_descriptor_for_type_key_inner(context),
                ),
            (root, [context]) if is_websocket_receive_event_generic_type(root) => self
                .websocket_receive_event_runtime_descriptor(
                    self.runtime_descriptor_for_type_key_inner(context),
                ),
            (name, []) => self
                .runtime_descriptor_for_standard_library_symbol(name)
                .unwrap_or_else(|| RuntimeTypeDescriptorIr::named(name)),
            _ => RuntimeTypeDescriptorIr::named(name),
        }
    }

    fn runtime_descriptor_for_named_symbol(&self, symbol: &str) -> RuntimeTypeDescriptorIr {
        if let Some(descriptor) = self.runtime_descriptor_for_standard_library_symbol(symbol) {
            return descriptor;
        }
        if let Some(alias) = self.aliases.get(symbol) {
            if alias.transparent {
                return self.runtime_descriptor_for_type_key_inner(&alias.target);
            }
            return RuntimeTypeDescriptorIr::Representation {
                name: symbol.to_string(),
                representation: Box::new(self.runtime_descriptor_for_type_key_inner(&alias.target)),
                discriminator: alias.discriminator.clone(),
            };
        }
        if let Some(ty) = self.types.get(symbol) {
            match &ty.descriptor {
                ContractTypeDescriptorProjection::Record { fields } => {
                    return RuntimeTypeDescriptorIr::named_with_fields(
                        symbol,
                        self.runtime_descriptor_field_map(fields),
                    );
                }
                ContractTypeDescriptorProjection::Union { variants } => {
                    return RuntimeTypeDescriptorIr::Representation {
                        name: symbol.to_string(),
                        representation: Box::new(RuntimeTypeDescriptorIr::Union {
                            items: variants
                                .iter()
                                .map(|variant| self.runtime_descriptor_for_type_key_inner(variant))
                                .collect(),
                        }),
                        discriminator: ty.discriminator.clone(),
                    };
                }
                ContractTypeDescriptorProjection::Native { .. } => {}
            }
        }
        RuntimeTypeDescriptorIr::named(symbol)
    }

    fn runtime_descriptor_field_map(
        &self,
        fields: &BTreeMap<String, ContractTypeKey>,
    ) -> BTreeMap<String, RuntimeTypeDescriptorIr> {
        fields
            .iter()
            .map(|(name, ty)| (name.clone(), self.runtime_descriptor_for_type_key_inner(ty)))
            .collect()
    }

    fn websocket_connection_runtime_descriptor(
        &self,
        context_type: RuntimeTypeDescriptorIr,
    ) -> RuntimeTypeDescriptorIr {
        let mut fields = BTreeMap::new();
        fields.insert("id".to_string(), RuntimeTypeDescriptorIr::named("string"));
        fields.insert(
            "businessIdentity".to_string(),
            RuntimeTypeDescriptorIr::nullable(RuntimeTypeDescriptorIr::named("string")),
        );
        fields.insert("context".to_string(), context_type);
        RuntimeTypeDescriptorIr::named_with_fields("std.websocket.WebSocketConnection", fields)
    }

    fn websocket_receive_event_runtime_descriptor(
        &self,
        context_type: RuntimeTypeDescriptorIr,
    ) -> RuntimeTypeDescriptorIr {
        let mut fields = BTreeMap::new();
        fields.insert(
            "connection".to_string(),
            self.websocket_connection_runtime_descriptor(context_type),
        );
        fields.insert(
            "message".to_string(),
            self.runtime_descriptor_for_standard_library_symbol("std.websocket.ConnectionMessage")
                .unwrap_or_else(|| {
                    RuntimeTypeDescriptorIr::named("std.websocket.ConnectionMessage")
                }),
        );
        RuntimeTypeDescriptorIr::named_with_fields("std.websocket.WebSocketReceiveEvent", fields)
    }

    fn runtime_descriptor_for_standard_library_symbol(
        &self,
        name: &str,
    ) -> Option<RuntimeTypeDescriptorIr> {
        let prelude = self.prelude();
        let symbol = prelude.known_type_symbol(name)?;
        let decl = prelude.lowered_type_decl(&symbol)?;
        self.runtime_descriptor_for_standard_library_decl(&symbol, &decl, &BTreeMap::new())
    }

    fn runtime_descriptor_for_generic_standard_library_type(
        &self,
        name: &str,
        args: &[RuntimeTypeDescriptorIr],
    ) -> Option<RuntimeTypeDescriptorIr> {
        let prelude = self.prelude();
        let symbol = prelude.known_type_symbol(name)?;
        let decl = prelude.lowered_type_decl(&symbol)?;
        if decl.type_params.len() != args.len() {
            return None;
        }
        let substitutions = decl
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        self.runtime_descriptor_for_standard_library_decl(&symbol, &decl, &substitutions)
    }

    fn runtime_descriptor_for_standard_library_decl(
        &self,
        symbol: &str,
        decl: &TypeDeclIr,
        substitutions: &BTreeMap<String, RuntimeTypeDescriptorIr>,
    ) -> Option<RuntimeTypeDescriptorIr> {
        match &decl.descriptor {
            TypeDescriptorIr::Record { fields } => {
                Some(RuntimeTypeDescriptorIr::named_with_fields(
                    symbol,
                    self.runtime_descriptor_for_standard_library_fields(fields, substitutions),
                ))
            }
            TypeDescriptorIr::Alias { target } => Some(RuntimeTypeDescriptorIr::Representation {
                name: symbol.to_string(),
                representation: Box::new(
                    self.runtime_descriptor_for_standard_library_type_ref(target, substitutions)?,
                ),
                discriminator: decl.discriminator.clone(),
            }),
            TypeDescriptorIr::Union { variants } => Some(RuntimeTypeDescriptorIr::Representation {
                name: symbol.to_string(),
                representation: Box::new(RuntimeTypeDescriptorIr::Union {
                    items: variants
                        .iter()
                        .map(|variant| {
                            self.runtime_descriptor_for_standard_library_type_ref(
                                variant,
                                substitutions,
                            )
                        })
                        .collect::<Option<Vec<_>>>()?,
                }),
                discriminator: decl.discriminator.clone(),
            }),
            TypeDescriptorIr::Native { .. } => None,
        }
    }

    fn runtime_descriptor_for_standard_library_type_ref(
        &self,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, RuntimeTypeDescriptorIr>,
    ) -> Option<RuntimeTypeDescriptorIr> {
        match ty {
            TypeRefIr::Native { name, args } if args.is_empty() => {
                Some(RuntimeTypeDescriptorIr::named(name))
            }
            TypeRefIr::Native { name, args } => Some(RuntimeTypeDescriptorIr::Builtin {
                name: name.clone(),
                args: Some(
                    args.iter()
                        .map(|arg| {
                            self.runtime_descriptor_for_standard_library_type_ref(
                                arg,
                                substitutions,
                            )
                        })
                        .collect::<Option<Vec<_>>>()?,
                ),
                fields: None,
            }),
            TypeRefIr::PackageSymbol { symbol } => {
                self.runtime_descriptor_for_standard_library_symbol(&symbol.symbol_path)
            }
            TypeRefIr::Record { fields } => Some(RuntimeTypeDescriptorIr::Record {
                fields: self.runtime_descriptor_for_standard_library_fields(fields, substitutions),
            }),
            TypeRefIr::Union { items } => Some(RuntimeTypeDescriptorIr::Union {
                items: items
                    .iter()
                    .map(|item| {
                        self.runtime_descriptor_for_standard_library_type_ref(item, substitutions)
                    })
                    .collect::<Option<Vec<_>>>()?,
            }),
            TypeRefIr::Nullable { inner } => Some(RuntimeTypeDescriptorIr::nullable(
                self.runtime_descriptor_for_standard_library_type_ref(inner, substitutions)?,
            )),
            TypeRefIr::Literal { value } => Some(RuntimeTypeDescriptorIr::Literal {
                value: value.clone(),
            }),
            TypeRefIr::TypeParam { name } => substitutions.get(name).cloned(),
            TypeRefIr::AnyInterface { .. } => Some(boundary_rejected_runtime_type_descriptor()),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Function { .. } => Some(RuntimeTypeDescriptorIr::named(
                standard_library_type_ref_display_text(ty),
            )),
        }
    }

    fn runtime_descriptor_for_standard_library_fields(
        &self,
        fields: &BTreeMap<String, TypeRefIr>,
        substitutions: &BTreeMap<String, RuntimeTypeDescriptorIr>,
    ) -> BTreeMap<String, RuntimeTypeDescriptorIr> {
        fields
            .iter()
            .filter_map(|(name, ty)| {
                Some((
                    name.clone(),
                    self.runtime_descriptor_for_standard_library_type_ref(ty, substitutions)?,
                ))
            })
            .collect()
    }
}

pub fn canonical_contract_projection_schema_json(projection: &ContractProjection) -> String {
    super::canonical_contract_projection_schema(projection).canonical_json()
}

impl ContractProjection {
    fn public_schema_symbol_for_source(
        &self,
        index: &ContractProjectionIndex<'_>,
        source_key: &ProjectionSourceSymbolKey,
    ) -> Option<String> {
        let public_symbol = index.public_symbol_for_source_key(source_key)?;
        (self.types.contains_key(&public_symbol) || self.aliases.contains_key(&public_symbol))
            .then_some(public_symbol)
    }
}

fn source_string_literal_union_schema(items: &[TypeRefIr]) -> Option<JsonSchema> {
    let values = items
        .iter()
        .map(|item| match item {
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => Some(value.clone()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(JsonSchema::string_enum(values))
}

fn source_record_discriminator_value(ty: &TypeRefIr, discriminator: &str) -> Option<String> {
    let TypeRefIr::Record { fields } = ty else {
        return None;
    };
    fields.get(discriminator).and_then(|field| match field {
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => Some(value.clone()),
        _ => None,
    })
}

fn source_type_ref_display_text(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    ty: &TypeRefIr,
) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(|arg| source_type_ref_display_text(index, module_path, arg))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => index
            .type_binding_by_module_type_index(module_path, *type_index)
            .map(|binding| binding.local_name.to_string())
            .unwrap_or_else(|| format!("__invalid_local_type_{type_index}")),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => index
            .type_binding_by_module_type_index(module_path, *type_index)
            .map(|binding| binding.local_name.to_string())
            .unwrap_or_else(|| format!("root.{module_path}")),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            if symbol.module_path.is_empty() {
                symbol.symbol.clone()
            } else {
                symbol.symbol_path()
            }
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        source_type_ref_display_text(index, module_path, ty)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Union { items } => items
            .iter()
            .map(|item| source_type_ref_display_text(index, module_path, item))
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => {
            format!(
                "{}?",
                source_type_ref_display_text(index, module_path, inner)
            )
        }
        TypeRefIr::AnyInterface { interface } => {
            source_any_interface_display_text(index, module_path, interface)
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => {
                serde_json::to_string(value).expect("string literal should serialize")
            }
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| format!(
                    "{}: {}",
                    param.name,
                    source_type_ref_display_text(index, module_path, &param.ty)
                ))
                .collect::<Vec<_>>()
                .join(", "),
            source_type_ref_display_text(index, module_path, return_type)
        ),
    }
}

fn source_any_interface_display_text(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    interface: &InterfaceInstantiationRef,
) -> String {
    let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map_or_else(
            |_| interface.interface_abi_id.clone(),
            |ty| source_type_ref_display_text(index, module_path, &ty),
        );
    if interface.canonical_type_args.is_empty() {
        format!("any {interface_name}")
    } else {
        format!(
            "any {interface_name}<{}>",
            interface
                .canonical_type_args
                .iter()
                .map(|arg| source_type_ref_display_text(index, module_path, arg))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn standard_library_type_ref_display_text(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(standard_library_type_ref_display_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", standard_library_type_ref_display_text(ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Union { items } => items
            .iter()
            .map(standard_library_type_ref_display_text)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => {
            format!("{}?", standard_library_type_ref_display_text(inner))
        }
        TypeRefIr::AnyInterface { interface } => {
            standard_library_any_interface_display_text(interface)
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => {
                serde_json::to_string(value).expect("string literal should serialize")
            }
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::PublicationType { module_path, .. } => format!("root.{module_path}"),
        TypeRefIr::LocalType { type_index } => format!("__invalid_local_type_{type_index}"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            if symbol.module_path.is_empty() {
                symbol.symbol.clone()
            } else {
                symbol.symbol_path()
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| format!(
                    "{}: {}",
                    param.name,
                    standard_library_type_ref_display_text(&param.ty)
                ))
                .collect::<Vec<_>>()
                .join(", "),
            standard_library_type_ref_display_text(return_type)
        ),
    }
}

fn standard_library_any_interface_display_text(interface: &InterfaceInstantiationRef) -> String {
    let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map_or_else(
            |_| interface.interface_abi_id.clone(),
            |ty| standard_library_type_ref_display_text(&ty),
        );
    if interface.canonical_type_args.is_empty() {
        format!("any {interface_name}")
    } else {
        format!(
            "any {interface_name}<{}>",
            interface
                .canonical_type_args
                .iter()
                .map(standard_library_type_ref_display_text)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn boundary_rejected_type_schema() -> JsonSchema {
    // AnyInterface must be rejected by boundary validation before schema emission.
    // This fallback is defensive only and is not a public ABI contract; empty
    // oneOf is intentionally unsatisfiable instead of permissive `any`.
    JsonSchema::one_of(Vec::new())
}

fn boundary_rejected_runtime_type_descriptor() -> RuntimeTypeDescriptorIr {
    // AnyInterface must be rejected by boundary validation before runtime
    // descriptor emission. This sentinel is defensive only and is not a public
    // ABI contract.
    RuntimeTypeDescriptorIr::named("__boundary_rejected_any_interface")
}

fn string_map_key_union_schema(items: &[ContractTypeKey]) -> Option<JsonSchema> {
    let values = items
        .iter()
        .map(|item| match item {
            ContractTypeKey::Literal(ContractLiteralKey::String(value)) => Some(value.clone()),
            ContractTypeKey::Builtin { name, args } if args.is_empty() && name == "string" => {
                Some(String::new())
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if values.iter().any(String::is_empty) {
        return Some(JsonSchema::string());
    }
    Some(JsonSchema::string_enum(values))
}

fn schema_is_string_like(schema: &JsonSchema) -> bool {
    schema.schema_type() == Some("string")
}

fn is_transport_message_union(items: &[ContractTypeKey]) -> bool {
    items.len() == 2
        && items.iter().any(is_text_transport_message_branch)
        && items.iter().any(is_binary_transport_message_branch)
}

fn is_text_transport_message_branch(item: &ContractTypeKey) -> bool {
    contract_type_key_name(item).is_some_and(|name| is_text_connection_message_type(&name))
}

fn is_binary_transport_message_branch(item: &ContractTypeKey) -> bool {
    contract_type_key_name(item).is_some_and(|name| is_binary_connection_message_type(&name))
}

fn is_connect_result_generic_type(root: &str) -> bool {
    matches!(
        root,
        "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
    )
}

fn is_websocket_connection_generic_type(root: &str) -> bool {
    matches!(
        root,
        "WebSocketConnection" | "std.websocket.WebSocketConnection"
    )
}

fn is_websocket_receive_event_generic_type(root: &str) -> bool {
    matches!(
        root,
        "WebSocketReceiveEvent" | "std.websocket.WebSocketReceiveEvent"
    )
}

fn is_connection_message_type(name: &str) -> bool {
    prelude_symbol(name) == "std.websocket.ConnectionMessage"
}

fn is_text_connection_message_type(name: &str) -> bool {
    prelude_symbol(name) == "std.websocket.TextConnectionMessage"
}

fn is_binary_connection_message_type(name: &str) -> bool {
    prelude_symbol(name) == "std.websocket.BinaryConnectionMessage"
}

fn prelude_symbol(name: &str) -> String {
    compiler_owned_type_symbol(name)
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}

fn standard_library_package_symbol(
    prelude: &PreludeProjection,
    package: &ContractPackageRefKey,
    symbol_path: &str,
) -> Option<String> {
    let is_standard_package = match package {
        ContractPackageRefKey::PackageId { package_id } => {
            package_id == PRELUDE_REGISTRY_ID || package_id == "skiff.run/std"
        }
        ContractPackageRefKey::Dependency { dependency_ref } => {
            dependency_ref == "std"
                || dependency_ref == PRELUDE_REGISTRY_ID
                || dependency_ref == "skiff.run/std"
        }
    };
    if !is_standard_package && !symbol_path.starts_with("std.") {
        return None;
    }

    prelude
        .known_type_symbol(symbol_path)
        .or_else(|| prelude.known_type_symbol(&format!("std.{symbol_path}")))
}

fn contract_type_key_name(key: &ContractTypeKey) -> Option<String> {
    match key {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => Some(name.clone()),
        ContractTypeKey::Named(name) => Some(name.canonical_symbol()),
        ContractTypeKey::PackageSymbol { symbol_path, .. } => Some(
            compiler_owned_type_symbol(symbol_path)
                .map(str::to_string)
                .unwrap_or_else(|| symbol_path.clone()),
        ),
        _ => None,
    }
}

fn contract_record_discriminator_value(
    key: &ContractTypeKey,
    discriminator: &str,
) -> Option<String> {
    let ContractTypeKey::Record { fields } = key else {
        return None;
    };
    match fields.get(discriminator)? {
        ContractTypeKey::Literal(ContractLiteralKey::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn contract_type_key_display_text(key: &ContractTypeKey) -> String {
    match key {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => name.clone(),
        ContractTypeKey::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(contract_type_key_display_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContractTypeKey::Named(name) => name.canonical_symbol(),
        ContractTypeKey::PackageSymbol { symbol_path, .. } => symbol_path.clone(),
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } if canonical_type_args.is_empty() => {
            format!("any {}", contract_type_key_display_text(interface))
        }
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => format!(
            "any {}<{}>",
            contract_type_key_display_text(interface),
            canonical_type_args
                .iter()
                .map(contract_type_key_display_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContractTypeKey::DbObjectSymbol {
            module_path,
            symbol,
        } => format!("{module_path}.{symbol}"),
        ContractTypeKey::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", contract_type_key_display_text(ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContractTypeKey::Union { items } => items
            .iter()
            .map(contract_type_key_display_text)
            .collect::<Vec<_>>()
            .join(" | "),
        ContractTypeKey::Nullable { inner } => {
            format!("{}?", contract_type_key_display_text(inner))
        }
        ContractTypeKey::Literal(ContractLiteralKey::Null) => "null".to_string(),
        ContractTypeKey::Literal(ContractLiteralKey::Bool(value)) => value.to_string(),
        ContractTypeKey::Literal(ContractLiteralKey::Number(value)) => value.clone(),
        ContractTypeKey::Literal(ContractLiteralKey::String(value)) => {
            serde_json::to_string(value).expect("string literal should serialize")
        }
        ContractTypeKey::TypeParam { name } => name.clone(),
        ContractTypeKey::Function { .. } => "function".to_string(),
    }
}

#[allow(dead_code)]
fn with_prelude_schema_metadata(
    schema: JsonSchema,
    symbol: &str,
    schema_identity: &str,
) -> JsonSchema {
    schema
        .with_x_skiff_package(PRELUDE_REGISTRY_ID.to_string())
        .with_x_skiff_symbol(symbol.to_string())
        .with_x_skiff_schema_identity(schema_identity)
}

fn date_time_schema() -> JsonSchema {
    JsonSchema::string()
        .with_format("date-time")
        .with_x_skiff_symbol("Date")
}
