use skiff_artifact_model::TypeRefIr;

use super::{
    model::{
        Analyzer, DefaultValueKindNodes, DefaultValueNodes, DerivedConstraint,
        FunctionMachineCarrierFacts, MachineCarrier, MachineDefaultValueFact,
        MachineDefaultValueKind, MachineShapeCarrierFact, MachineWritablePathFact,
        MachineWritableStepFact, Node, PackageMachineCarrierFacts, WritablePathNodes,
        WritableStepNodes,
    },
    policy::{carrier_error, semantic_accepts_carrier},
};
use crate::bytecode::BytecodeEmissionError;

impl Analyzer<'_> {
    pub(super) fn equal(&mut self, left: usize, right: usize, cause: impl Into<String>) {
        self.equalities.push((left, right, cause.into()));
    }

    pub(super) fn assign(
        &mut self,
        node: usize,
        ty: TypeRefIr,
        cause: &str,
    ) -> Result<bool, BytecodeEmissionError> {
        match &self.nodes[node].value {
            Some(existing) if existing != &ty => Err(carrier_error(
                &self.nodes[node].function_key,
                format!(
                    "{} has conflicting exact carriers {existing:?} and {ty:?} ({cause})",
                    self.nodes[node].location
                ),
            )),
            Some(_) => Ok(false),
            None => {
                self.nodes[node].value = Some(ty);
                Ok(true)
            }
        }
    }

    pub(super) fn analyze(mut self) -> Result<PackageMachineCarrierFacts, BytecodeEmissionError> {
        self.propagate()?;
        // Unconstrained boundary positions keep their exact MIR type. This
        // is not normalization: any connected physical producer has already
        // won above and will be checked against the semantic position below.
        for index in 0..self.nodes.len() {
            if self.nodes[index].value.is_none() {
                let semantic = self.nodes[index].semantic.clone();
                self.assign(index, semantic, "unconstrained exact boundary")?;
            }
        }
        self.propagate()?;
        for node in &self.nodes {
            let carrier = node.value.as_ref().expect("all carrier nodes resolved");
            if !semantic_accepts_carrier(&node.semantic, carrier, node.role) {
                return Err(carrier_error(
                    &node.function_key,
                    format!(
                        "{} semantic type {:?} cannot carry exact machine type {carrier:?}",
                        node.location, node.semantic
                    ),
                ));
            }
        }
        self.finish()
    }

    fn propagate(&mut self) -> Result<(), BytecodeEmissionError> {
        let max_rounds = self
            .nodes
            .len()
            .saturating_add(self.derived.len())
            .saturating_add(2);
        for _ in 0..max_rounds {
            let mut changed = false;
            for index in 0..self.equalities.len() {
                let (left, right, cause) = self.equalities[index].clone();
                changed |= self.close_equal_shapes(left, right, &cause)?;
                match (
                    self.nodes[left].value.clone(),
                    self.nodes[right].value.clone(),
                ) {
                    (Some(left_ty), Some(right_ty)) if left_ty != right_ty => {
                        return Err(carrier_error(
                            &self.nodes[left].function_key,
                            format!(
                                "{} requires exact carrier equality, found {left_ty:?} and {right_ty:?}",
                                cause
                            ),
                        ));
                    }
                    (Some(ty), None) => changed |= self.assign(right, ty, &cause)?,
                    (None, Some(ty)) => changed |= self.assign(left, ty, &cause)?,
                    _ => {}
                }
            }
            for index in 0..self.derived.len() {
                changed |= self.apply_derived(index)?;
            }
            if !changed {
                return Ok(());
            }
        }
        Err(BytecodeEmissionError::CanonicalSerialization {
            context: "machine carrier analysis".to_string(),
            message: "finite carrier graph did not converge".to_string(),
        })
    }

    fn close_equal_shapes(
        &mut self,
        left: usize,
        right: usize,
        cause: &str,
    ) -> Result<bool, BytecodeEmissionError> {
        match (self.nodes[left].shape, self.nodes[right].shape) {
            (Some(shape), None) => {
                self.nodes[right].shape = Some(shape);
                Ok(true)
            }
            (None, Some(shape)) => {
                self.nodes[left].shape = Some(shape);
                Ok(true)
            }
            (Some(left_shape), Some(right_shape)) if left_shape != right_shape => {
                let pair = if left_shape < right_shape {
                    (left_shape, right_shape)
                } else {
                    (right_shape, left_shape)
                };
                if self.shape_equalities.contains(&pair) {
                    return Ok(false);
                }
                let left_owner = self.shapes[left_shape].owner.clone();
                let right_owner = self.shapes[right_shape].owner.clone();
                if left_owner != right_owner {
                    return Err(carrier_error(
                        &self.nodes[left].function_key,
                        format!(
                            "{cause} requires exact producer-shape owner equality, found {left_owner:?} and {right_owner:?}"
                        ),
                    ));
                }
                let left_fields = &self.shapes[left_shape].fields;
                let right_fields = &self.shapes[right_shape].fields;
                if left_fields.keys().ne(right_fields.keys()) {
                    return Err(carrier_error(
                        &self.nodes[left].function_key,
                        format!(
                            "{cause} requires identical exact producer-shape field sets for {left_owner:?}"
                        ),
                    ));
                }
                let field_pairs = left_fields
                    .iter()
                    .map(|(name, left)| {
                        (
                            *left,
                            *right_fields
                                .get(name)
                                .expect("exact field sets were checked"),
                            name.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                self.shape_equalities.insert(pair);
                for (left, right, name) in field_pairs {
                    self.equal(left, right, format!("{cause} shape field `{name}`"));
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn apply_derived(&mut self, index: usize) -> Result<bool, BytecodeEmissionError> {
        match &self.derived[index] {
            DerivedConstraint::Array {
                output,
                items,
                empty_type,
                location,
            } => {
                let output = *output;
                let items = items.clone();
                let empty_type = empty_type.clone();
                let location = location.clone();
                let element = common_value(&self.nodes, &items, &location)?;
                let ty = match element {
                    Some(element) => TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![element],
                    },
                    None if items.is_empty() => empty_type,
                    None => return Ok(false),
                };
                self.assign(output, ty, &location)
            }
            DerivedConstraint::Map {
                output,
                values,
                empty_type,
                location,
            } => {
                let output = *output;
                let values = values.clone();
                let empty_type = empty_type.clone();
                let location = location.clone();
                let value = common_value(&self.nodes, &values, &location)?;
                let ty = match value {
                    Some(value) => TypeRefIr::Builtin {
                        name: "Map".to_string(),
                        args: vec![TypeRefIr::builtin("string"), value],
                    },
                    None if values.is_empty() => empty_type,
                    None => return Ok(false),
                };
                self.assign(output, ty, &location)
            }
            DerivedConstraint::Index {
                object,
                selector,
                result,
                location,
            } => {
                let object = *object;
                let selector = *selector;
                let result = *result;
                let location = location.clone();
                let Some(ty) = self.nodes[object].value.clone() else {
                    return Ok(false);
                };
                match ty {
                    TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
                        let mut changed = self.assign(
                            selector,
                            TypeRefIr::builtin("number"),
                            &format!("{location} Array selector"),
                        )?;
                        changed |= self.assign(result, args[0].clone(), &location)?;
                        Ok(changed)
                    }
                    TypeRefIr::Builtin { name, args } if name == "Map" && args.len() == 2 => {
                        let mut changed = self.assign(selector, args[0].clone(), &location)?;
                        changed |= self.assign(result, args[1].clone(), &location)?;
                        Ok(changed)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[result].function_key,
                        format!("{location} has no exact Array/Map machine layout"),
                    )),
                }
            }
            DerivedConstraint::ForIn {
                iterable,
                item,
                value,
                kind,
                location,
            } => {
                let iterable = *iterable;
                let item = *item;
                let value = *value;
                let kind = *kind;
                let location = location.clone();
                let Some(ty) = self.nodes[iterable].value.clone() else {
                    return Ok(false);
                };
                match (kind, ty) {
                    (
                        skiff_compiler_lowering::mir::MirForInItemKind::ArrayItem,
                        TypeRefIr::Builtin { name, args },
                    ) if name == "Array" && args.len() == 1 => {
                        self.assign(item, args[0].clone(), &location)
                    }
                    (
                        skiff_compiler_lowering::mir::MirForInItemKind::StreamItem,
                        TypeRefIr::Builtin { name, args },
                    ) if name == "Stream" && args.len() == 1 => {
                        self.assign(item, args[0].clone(), &location)
                    }
                    (
                        skiff_compiler_lowering::mir::MirForInItemKind::MapKey,
                        TypeRefIr::Builtin { name, args },
                    ) if name == "Map" && args.len() == 2 => {
                        let mut changed = self.assign(item, args[0].clone(), &location)?;
                        if let Some(value) = value {
                            changed |= self.assign(value, args[1].clone(), &location)?;
                        }
                        Ok(changed)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[item].function_key,
                        format!("{location} iterable has no exact machine item layout"),
                    )),
                }
            }
            DerivedConstraint::StreamNext {
                endpoint,
                item,
                location,
            } => {
                let endpoint = *endpoint;
                let item = *item;
                let location = location.clone();
                let Some(ty) = self.nodes[endpoint].value.clone() else {
                    return Ok(false);
                };
                match ty {
                    TypeRefIr::Builtin { name, args } if name == "Stream" && args.len() == 1 => {
                        self.assign(item, args[0].clone(), &location)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[item].function_key,
                        format!("{location} endpoint has no exact Stream<T> machine layout"),
                    )),
                }
            }
        }
    }

    fn finish(self) -> Result<PackageMachineCarrierFacts, BytecodeEmissionError> {
        let mut functions = std::collections::BTreeMap::new();
        for (function_index, function) in self.functions.into_iter().enumerate() {
            let expression_carriers = function
                .expressions
                .iter()
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, *node)))
                .collect();
            let slot_carriers = function
                .slots
                .iter()
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, *node)))
                .collect();
            let result_carrier = function
                .result
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)));
            let stream_result_carrier = function
                .stream_result
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)));
            let stream_next_items = function
                .stream_next_items
                .iter()
                .map(|(statement, node)| {
                    (
                        *statement,
                        MachineCarrier::type_only(resolved(&self.nodes, *node)),
                    )
                })
                .collect();
            let expression_shapes = function
                .expressions
                .iter()
                .map(|node| {
                    self.nodes[*node]
                        .shape
                        .map(|shape| shape_fact(&self.nodes, &self.shapes, shape))
                })
                .collect();
            let slot_shapes = function
                .slots
                .iter()
                .map(|node| {
                    self.nodes[*node]
                        .shape
                        .map(|shape| shape_fact(&self.nodes, &self.shapes, shape))
                })
                .collect();
            let shapes = function
                .shape_indices
                .iter()
                .map(|shape| shape_fact(&self.nodes, &self.shapes, *shape))
                .collect();
            let construct_shapes = function
                .construct_shape_indices
                .iter()
                .map(|(expression, shape)| {
                    (*expression, shape_fact(&self.nodes, &self.shapes, *shape))
                })
                .collect();
            let writable_paths = function
                .writable_paths
                .iter()
                .map(|(statement, path)| {
                    (*statement, writable_fact(&self.nodes, &self.shapes, path))
                })
                .collect();
            let catch_defaults = function
                .catch_defaults
                .iter()
                .map(|(expression, default)| {
                    (
                        *expression,
                        default_fact(&self.nodes, &self.shapes, default),
                    )
                })
                .collect();
            let catch_exception_shapes = function
                .catch_exception_shapes
                .iter()
                .map(|(expression, shape)| {
                    (*expression, shape_fact(&self.nodes, &self.shapes, *shape))
                })
                .collect();
            let all_carriers = self
                .nodes
                .iter()
                .filter(|node| node.function == function_index)
                .map(|node| {
                    MachineCarrier::type_only(
                        node.value
                            .clone()
                            .expect("carrier analysis resolved every node"),
                    )
                })
                .collect();
            functions.insert(
                function.key,
                FunctionMachineCarrierFacts {
                    expression_carriers,
                    slot_carriers,
                    result_carrier,
                    stream_result_carrier,
                    stream_next_items,
                    expression_shapes,
                    slot_shapes,
                    shapes,
                    construct_shapes,
                    writable_paths,
                    catch_defaults,
                    catch_exception_shapes,
                    all_carriers,
                },
            );
        }
        Ok(PackageMachineCarrierFacts { functions })
    }
}

