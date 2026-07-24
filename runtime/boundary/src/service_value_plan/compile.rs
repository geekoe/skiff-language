use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    websocket_ingress::{
        canonical_websocket_shape_spec, WebSocketShape, WebSocketShapeId, WebSocketShapeType,
    },
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord,
};
use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
};

use super::codec_error;
use crate::{
    json, service_linkable::ServiceLinkableMaterializationError,
    service_linkable_schema::resolve_record, service_schema_records::ServiceSchemaRecords,
};

type PackageSchema = ServiceSchemaRecords;

pub(super) fn compile(
    contract_type: &ContractTypeRef,
    schema: &PackageSchema,
) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
    ServiceValuePlanCompiler::new(schema).compile(contract_type)
}

struct ServiceValuePlanCompiler<'schema> {
    schema: &'schema PackageSchema,
    active_contract_types: BTreeSet<PackageSchemaTypeId>,
    compiled_contract_types: BTreeMap<PackageSchemaTypeId, RuntimeTypePlan>,
    active_websocket_shapes: BTreeSet<WebSocketShapeId>,
}

impl<'schema> ServiceValuePlanCompiler<'schema> {
    fn new(schema: &'schema PackageSchema) -> Self {
        Self {
            schema,
            active_contract_types: BTreeSet::new(),
            compiled_contract_types: BTreeMap::new(),
            active_websocket_shapes: BTreeSet::new(),
        }
    }

