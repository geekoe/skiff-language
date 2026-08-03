use super::builtins::std_runtime_builtin_node;
use super::*;

impl RuntimeTypePlanLinkedExt for RuntimeTypePlan {
    /// Build a `RuntimeTypePlan` directly from a service dependency operation's
    /// artifact `TypeRefIr`.
    ///
    /// This mirrors `from_descriptor(serde_json::to_value(type_ref))` without
    /// using a `serde_json::Value` round-trip and without resolving refs against
    /// the caller `RuntimeProgram`. Service dependency signatures come from the
    /// callee artifact, so `LocalType`/symbols intentionally remain unresolved
    /// (`Unknown`) at any depth, matching the old descriptor path.
    fn from_artifact_type_ref(type_ref: &skiff_artifact_model::TypeRefIr) -> Result<Self> {
        use skiff_artifact_model::TypeRefIr;

        let node = match type_ref {
            TypeRefIr::Builtin { name, args } => Self::artifact_builtin_node(name, args)?,
            TypeRefIr::Record { fields } => RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        Ok(RuntimeRecordFieldPlan {
                            name: name.clone(),
                            ty: Self::from_artifact_type_ref(ty)?,
                            required: !matches!(ty, TypeRefIr::Nullable { .. }),
                            identity: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                boundary_record_kind: None,
            },
            TypeRefIr::Union { items } => RuntimeTypeNode::Union(
                items
                    .iter()
                    .map(Self::from_artifact_type_ref)
                    .collect::<Result<Vec<_>>>()?,
            ),
            TypeRefIr::Nullable { inner } => {
                RuntimeTypeNode::Nullable(Box::new(Self::from_artifact_type_ref(inner)?))
            }
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => RuntimeTypeNode::LiteralString(value.clone()),
            TypeRefIr::AppliedNominal { .. } => {
                return Err(RuntimeError::InvalidArtifact(
                    "artifact applied nominal must be linked before building an execution type plan"
                        .to_string(),
                ));
            }
            TypeRefIr::Literal { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::Function { .. } => RuntimeTypeNode::Unknown,
        };
        Ok(Self {
            label: artifact_type_ref_label(type_ref).to_string(),
            named_type_name: artifact_type_ref_named_type_name(type_ref),
            identity: RuntimeTypeIdentityPlan::default(),
            node,
        })
    }

    fn from_artifact_type_ref_in_program(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: &LinkedProgramImage,
        current_addr: &ExecutableAddr,
    ) -> Result<Self> {
        Self::from_artifact_type_ref_in_program_ref(
            type_ref,
            &PlanContext::new(program, current_addr),
        )
    }

    fn from_artifact_type_ref_in_type_view(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
    ) -> Result<Self> {
        Self::from_artifact_type_ref_in_program_ref(
            type_ref,
            &PlanContext::from_type_view(program, current_addr),
        )
    }

    fn from_artifact_type_ref_in_program_ref(
        type_ref: &skiff_artifact_model::TypeRefIr,
        ctx: &PlanContext<'_>,
    ) -> Result<Self> {
        use skiff_artifact_model::TypeRefIr;

        let node = match type_ref {
            TypeRefIr::Builtin { name, args } => {
                Self::artifact_builtin_node_in_program(name, args, ctx)?
            }
            TypeRefIr::Record { fields } => RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        Ok(RuntimeRecordFieldPlan {
                            name: name.clone(),
                            ty: Self::from_artifact_type_ref_in_program_ref(ty, ctx)?,
                            required: !matches!(ty, TypeRefIr::Nullable { .. }),
                            identity: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                boundary_record_kind: None,
            },
            TypeRefIr::Union { items } => RuntimeTypeNode::Union(
                items
                    .iter()
                    .map(|item| Self::from_artifact_type_ref_in_program_ref(item, ctx))
                    .collect::<Result<Vec<_>>>()?,
            ),
            TypeRefIr::Nullable { inner } => RuntimeTypeNode::Nullable(Box::new(
                Self::from_artifact_type_ref_in_program_ref(inner, ctx)?,
            )),
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => RuntimeTypeNode::LiteralString(value.clone()),
            TypeRefIr::PackageSymbol { symbol } => {
                let linked = LinkedTypeRef::PackageSymbol {
                    symbol: symbol.clone(),
                };
                return Self::from_linked_nested_ref(&linked, ctx);
            }
            TypeRefIr::PackageSchema { .. } => RuntimeTypeNode::Unknown,
            TypeRefIr::AppliedNominal { .. } => {
                return Err(RuntimeError::InvalidArtifact(
                    "artifact applied nominal must be linked before building an execution type plan"
                        .to_string(),
                ));
            }
            TypeRefIr::Literal { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::Function { .. } => RuntimeTypeNode::Unknown,
        };
        Ok(Self {
            label: artifact_type_ref_label(type_ref).to_string(),
            named_type_name: artifact_type_ref_named_type_name(type_ref),
            identity: RuntimeTypeIdentityPlan::default(),
            node,
        })
    }

    /// Build a `RuntimeTypePlan` directly from a `LinkedTypeRef`, bypassing the
    /// JSON round-trip used by `Interpreter::program_type_descriptor` +
    /// `from_descriptor`.
    ///
    /// This is the TOP-LEVEL entry point and mirrors the top-level dispatch of
    /// `program_type_descriptor`:
    ///   * `Address` resolves: its interned `LinkedTypeDescriptor` is fetched and
    ///     processed (recursively resolving nested refs).
    ///   * `LocalType` / `ServiceSymbol` / `PackageSymbol`
    ///     ERROR with `InvalidArtifact("... was not linked before execution")` —
    ///     top-level resolution of these is intentionally NOT performed; they
    ///     only resolve when encountered *nested* inside an already-interned
    ///     descriptor (see [`Self::from_linked_ref`]).
    ///   * Structural variants (Builtin/Record/Union/Nullable/Literal) are built
    ///     natively.
    ///   * `DbObjectSymbol` resolves through the current unit's explicit
    ///     module/symbol file declarations or link targets when the attached
    ///     object type is interned. Service-unit refs may also use the service
    ///     type export table by structured module/symbol key. Missing package
    ///     locals still bridge to the old unknown descriptor fallback rather
    ///     than falling back to an unrelated package export.
    ///   * `Function`/`TypeParam` are bridged through
    ///     `from_descriptor(type_ref_to_value(..))`. The reference pipeline emits
    ///     `type_ref_to_value` for these and `from_descriptor` yields `Unknown`
    ///     regardless of whether any nested ref would resolve (their descriptor
    ///     `kind`s are not recognised as structural nodes), so the bridge is
    ///     observably equivalent.
    ///
    /// label / named_type_name are derived exactly as the JSON path would: from
    /// the current node's serialisation (`type_ref_to_value` for refs,
    /// `type_descriptor_to_value` for a resolved descriptor) via
    /// `descriptor_label` / `named_type_name`, so they match byte-for-byte.
    ///
    /// TERMINATION: cycles are bounded purely by the depth-32 cap, exactly as the
    /// reference's `resolve_program_descriptor_refs` does (it has no visited
    /// set). An early visited-set short-circuit is deliberately NOT used because
    /// it would truncate self-referential types *earlier* than depth 32 and so
    /// diverge from the reference's observable (depth-truncated) plan. The depth
    /// accounting in [`PlanContext::deeper_by`] mirrors the JSON nesting so the
    /// cap trips at the identical node.
    #[allow(dead_code)]
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        match type_ref {
            // Top-level Address resolves directly (program_type_descriptor's
            // dedicated Address arm fetches the descriptor itself, then runs the
            // resolve walk at depth 0).
            LinkedTypeRef::Address { addr } => {
                let declaration = ctx.program.types.declaration(addr).ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram type address {addr} is not interned"
                    ))
                })?;
                Self::from_linked_declaration(declaration, ctx)
            }
            // Top-level LocalType / non-http ServiceSymbol / non-http
            // PackageSymbol error identically to program_type_descriptor.
            LinkedTypeRef::LocalType { .. }
            | LinkedTypeRef::ServiceSymbol { .. }
            | LinkedTypeRef::PackageSymbol { .. } => Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram type ref {} was not linked before execution",
                linked_type_ref_kind(type_ref)
            ))),
            // Structural / bridged variants share the nested-walk logic; at the
            // top level they are processed at depth 0 just like the resolve walk
            // would process program_type_descriptor's `type_ref_to_value(..)`.
            _ => Self::from_linked_ref(type_ref, ctx),
        }
    }

    /// Build a plan for a `LinkedTypeRef` that is already known to sit in a
    /// nested type-ref position.
    ///
    /// This intentionally uses the resolver semantics of [`Self::from_linked_ref`]:
    /// `LocalType`, `ServiceSymbol`, and `PackageSymbol` refs resolve here,
    /// unlike the top-level [`Self::from_linked`] entry point where those refs
    /// still report the historical "not linked before execution" artifact
    /// error.
    fn from_linked_nested_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        Self::from_linked_ref(type_ref, ctx)
    }

    /// Mirrors `resolve_program_descriptor_refs` encountering a type-ref value at
    /// `ctx.depth`. Used both for the top-level structural/bridged variants and
    /// for every nested ref reached while resolving a descriptor's children.
    ///
    /// Unlike [`Self::from_linked`], symbol/localType refs DO resolve here (this
    /// is the "nested" position), matching the asymmetry the oracle pins down.
    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        // Past the depth cap the JSON walk returns the value unresolved; the
        // unresolved subtree now degrades directly to Unknown instead of
        // crossing the legacy descriptor bridge.
        if ctx.over_depth_cap() {
            return Ok(unknown_plan_for_type_ref(type_ref));
        }
        let node = match type_ref {
            LinkedTypeRef::Native { name, args } => Self::builtin_node(name, args, ctx)?,
            // record object -> `fields` object map -> field value. The JSON
            // substitution pass never descends into the `fields` object map, so
            // substitutions are dropped here.
            LinkedTypeRef::Record { fields } => RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| {
                        Ok(RuntimeRecordFieldPlan {
                            name: name.clone(),
                            ty: Self::from_linked_ref(
                                field_ty,
                                &ctx.without_substitutions().deeper_by(2),
                            )?,
                            required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                            identity: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                boundary_record_kind: None,
            },
            // Inline union serialises as `items` (an array), which the JSON
            // substitution pass DOES descend into, so substitutions are kept.
            LinkedTypeRef::Union { items } => RuntimeTypeNode::Union(
                items
                    .iter()
                    .map(|item| Self::from_linked_ref(item, &ctx.deeper_by(2)))
                    .collect::<Result<Vec<_>>>()?,
            ),
            LinkedTypeRef::Nullable { inner } => RuntimeTypeNode::Nullable(Box::new(
                // nullable object -> `inner` value.
                Self::from_linked_ref(inner, &ctx.deeper_by(1))?,
            )),
            LinkedTypeRef::AnyInterface { .. } => RuntimeTypeNode::Unknown,
            LinkedTypeRef::Literal { value } => match value {
                LiteralIr::String { value } => RuntimeTypeNode::LiteralString(value.clone()),
                _ => RuntimeTypeNode::Unknown,
            },
            // Nested refs resolve against the program's type context. If the ref
            // cannot be resolved (missing symbol / descriptor) the JSON walk
            // leaves the bare ref in place, which `from_descriptor` maps to
            // Unknown — so we fall back to the bridge in that case.
            LinkedTypeRef::Address { addr } => {
                return Self::resolve_addr_or_bridge(type_ref, addr.clone(), ctx);
            }
            LinkedTypeRef::LocalType { type_index } => {
                let addr = TypeAddr {
                    unit: ctx.current_addr.unit.clone(),
                    file: ctx.current_addr.file.clone(),
                    type_index: *type_index,
                };
                return Self::resolve_addr_or_bridge(type_ref, addr, ctx);
            }
            LinkedTypeRef::PublicationType {
                module_path,
                type_index,
            } => {
                match program_publication_type_addr(
                    ctx.program,
                    &ctx.current_addr.unit,
                    module_path,
                    *type_index,
                ) {
                    Some(addr) => return Self::resolve_addr_or_bridge(type_ref, addr, ctx),
                    None => return Ok(unknown_plan_for_type_ref(type_ref)),
                }
            }
            LinkedTypeRef::ServiceSymbol { symbol } => {
                if is_actor_declaration_symbol(ctx.program, symbol) {
                    // The actor registry intrinsic pins the actor declaration
                    // with a service symbol. An Actor handle is an opaque
                    // runtime value, not a service record.
                    return Ok(unknown_plan_for_type_ref(type_ref));
                }
                match program_service_symbol_type_addr(ctx.program, &ctx.current_addr.unit, symbol)?
                {
                    Some(addr) => return Self::resolve_addr_or_bridge(type_ref, addr, ctx),
                    None => return Ok(unknown_plan_for_type_ref(type_ref)),
                }
            }
            LinkedTypeRef::PackageSymbol { symbol } => {
                match program_package_type_addr(ctx.program, symbol) {
                    Some(addr) => return Self::resolve_addr_or_bridge(type_ref, addr, ctx),
                    None => return Ok(unknown_plan_for_type_ref(type_ref)),
                }
            }
            LinkedTypeRef::AppliedNominal { base, arguments } => {
                return applied_nominal_plan(base, arguments, ctx);
            }
            LinkedTypeRef::PackageSchema { .. } => {
                return Err(RuntimeError::InvalidArtifact(
                    "PackageSchema is not admitted in executable linked type plans".to_string(),
                ));
            }
            // A bound type parameter expands to the plan its JSON replacement
            // Value would yield; an unbound one falls through to Unknown via the
            // bridge, exactly as the JSON path leaves it unresolved.
            LinkedTypeRef::TypeParam { name } => {
                if let Some(bound) = ctx.substitution(name) {
                    return Self::from_linked_substituted(bound, ctx);
                }
                return Err(RuntimeError::InvalidArtifact(format!(
                    "linked type plan contains unbound type parameter {name}"
                )));
            }
            LinkedTypeRef::DbObjectSymbol { symbol } => {
                match program_db_object_type_addr(ctx.program, &ctx.current_addr.unit, symbol)? {
                    Some(addr) => return Self::resolve_addr_or_bridge(type_ref, addr, ctx),
                    None => return Ok(unknown_plan_for_type_ref(type_ref)),
                }
            }
            // Function descriptors are not recognised as structural nodes by
            // from_descriptor; the JSON walk may resolve nested refs but the
            // outer kind still yields Unknown, so the bridge is equivalent.
            LinkedTypeRef::Function { .. } => {
                return Ok(unknown_plan_for_type_ref(type_ref));
            }
        };
        Ok(Self {
            label: linked_type_ref_label(type_ref).to_string(),
            named_type_name: linked_type_ref_named_type_name(type_ref),
            identity: RuntimeTypeIdentityPlan::default(),
            node,
        })
    }

    /// Closes a bound type parameter recursively before producing a plan.
    ///
    /// The closed replacement is evaluated without the old substitution frame,
    /// so no type parameter can accidentally be rebound after closure.
    fn from_linked_substituted(bound: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        let substituted = close_linked_type_ref(bound, ctx.substitutions)?;
        Self::from_linked(&substituted, &ctx.without_substitutions())
    }

    /// Resolves a ref that pointed at `addr`: if the descriptor is interned,
    /// process it (recursing into its children one JSON level deeper); otherwise
    /// fall back to the bare-ref bridge (the JSON walk would leave the ref
    /// unresolved -> Unknown).
    fn resolve_addr_or_bridge(
        type_ref: &LinkedTypeRef,
        addr: TypeAddr,
        ctx: &PlanContext,
    ) -> Result<Self> {
        match ctx.program.types.declaration(&addr) {
            // resolving a ref object to its descriptor is one JSON level.
            Some(declaration) => Self::from_linked_declaration(declaration, &ctx.deeper_by(1)),
            None => Ok(unknown_plan_for_type_ref(type_ref)),
        }
    }

    fn from_linked_declaration(declaration: &TypeDeclIr, ctx: &PlanContext) -> Result<Self> {
        if !declaration.type_params.is_empty() {
            return Err(RuntimeError::InvalidArtifact(format!(
                "generic nominal {} requires an applied nominal wrapper with {} arguments",
                declaration.name,
                declaration.type_params.len()
            )));
        }
        let mut plan = Self::from_linked_descriptor(&declaration.descriptor, ctx)?;
        apply_nominal_owner_context(&mut plan, &declaration.name);
        Ok(plan)
    }

    /// Mirrors `resolve_program_descriptor_refs` processing a fetched
    /// `LinkedTypeDescriptor` (already converted to JSON) at `ctx.depth`.
    fn from_linked_descriptor(
        descriptor: &LinkedTypeDescriptor,
        ctx: &PlanContext,
    ) -> Result<Self> {
        // Past the cap the descriptor JSON is returned raw and parsed by
        // no further ref resolution; represent that unresolved descriptor as
        // Unknown directly.
        if ctx.over_depth_cap() {
            return Ok(unknown_plan_for_descriptor(descriptor));
        }
        let node = match descriptor {
            LinkedTypeDescriptor::Record { fields } => RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| {
                        Ok(RuntimeRecordFieldPlan {
                            name: name.clone(),
                            ty: Self::from_linked_ref(field_ty, &ctx.deeper_by(2))?,
                            required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                            identity: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                boundary_record_kind: None,
            },
            LinkedTypeDescriptor::Representation { representation } => {
                RuntimeTypeNode::Representation {
                    type_name: "representation".to_string(),
                    payload: Box::new(Self::from_linked_ref(representation, &ctx.deeper_by(1))?),
                }
            }
            LinkedTypeDescriptor::Alias { target } => {
                RuntimeTypeNode::Alias(Box::new(Self::from_linked_ref(target, &ctx.deeper_by(1))?))
            }
            LinkedTypeDescriptor::Union { branches } => RuntimeTypeNode::Union(
                branches
                    .iter()
                    .map(|branch| linked_named_union_branch_plan(branch, &ctx.deeper_by(2)))
                    .collect::<Result<Vec<_>>>()?,
            ),
            LinkedTypeDescriptor::Interface => {
                return Err(RuntimeError::InvalidArtifact(
                    "interface declaration cannot be materialized as a value type plan".to_string(),
                ));
            }
        };
        Ok(Self {
            label: linked_type_descriptor_label(descriptor).to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node,
        })
    }

    /// Builds the node for a `Builtin` `LinkedTypeRef`. Generic Array/Map
    /// recurse on their args; everything else routes through the JSON path's
    /// builtin recognition so leaf builtins (string/number/.../Json) and any
    /// standard builtin descriptors resolve exactly as `from_descriptor` does.
    #[allow(dead_code)]
    fn builtin_node(
        name: &str,
        args: &[LinkedTypeRef],
        ctx: &PlanContext,
    ) -> Result<RuntimeTypeNode> {
        let input = PlanInput::Linked { name, args };
        if let Some(node) = structural_builtin_node(&input, Some(ctx)) {
            return node;
        }
        if let Some(node) = db_result_node(&input, Some(ctx)) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node(name, args.len()) {
            return node;
        }
        Ok(RuntimeBuiltinShape::of_name(name)
            .and_then(RuntimeBuiltinShape::leaf_node)
            .unwrap_or(RuntimeTypeNode::Unknown))
    }

    fn artifact_builtin_node(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
    ) -> Result<RuntimeTypeNode> {
        let input = PlanInput::Artifact { name, args };
        if let Some(node) = structural_builtin_node(&input, None) {
            return node;
        }
        if let Some(node) = db_result_node(&input, None) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node(name, args.len()) {
            return node;
        }
        Ok(RuntimeBuiltinShape::of_name(name)
            .and_then(RuntimeBuiltinShape::leaf_node)
            .unwrap_or(RuntimeTypeNode::Unknown))
    }

    fn artifact_builtin_node_in_program(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
        ctx: &PlanContext<'_>,
    ) -> Result<RuntimeTypeNode> {
        let input = PlanInput::ArtifactInProgram { name, args };
        if let Some(node) = structural_builtin_node(&input, Some(ctx)) {
            return node;
        }
        if let Some(node) = db_result_node(&input, Some(ctx)) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node(name, args.len()) {
            return node;
        }
        Ok(RuntimeBuiltinShape::of_name(name)
            .and_then(RuntimeBuiltinShape::leaf_node)
            .unwrap_or(RuntimeTypeNode::Unknown))
    }
}
