use skiff_artifact_model::{
    AbiAliasId, AbiCallableId, AbiConstId, AbiDeclarationKind, AbiInstanceId, AbiInterfaceId,
    AbiSourceDeclarationAnchor, AbiSymbolId, AbiSymbolIdFact, AbiTypeId, InterfaceInstantiationRef,
    TypeRefIr,
};

/// Derives the nominal type id from its declaration anchor and ordered type arguments.
pub fn abi_type_id_from_source_anchor(
    anchor: &AbiSourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> AbiTypeId {
    AbiTypeId::from_key_bytes(source_anchor_key("type", anchor, type_args))
}

/// Derives the nominal alias id. Aliases remain distinct from nominal type ids.
pub fn abi_alias_id_from_source_anchor(anchor: &AbiSourceDeclarationAnchor) -> AbiAliasId {
    AbiAliasId::from_key_bytes(source_anchor_key("alias", anchor, &[]))
}

/// Derives the nominal interface id from its anchor and ordered type arguments.
pub fn abi_interface_id_from_source_anchor(
    anchor: &AbiSourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> AbiInterfaceId {
    AbiInterfaceId::from_key_bytes(source_anchor_key("interface", anchor, type_args))
}

/// Derives the nominal callable id from its declaration anchor.
pub fn abi_callable_id_from_source_anchor(anchor: &AbiSourceDeclarationAnchor) -> AbiCallableId {
    AbiCallableId::from_key_bytes(source_anchor_key("callable", anchor, &[]))
}

/// Derives the nominal const id from its declaration anchor.
pub fn abi_const_id_from_source_anchor(anchor: &AbiSourceDeclarationAnchor) -> AbiConstId {
    AbiConstId::from_key_bytes(source_anchor_key("const", anchor, &[]))
}

/// Derives the nominal instance id from its declaration anchor.
pub fn abi_instance_id_from_source_anchor(anchor: &AbiSourceDeclarationAnchor) -> AbiInstanceId {
    AbiInstanceId::from_key_bytes(source_anchor_key("instance", anchor, &[]))
}

/// Projects an opaque nominal type id into its artifact wire key.
pub fn abi_type_id_key(id: &AbiTypeId) -> String {
    abi_id_key_hex(id.key_bytes())
}

/// Projects a typed nominal symbol id into its artifact DTO.
pub fn abi_symbol_id_fact(symbol: &AbiSymbolId) -> AbiSymbolIdFact {
    match symbol {
        AbiSymbolId::Type(id) => AbiSymbolIdFact::Type {
            abi_type_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Alias(id) => AbiSymbolIdFact::Alias {
            abi_alias_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Interface(id) => AbiSymbolIdFact::Interface {
            abi_interface_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Callable(id) => AbiSymbolIdFact::Callable {
            abi_callable_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Const(id) => AbiSymbolIdFact::Const {
            abi_const_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Instance(id) => AbiSymbolIdFact::Instance {
            abi_instance_id: abi_id_key_hex(id.key_bytes()),
        },
    }
}

/// Returns the canonical structural key used by current ABI DTOs for a type ref.
pub fn type_ref_abi_key(ty: &TypeRefIr) -> String {
    canonical_json_string(ty, "TypeRefIr")
}

/// Builds the canonical interface instantiation DTO from a declaration identity and ordered args.
pub fn interface_instantiation_ref(
    interface_decl_identity: TypeRefIr,
    canonical_type_args: Vec<TypeRefIr>,
) -> InterfaceInstantiationRef {
    InterfaceInstantiationRef {
        interface_abi_id: type_ref_abi_key(&interface_decl_identity),
        canonical_type_args,
    }
}

/// Splits native generic interface refs into declaration identity plus ordered type args.
pub fn interface_instantiation_ref_for_type_ref(ty: &TypeRefIr) -> InterfaceInstantiationRef {
    match ty {
        TypeRefIr::Builtin { name, args } if !args.is_empty() => interface_instantiation_ref(
            TypeRefIr::Builtin {
                name: name.clone(),
                args: Vec::new(),
            },
            args.clone(),
        ),
        _ => interface_instantiation_ref(ty.clone(), Vec::new()),
    }
}

/// Derives the method id from its interface declaration, ordered generic args and method name.
pub fn canonical_interface_method_abi_id(
    interface: &InterfaceInstantiationRef,
    method_name: &str,
) -> String {
    canonical_interface_method_abi_id_from_parts(
        &interface.interface_abi_id,
        &interface.canonical_type_args,
        method_name,
    )
}

/// Derives a method id for another typed representation of the same canonical interface parts.
pub fn canonical_interface_method_abi_id_from_parts<T: serde::Serialize>(
    interface_abi_id: &str,
    canonical_type_args: &[T],
    method_name: &str,
) -> String {
    if canonical_type_args.is_empty() {
        format!("method:{interface_abi_id}:{method_name}")
    } else {
        let type_args = canonical_json_string(&canonical_type_args, "interface type args");
        format!("method:{interface_abi_id}:{type_args}:{method_name}")
    }
}

/// Returns the canonical key for equality/deduplication of interface instantiation DTOs.
pub fn canonical_interface_instantiation_key(interface: &InterfaceInstantiationRef) -> String {
    canonical_json_string(interface, "interface instantiation")
}

fn canonical_json_string(value: &impl serde::Serialize, label: &str) -> String {
    let bytes = skiff_canonical_json::canonical_json_bytes(value)
        .unwrap_or_else(|error| panic!("{label} must serialize for ABI identity: {error}"));
    String::from_utf8(bytes).expect("canonical JSON must be UTF-8")
}

fn source_anchor_key(
    symbol_kind: &str,
    anchor: &AbiSourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_framed(&mut bytes, symbol_kind.as_bytes());
    write_framed(&mut bytes, anchor.publication_id.as_bytes());
    bytes.extend_from_slice(&anchor.abi_epoch.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&(anchor.module_path.len() as u32).to_le_bytes());
    for segment in &anchor.module_path {
        write_framed(&mut bytes, segment.as_bytes());
    }
    write_framed(&mut bytes, anchor.symbol.as_bytes());
    bytes.push(declaration_kind_tag(anchor.kind));
    bytes.push(0);
    bytes.extend_from_slice(&(type_args.len() as u32).to_le_bytes());
    for argument in type_args {
        write_framed(&mut bytes, argument.key_bytes());
    }
    bytes
}

fn declaration_kind_tag(kind: AbiDeclarationKind) -> u8 {
    match kind {
        AbiDeclarationKind::Type => 0,
        AbiDeclarationKind::Alias => 1,
        AbiDeclarationKind::Interface => 2,
        AbiDeclarationKind::Callable => 3,
        AbiDeclarationKind::Const => 4,
        AbiDeclarationKind::Instance => 5,
    }
}

fn write_framed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
    output.push(0);
}

fn abi_id_key_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}