    fn compile(
        &mut self,
        contract_type: &ContractTypeRef,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        match contract_type {
            ContractTypeRef::Builtin { name, arguments } => self.compile_builtin(name, arguments),
            ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.compile_package_schema_type(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            ),
            ContractTypeRef::TypeParam { name } => {
                invalid_contract_plan(format!("unresolved contract type parameter {name}"))
            }
            ContractTypeRef::Record { fields } => {
                let fields = self.compile_record_fields(fields)?;
                Ok(plan(
                    "contract inline record",
                    None,
                    RuntimeTypeNode::Record {
                        fields,
                        boundary_record_kind: None,
                    },
                ))
            }
            ContractTypeRef::StructuralUnion { variants } => {
                self.compile_structural_union("contract structural union", variants)
            }
            ContractTypeRef::Nullable { inner } => Ok(plan(
                "contract nullable",
                None,
                RuntimeTypeNode::Nullable(Box::new(self.compile(inner)?)),
            )),
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value },
            } => Ok(literal_plan(value)),
        }
    }

    fn compile_builtin(
        &mut self,
        name: &str,
        arguments: &[ContractTypeRef],
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        if let Some(http_type) =
            skiff_artifact_model::http_boundary::canonical_http_boundary_type(name)
        {
            if !arguments.is_empty() {
                return invalid_contract_plan(format!(
                    "builtin {name} expects 0 argument(s), got {}",
                    arguments.len()
                ));
            }
            return self.compile(&http_type);
        }
        if let Some(builtin) = canonical_websocket_shape_spec().contract_builtin_named(name) {
            if arguments.len() != builtin.context_arity() {
                return invalid_contract_plan(format!(
                    "builtin {name} expects {} argument(s), got {}",
                    builtin.context_arity(),
                    arguments.len()
                ));
            }
            let context_type = arguments
                .first()
                .expect("canonical WebSocket builtins have one Context argument");
            if !is_websocket_context_type(context_type) {
                return invalid_contract_plan(format!(
                    "builtin {name} Context must be null or one exact PackageSchema type"
                ));
            }
            let context = self.compile(context_type)?;
            return self.compile_websocket_shape(builtin.shape(), &context);
        }

        let no_arguments = arguments.is_empty();
        let node = match name {
            "void" | "null" if no_arguments => RuntimeTypeNode::Null,
            "bool" if no_arguments => RuntimeTypeNode::Bool,
            "number" if no_arguments => RuntimeTypeNode::Number,
            "integer" if no_arguments => RuntimeTypeNode::Integer,
            "string" if no_arguments => RuntimeTypeNode::String,
            "bytes" if no_arguments => RuntimeTypeNode::Bytes,
            "Date" if no_arguments => RuntimeTypeNode::Date,
            "Duration" if no_arguments => RuntimeTypeNode::Representation {
                type_name: "std.time.Duration".to_string(),
                payload: Box::new(plan(
                    "contract builtin integer",
                    Some("integer".to_string()),
                    RuntimeTypeNode::Integer,
                )),
            },
            "Json" if no_arguments => RuntimeTypeNode::Json,
            "JsonObject" if no_arguments => RuntimeTypeNode::JsonObject,
            "Array" if arguments.len() == 1 => {
                RuntimeTypeNode::Array(Box::new(self.compile(&arguments[0])?))
            }
            "Map" if arguments.len() == 2 => {
                let key = self.compile_map_key(&arguments[0])?;
                let value = self.compile(&arguments[1])?;
                RuntimeTypeNode::Map {
                    key: Box::new(key),
                    value: Box::new(value),
                }
            }
            "Array" | "Map" => {
                let expected = usize::from(name == "Array") + usize::from(name == "Map") * 2;
                return invalid_contract_plan(format!(
                    "builtin {name} expects {expected} argument(s), got {}",
                    arguments.len()
                ));
            }
            _ => {
                return invalid_contract_plan(format!(
                    "unknown or non-canonical contract builtin {name}"
                ))
            }
        };
        Ok(plan(
            format!("contract builtin {name}"),
            Some(name.to_string()),
            node,
        ))
    }

    fn compile_map_key(
        &mut self,
        key: &ContractTypeRef,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        match key {
            ContractTypeRef::Builtin { name, arguments }
                if name == "string" && arguments.is_empty() =>
            {
                self.compile(key)
            }
            ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => {
                let schema_type =
                    self.schema_type(package_id, stable_schema_key, package_schema_type_id)?;
                let ContractTypeDescriptor::Representation { target } =
                    &schema_type.canonical_descriptor.descriptor
                else {
                    return invalid_contract_plan(format!(
                        "Map key {package_schema_type_id} must be one nominal representation over string"
                    ));
                };
                if !matches!(
                    target,
                    ContractTypeRef::Builtin { name, arguments }
                        if name == "string" && arguments.is_empty()
                ) {
                    return invalid_contract_plan(format!(
                        "Map key representation {package_schema_type_id} must target exact string"
                    ));
                }
                self.compile_package_schema_type(
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                )
            }
            _ => invalid_contract_plan(
                "Map key must be exact string or one nominal representation over string",
            ),
        }
    }

    fn compile_package_schema_type(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        let schema_type =
            self.schema_type(package_id, stable_schema_key, package_schema_type_id)?;
        if let Some(plan) = self.compiled_contract_types.get(package_schema_type_id) {
            return Ok(plan.clone());
        }
        if !self
            .active_contract_types
            .insert(package_schema_type_id.clone())
        {
            return Err(ServiceLinkableMaterializationError::CyclicSchema {
                package_schema_type_id: package_schema_type_id.clone(),
            });
        }
        let result = self.compile_descriptor(schema_type);
        self.active_contract_types.remove(package_schema_type_id);
        let compiled = result?;
        self.compiled_contract_types
            .insert(package_schema_type_id.clone(), compiled.clone());
        Ok(compiled)
    }

    fn schema_type(
        &self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<&'schema PackageSchemaTypeRecord, ServiceLinkableMaterializationError> {
        resolve_record(
            package_id,
            stable_schema_key,
            package_schema_type_id,
            self.schema,
        )
    }

    fn compile_descriptor(
        &mut self,
        schema_type: &PackageSchemaTypeRecord,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        let package_schema_type_id = &schema_type.package_schema_type_id;
        let node = match &schema_type.canonical_descriptor.descriptor {
            ContractTypeDescriptor::Record { fields } => RuntimeTypeNode::Record {
                fields: self.compile_record_fields(fields)?,
                boundary_record_kind: Some(package_identity(schema_type)),
            },
            ContractTypeDescriptor::StructuralUnion { variants } => {
                self.compile_structural_union(&schema_type.stable_schema_key, variants)?
                    .node
            }
            ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field,
                branches,
            } => {
                if branches.is_empty() {
                    return invalid_contract_plan(format!(
                        "discriminated union {package_schema_type_id} has no branches"
                    ));
                }
                let mut tags = BTreeSet::new();
                let mut compiled = Vec::with_capacity(branches.len());
                for branch in branches {
                    if !tags.insert(branch.tag.as_str()) {
                        return invalid_contract_plan(format!(
                            "discriminated union {package_schema_type_id} repeats tag {}",
                            branch.tag
                        ));
                    }
                    let mut branch_plan = self.compile(&branch.branch_type)?;
                    if record_literal_field(&branch_plan, discriminator_field)
                        != Some(branch.tag.as_str())
                    {
                        return invalid_contract_plan(format!(
                            "discriminated union {package_schema_type_id} branch {} has the wrong discriminator",
                            branch.tag
                        ));
                    }
                    branch_plan.identity.union_branch =
                        Some(format!("{}:{}", package_identity(schema_type), branch.tag));
                    compiled.push(branch_plan);
                }
                RuntimeTypeNode::Union(compiled)
            }
            ContractTypeDescriptor::Representation { target } => RuntimeTypeNode::Representation {
                type_name: schema_type.stable_schema_key.clone(),
                payload: Box::new(self.compile(target)?),
            },
            ContractTypeDescriptor::Alias { .. } => {
                return Err(ServiceLinkableMaterializationError::AliasSchema {
                    package_schema_type_id: package_schema_type_id.clone(),
                })
            }
            ContractTypeDescriptor::Enumeration { variants } => {
                if variants.is_empty() {
                    return invalid_contract_plan(format!(
                        "enumeration {package_schema_type_id} has no variants"
                    ));
                }
                let mut seen = BTreeSet::new();
                let mut compiled = Vec::with_capacity(variants.len());
                for variant in variants {
                    if !seen.insert(variant.as_str()) {
                        return invalid_contract_plan(format!(
                            "enumeration {package_schema_type_id} repeats variant {variant}"
                        ));
                    }
                    compiled.push(literal_plan(variant));
                }
                RuntimeTypeNode::Union(compiled)
            }
            ContractTypeDescriptor::CallbackInterface { .. } => {
                return Err(
                    ServiceLinkableMaterializationError::CallbackInterfaceSchema {
                        package_schema_type_id: package_schema_type_id.clone(),
                    },
                )
            }
        };
        let mut identity = RuntimeTypeIdentityPlan {
            nominal: Some(package_identity(schema_type)),
            ..RuntimeTypeIdentityPlan::default()
        };
        if matches!(node, RuntimeTypeNode::Union(_)) {
            identity.union = Some(package_identity(schema_type));
        }
        Ok(RuntimeTypePlan {
            label: format!(
                "package type {}:{}",
                schema_type.package_id, schema_type.stable_schema_key
            ),
            named_type_name: Some(schema_type.stable_schema_key.clone()),
            identity,
            node,
        })
    }

    fn compile_record_fields(
        &mut self,
        fields: &BTreeMap<String, ContractTypeRef>,
    ) -> Result<Vec<RuntimeRecordFieldPlan>, ServiceLinkableMaterializationError> {
        fields
            .iter()
            .map(|(name, ty)| {
                reject_reserved_contract_field(name)?;
                Ok(RuntimeRecordFieldPlan::new(
                    name.clone(),
                    self.compile(ty)?,
                    true,
                ))
            })
            .collect()
    }

    fn compile_structural_union(
        &mut self,
        label: &str,
        variants: &[ContractTypeRef],
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        if variants.is_empty() {
            return invalid_contract_plan(format!("{label} has no variants"));
        }
        Ok(plan(
            label,
            None,
            RuntimeTypeNode::Union(
                variants
                    .iter()
                    .map(|variant| self.compile(variant))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ))
    }

    fn compile_websocket_shape(
        &mut self,
        shape_id: WebSocketShapeId,
        context: &RuntimeTypePlan,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        if !self.active_websocket_shapes.insert(shape_id) {
            return invalid_contract_plan(format!(
                "canonical WebSocket shape graph cycles at {}",
                shape_id.canonical_name()
            ));
        }
        let shape = canonical_websocket_shape_spec().shape(shape_id);
        let node = match shape {
            WebSocketShape::Record { fields } => RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok(RuntimeRecordFieldPlan::new(
                            field.name(),
                            self.compile_websocket_shape_type(field.ty(), context)?,
                            true,
                        ))
                    })
                    .collect::<Result<Vec<_>, ServiceLinkableMaterializationError>>()?,
                boundary_record_kind: Some(shape_id.canonical_name().to_string()),
            },
            WebSocketShape::TaggedUnion {
                discriminator_field,
                variants,
            } => {
                let mut tags = BTreeSet::new();
                let mut plans = Vec::with_capacity(variants.len());
                for variant in variants {
                    let tag = variant.tag(discriminator_field).ok_or_else(|| {
                        ServiceLinkableMaterializationError::InvalidContractPlan {
                            message: format!(
                                "canonical WebSocket variant {} has no exact {discriminator_field} tag",
                                variant.canonical_name()
                            ),
                        }
                    })?;
                    if !tags.insert(tag) {
                        return invalid_contract_plan(format!(
                            "canonical WebSocket shape {} repeats tag {tag}",
                            shape_id.canonical_name()
                        ));
                    }
                    let fields = variant
                        .fields()
                        .iter()
                        .map(|field| {
                            Ok(RuntimeRecordFieldPlan::new(
                                field.name(),
                                self.compile_websocket_shape_type(field.ty(), context)?,
                                true,
                            ))
                        })
                        .collect::<Result<Vec<_>, ServiceLinkableMaterializationError>>()?;
                    plans.push(plan(
                        variant.canonical_name(),
                        Some(variant.canonical_name().to_string()),
                        RuntimeTypeNode::Record {
                            fields,
                            boundary_record_kind: Some(variant.canonical_name().to_string()),
                        },
                    ));
                }
                RuntimeTypeNode::Union(plans)
            }
        };
        self.active_websocket_shapes.remove(&shape_id);
        Ok(plan(
            shape_id.canonical_name(),
            Some(shape_id.canonical_name().to_string()),
            node,
        ))
    }

    fn compile_websocket_shape_type(
        &mut self,
        shape_type: &WebSocketShapeType,
        context: &RuntimeTypePlan,
    ) -> Result<RuntimeTypePlan, ServiceLinkableMaterializationError> {
        match shape_type {
            WebSocketShapeType::String => Ok(plan(
                "canonical WebSocket string",
                Some("string".to_string()),
                RuntimeTypeNode::String,
            )),
            WebSocketShapeType::Integer => Ok(plan(
                "canonical WebSocket integer",
                Some("integer".to_string()),
                RuntimeTypeNode::Integer,
            )),
            WebSocketShapeType::Context => Ok(context.clone()),
            WebSocketShapeType::Shape(shape_id) => self.compile_websocket_shape(*shape_id, context),
            WebSocketShapeType::Array(item) => Ok(plan(
                "canonical WebSocket array",
                Some("Array".to_string()),
                RuntimeTypeNode::Array(Box::new(self.compile_websocket_shape_type(item, context)?)),
            )),
            WebSocketShapeType::Nullable(inner) => Ok(plan(
                "canonical WebSocket nullable",
                None,
                RuntimeTypeNode::Nullable(Box::new(
                    self.compile_websocket_shape_type(inner, context)?,
                )),
            )),
            WebSocketShapeType::StringLiteral(value) => Ok(literal_plan(value)),
            WebSocketShapeType::StringLiteralUnion(values) => {
                if values.is_empty() {
                    return invalid_contract_plan(
                        "canonical WebSocket literal union has no variants",
                    );
                }
                Ok(plan(
                    "canonical WebSocket literal union",
                    None,
                    RuntimeTypeNode::Union(
                        values.iter().map(|value| literal_plan(value)).collect(),
                    ),
                ))
            }
        }
    }
}