fn common_value(
    nodes: &[Node],
    members: &[usize],
    location: &str,
) -> Result<Option<TypeRefIr>, BytecodeEmissionError> {
    let mut value: Option<TypeRefIr> = None;
    for member in members {
        let Some(candidate) = nodes[*member].value.as_ref() else {
            return Ok(None);
        };
        if value.as_ref().is_some_and(|value| value != candidate) {
            return Err(carrier_error(
                &nodes[*member].function_key,
                format!("{location} has heterogeneous exact machine carriers"),
            ));
        }
        value = Some(candidate.clone());
    }
    Ok(value)
}

fn resolved(nodes: &[Node], node: usize) -> TypeRefIr {
    nodes[node]
        .value
        .clone()
        .expect("carrier analysis resolved every node")
}

fn shape_fact(
    nodes: &[Node],
    shapes: &[super::model::ShapeNodes],
    shape: usize,
) -> MachineShapeCarrierFact {
    let shape = &shapes[shape];
    MachineShapeCarrierFact {
        owner: shape.owner.clone(),
        fields: shape
            .fields
            .iter()
            .map(|(name, node)| {
                (
                    name.clone(),
                    MachineCarrier::type_only(resolved(nodes, *node)),
                )
            })
            .collect(),
    }
}

fn default_fact(
    nodes: &[Node],
    shapes: &[super::model::ShapeNodes],
    default: &DefaultValueNodes,
) -> MachineDefaultValueFact {
    let kind = match &default.kind {
        DefaultValueKindNodes::Literal { value } => MachineDefaultValueKind::Literal {
            value: value.clone(),
        },
        DefaultValueKindNodes::EmptyArray { element } => MachineDefaultValueKind::EmptyArray {
            element: MachineCarrier::type_only(element.clone()),
        },
        DefaultValueKindNodes::Record { shape, fields } => MachineDefaultValueKind::Record {
            shape: shape_fact(nodes, shapes, *shape),
            fields: fields
                .iter()
                .map(|(name, field)| (name.clone(), default_fact(nodes, shapes, field)))
                .collect(),
        },
    };
    MachineDefaultValueFact {
        carrier: MachineCarrier::type_only(resolved(nodes, default.value)),
        kind,
    }
}

