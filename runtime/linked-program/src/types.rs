use std::collections::{BTreeMap, HashMap};

pub use skiff_runtime_model::type_exports::{
    PackageSymbolKey, RuntimeTypeExports, ServiceSymbolKey,
};

use super::{
    addr::{PackageSlot, TypeAddr, UnitAddr},
    linked::{LinkedTypeDescriptor, LinkedTypeRef, LiteralIr, TypeDeclIr},
    package_unit::PackageUnit,
    ServiceSymbolRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn service_symbol_key_from_ref(symbol: &ServiceSymbolRef) -> ServiceSymbolKey {
    ServiceSymbolKey::new(symbol.module_path.clone(), symbol.symbol.clone())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeTypeContext {
    #[serde(default)]
    pub descriptors: HashMap<TypeAddr, TypeDeclIr>,
    #[serde(default)]
    pub exported_types: RuntimeTypeExports,
}

impl RuntimeTypeContext {
    pub fn descriptor(&self, addr: &TypeAddr) -> Option<&LinkedTypeDescriptor> {
        self.descriptors.get(addr).map(|decl| &decl.descriptor)
    }

    pub fn declaration(&self, addr: &TypeAddr) -> Option<&TypeDeclIr> {
        self.descriptors.get(addr)
    }

    pub fn diagnostic_exported_type(&self, label: &str) -> Option<&TypeAddr> {
        self.exported_types.get_by_diagnostic_label(label)
    }

    pub fn exported_service_type(&self, module_path: &str, symbol: &str) -> Option<&TypeAddr> {
        self.exported_types.get_service(module_path, symbol)
    }

    pub fn exported_package_type(
        &self,
        package_slot: PackageSlot,
        symbol_path: &str,
    ) -> Option<&TypeAddr> {
        self.exported_types.get_package(package_slot, symbol_path)
    }
}

pub fn anonymous_type_decl(
    name: impl Into<String>,
    descriptor: LinkedTypeDescriptor,
) -> TypeDeclIr {
    TypeDeclIr {
        name: name.into(),
        descriptor,
        type_params: Vec::new(),
        discriminator: None,
        implements: Vec::new(),
        source_span: None,
    }
}

impl LinkedTypeDescriptor {
    pub fn type_refs(&self) -> Vec<&LinkedTypeRef> {
        match self {
            Self::Record { fields } => fields.values().collect(),
            Self::Alias { target } => vec![target],
            Self::Union { variants } => variants.iter().collect(),
            Self::Native { .. } => Vec::new(),
        }
    }
}

pub fn type_descriptor_to_value(descriptor: &LinkedTypeDescriptor) -> Value {
    match descriptor {
        LinkedTypeDescriptor::Record { fields } => json!({
            "kind": "record",
            "fields": fields
                .iter()
                .map(|(name, ty)| (name.clone(), type_ref_to_value(ty)))
                .collect::<BTreeMap<_, _>>(),
        }),
        LinkedTypeDescriptor::Alias { target } => json!({
            "kind": "alias",
            "target": type_ref_to_value(target),
        }),
        LinkedTypeDescriptor::Union { variants } => json!({
            "kind": "union",
            "variants": variants.iter().map(type_ref_to_value).collect::<Vec<_>>(),
        }),
        LinkedTypeDescriptor::Native { symbol } => json!({
            "kind": "external",
            "symbol": symbol,
        }),
    }
}

pub fn type_ref_to_value(type_ref: &LinkedTypeRef) -> Value {
    match type_ref {
        LinkedTypeRef::Native { name, args } => json!({
            "kind": "builtin",
            "name": name,
            "args": args.iter().map(type_ref_to_value).collect::<Vec<_>>(),
        }),
        LinkedTypeRef::LocalType { type_index } => json!({
            "kind": "localType",
            "typeIndex": type_index,
        }),
        LinkedTypeRef::ServiceSymbol { symbol } => json!({
            "kind": "serviceSymbol",
            "symbol": symbol,
        }),
        LinkedTypeRef::PackageSymbol { symbol } => json!({
            "kind": "packageSymbol",
            "symbol": symbol,
        }),
        LinkedTypeRef::Record { fields } => json!({
            "kind": "record",
            "fields": fields
                .iter()
                .map(|(name, ty)| (name.clone(), type_ref_to_value(ty)))
                .collect::<BTreeMap<_, _>>(),
        }),
        LinkedTypeRef::Union { items } => json!({
            "kind": "union",
            "items": items.iter().map(type_ref_to_value).collect::<Vec<_>>(),
        }),
        LinkedTypeRef::Nullable { inner } => json!({
            "kind": "nullable",
            "inner": type_ref_to_value(inner),
        }),
        LinkedTypeRef::AnyInterface { interface } => json!({
            "kind": "anyInterface",
            "interface": {
                "interfaceAbiId": &interface.interface_abi_id,
                "canonicalTypeArgs": interface
                    .canonical_type_args
                    .iter()
                    .map(type_ref_to_value)
                    .collect::<Vec<_>>(),
            },
        }),
        LinkedTypeRef::Literal { value } => literal_to_value(value),
        LinkedTypeRef::TypeParam { name } => json!({
            "kind": "typeParam",
            "name": name,
        }),
        LinkedTypeRef::Function {
            params,
            return_type,
        } => json!({
            "kind": "function",
            "params": params
                .iter()
                .map(|param| json!({
                    "name": &param.name,
                    "ty": type_ref_to_value(&param.ty),
                }))
                .collect::<Vec<_>>(),
            "returnType": type_ref_to_value(return_type),
        }),
        LinkedTypeRef::DbObjectSymbol { symbol } => json!({
            "kind": "dbObjectSymbol",
            "symbol": symbol,
        }),
        LinkedTypeRef::Address { addr } => json!({
            "kind": "address",
            "addr": addr,
        }),
    }
}

fn literal_to_value(value: &LiteralIr) -> Value {
    match value {
        LiteralIr::Null => json!({
            "kind": "literal",
            "value": { "kind": "null" },
        }),
        LiteralIr::Bool { value } => json!({
            "kind": "literal",
            "value": { "kind": "bool", "value": value },
        }),
        LiteralIr::Number { value } => json!({
            "kind": "literal",
            "value": { "kind": "number", "value": value },
        }),
        LiteralIr::String { value } => json!({
            "kind": "literal",
            "value": { "kind": "string", "value": value },
        }),
    }
}

/// AbiTypeId 恢复链路辅助: 给定 `TypeAddr` 和加载时的 service / package 元数据,
/// 返回拥有该地址的 publication_id。
///
/// - `UnitAddr::Service` → `service_id`(服务的 publication id)
/// - `UnitAddr::Package(slot)` → `packages[slot].package_id`
///
/// 架构 L783-784 要求:跨 file/package 边界的类型引用必须带 owner context 或能恢复
/// `AbiTypeId`。恢复 `AbiTypeId` 需要 `publication_id`(作为 owner publication identity
/// 输入)。本函数提供从 `TypeAddr` 查到 `publication_id` 的链路。
///
/// # 设计说明
///
/// 不把 `publication_id` 内嵌进 `TypeAddr` 或 `LinkedFileUnit`(避免波及所有序列化
/// 点),而是在 linking context 里通过辅助查询提供。`TypeAddr` 本身已携带
/// `UnitAddr + FileAddr`,owner context 完备;本函数只是把 context 里已有的
/// publication_id 显式暴露出来。
pub fn publication_id_for_type_addr<'a>(
    addr: &TypeAddr,
    service_id: &'a str,
    packages: &'a [PackageUnit],
) -> Option<&'a str> {
    match &addr.unit {
        UnitAddr::Service => Some(service_id),
        UnitAddr::Package(slot) => packages.get(*slot).map(|p| p.package_id.as_str()),
    }
}