fn plan(
    label: impl Into<String>,
    named_type_name: Option<String>,
    node: RuntimeTypeNode,
) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: label.into(),
        named_type_name,
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

fn package_identity(record: &PackageSchemaTypeRecord) -> String {
    format!(
        "package-schema:{}:{}:{}",
        record.package_id, record.stable_schema_key, record.package_schema_type_id
    )
}

fn literal_plan(value: &str) -> RuntimeTypePlan {
    plan(
        format!("contract literal {value:?}"),
        None,
        RuntimeTypeNode::LiteralString(value.to_string()),
    )
}

fn is_websocket_context_type(contract_type: &ContractTypeRef) -> bool {
    matches!(
        contract_type,
        ContractTypeRef::Builtin { name, arguments }
            if name == "null" && arguments.is_empty()
    ) || matches!(contract_type, ContractTypeRef::PackageSchema { .. })
}

fn record_literal_field<'a>(plan: &'a RuntimeTypePlan, field_name: &str) -> Option<&'a str> {
    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        return None;
    };
    let field = fields.iter().find(|field| field.name == field_name)?;
    match field.ty.node() {
        RuntimeTypeNode::LiteralString(value) => Some(value),
        _ => None,
    }
}

fn reject_reserved_contract_field(
    field_name: &str,
) -> Result<(), ServiceLinkableMaterializationError> {
    json::reject_reserved_legacy_metadata_key(field_name).map_err(codec_error)
}

fn invalid_contract_plan<T>(
    message: impl Into<String>,
) -> Result<T, ServiceLinkableMaterializationError> {
    Err(ServiceLinkableMaterializationError::InvalidContractPlan {
        message: message.into(),
    })
}