fn writable_fact(
    nodes: &[Node],
    shapes: &[super::model::ShapeNodes],
    path: &WritablePathNodes,
) -> MachineWritablePathFact {
    MachineWritablePathFact {
        root: MachineCarrier::type_only(resolved(nodes, path.root)),
        leaf: MachineCarrier::type_only(resolved(nodes, path.leaf)),
        steps: path
            .steps
            .iter()
            .map(|step| match step {
                WritableStepNodes::DenseField { name, shape } => {
                    MachineWritableStepFact::DenseField {
                        name: name.clone(),
                        shape: shape_fact(nodes, shapes, *shape),
                    }
                }
                WritableStepNodes::ArrayIndex {
                    selector_expression,
                    selector,
                    element,
                } => MachineWritableStepFact::ArrayIndex {
                    selector_expression: *selector_expression,
                    selector: MachineCarrier::type_only(resolved(nodes, *selector)),
                    element: MachineCarrier::type_only(resolved(nodes, *element)),
                },
                WritableStepNodes::MapKey {
                    selector_expression,
                    selector,
                    key,
                    value,
                } => MachineWritableStepFact::MapKey {
                    selector_expression: *selector_expression,
                    selector: MachineCarrier::type_only(resolved(nodes, *selector)),
                    key: MachineCarrier::type_only(resolved(nodes, *key)),
                    value: MachineCarrier::type_only(resolved(nodes, *value)),
                },
            })
            .collect(),
    }
}
