use std::{collections::BTreeMap, sync::Arc};

use crate::error::{Error as RuntimeError, Result};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkOverlay, LinkedFileUnit, LinkedProgramImage,
    LinkedTypeDescriptor, LinkedTypeRef, LiteralIr, PackageRefIr, PackageSymbolRef, PackageUnit,
    ResolvedSymbol, RuntimeTypeContext, ServiceSymbolRef, TypeAddr, TypeDeclIr, UnitAddr,
};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableExpectedAnyInterfacePlan, RuntimeRecoverableExpectedRecordFieldPlan,
    RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan,
    RuntimeRecoverableInterfaceTypeRef, RuntimeRecoverableTypeIdentityRef,
};

pub use skiff_runtime_boundary::type_descriptor::{
    bare_type_name, type_name_root,
};
pub use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
};

/// Resolution context threaded through `RuntimeTypePlan::from_linked`.
///
/// Step 1 only stores what the eventual native resolution path will need: the
/// owning [`LinkedProgramImage`], the current executable address (used to resolve
/// `localType` refs against the current unit/file), and a recursion `depth`
/// mirroring the 32-level cap the JSON path enforces in
/// `resolve_program_descriptor_refs`.
///
/// `substitutions` carries the structured generic type-parameter bindings for
/// the enclosing call (formal-param name -> bound `LinkedTypeRef`). It mirrors
/// the JSON path's `TypeSubstitutions` map, but stays in the LINKED domain: all
/// linked substitution inputs are fully structured (`LinkedTypeRef::TypeParam`,
/// `Builtin`, ...) — there is no bare-string text form to parse — so the
/// string-text substitution branch of the JSON path
/// (`type_text_descriptor_with_substitutions`) is unreachable here. When
/// `from_linked` hits a `TypeParam { name }` that is bound, it recurses on the
/// bound ref with that param SHADOWED (removed) so a self-referential binding
/// terminates exactly like the JSON path's single non-recursive replacement.
#[derive(Clone, Copy)]
pub struct ProgramTypeView<'a> {
    pub service_files: &'a [Arc<LinkedFileUnit>],
    pub packages: &'a [Arc<PackageUnit>],
    pub package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    pub link_overlay: &'a LinkOverlay,
    pub types: &'a RuntimeTypeContext,
}

impl<'a> ProgramTypeView<'a> {
    pub fn new(
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
    ) -> Self {
        Self {
            service_files,
            packages,
            package_files,
            link_overlay,
            types,
        }
    }

    pub fn from_linked_image(program: &'a LinkedProgramImage) -> Self {
        Self::new(
            &program.service_files,
            &program.packages,
            &program.package_files,
            &program.link_overlay,
            &program.types,
        )
    }
}

impl<'a> From<&'a LinkedProgramImage> for ProgramTypeView<'a> {
    fn from(program: &'a LinkedProgramImage) -> Self {
        Self::from_linked_image(program)
    }
}

impl<'a> From<&'a Arc<LinkedProgramImage>> for ProgramTypeView<'a> {
    fn from(program: &'a Arc<LinkedProgramImage>) -> Self {
        Self::from_linked_image(program.as_ref())
    }
}

#[allow(dead_code)]
pub struct PlanContext<'a> {
    pub program: ProgramTypeView<'a>,
    pub current_addr: &'a ExecutableAddr,
    pub depth: usize,
    /// Generic bindings in effect, keyed by type-parameter name. `None` means
    /// "no substitutions" (the common non-generic case) and is allocation-free.
    pub substitutions: Option<&'a BTreeMap<String, LinkedTypeRef>>,
}

#[allow(dead_code)]
impl<'a> PlanContext<'a> {
    pub fn new(program: &'a LinkedProgramImage, current_addr: &'a ExecutableAddr) -> Self {
        Self::from_type_view(ProgramTypeView::from_linked_image(program), current_addr)
    }

    pub fn from_type_view(program: ProgramTypeView<'a>, current_addr: &'a ExecutableAddr) -> Self {
        Self {
            program,
            current_addr,
            depth: 0,
            substitutions: None,
        }
    }

    /// Like [`Self::new`] but carrying generic type-parameter bindings (formal
    /// name -> bound `LinkedTypeRef`). Used by call sites whose expected type
    /// previously had to flow through
    /// `program_type_descriptor_value_with_substitutions` on the `&Value` path.
    pub fn with_substitutions(
        program: &'a LinkedProgramImage,
        current_addr: &'a ExecutableAddr,
        substitutions: &'a BTreeMap<String, LinkedTypeRef>,
    ) -> Self {
        Self::with_substitutions_from_type_view(
            ProgramTypeView::from_linked_image(program),
            current_addr,
            substitutions,
        )
    }

    pub fn with_substitutions_from_type_view(
        program: ProgramTypeView<'a>,
        current_addr: &'a ExecutableAddr,
        substitutions: &'a BTreeMap<String, LinkedTypeRef>,
    ) -> Self {
        Self {
            program,
            current_addr,
            depth: 0,
            substitutions: Some(substitutions),
        }
    }

    /// Looks up the bound `LinkedTypeRef` for a type-parameter name, if any.
    fn substitution(&self, name: &str) -> Option<&'a LinkedTypeRef> {
        self.substitutions.and_then(|map| map.get(name))
    }

    /// Returns a child context with `depth + by`.
    ///
    /// The JSON reference walk (`resolve_program_descriptor_refs`) increments its
    /// recursion depth once per JSON-tree nesting level. `from_linked` mirrors
    /// that walk so the depth-32 truncation guard trips at the *same* node,
    /// reproducing the reference's observable (truncated) plan byte-for-byte.
    /// The increments encode the JSON nesting between a descriptor object and a
    /// child type ref:
    ///   * record/union → object, then the `fields`/`items` container, then the
    ///     child value: two levels (`+2`).
    ///   * builtin generics (`Array`/`Map`) → object, then the `args` array,
    ///     then the element: two levels (`+2`).
    ///   * nullable/alias → object, then the `inner`/`target` value: one level
    ///     (`+1`).
    ///   * resolving a ref object to its interned descriptor: one level (`+1`).
    fn deeper_by(&self, by: usize) -> PlanContext<'a> {
        PlanContext {
            program: self.program,
            current_addr: self.current_addr,
            depth: self.depth + by,
            substitutions: self.substitutions,
        }
    }

    /// Returns a copy of this context with no substitutions in scope, used when
    /// recursing into a position the JSON substitution pass does not descend
    /// into (record `fields` object map, alias `target`, descriptor union
    /// `variants`).
    fn without_substitutions(&self) -> PlanContext<'a> {
        PlanContext {
            program: self.program,
            current_addr: self.current_addr,
            depth: self.depth,
            substitutions: None,
        }
    }

    /// Mirrors `resolve_program_descriptor_refs`'s entry guard: once the JSON
    /// walk passes depth 32 it returns the value unresolved.
    fn over_depth_cap(&self) -> bool {
        self.depth > 32
    }
}

pub trait RuntimeTypePlanLinkedExt: Sized {
    fn from_artifact_type_ref(type_ref: &skiff_artifact_model::TypeRefIr) -> Result<Self>;

    fn from_artifact_type_ref_in_program(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: &LinkedProgramImage,
        current_addr: &ExecutableAddr,
    ) -> Result<Self>;

    fn from_artifact_type_ref_in_type_view(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
    ) -> Result<Self>;

    fn from_artifact_type_ref_in_program_ref(
        type_ref: &skiff_artifact_model::TypeRefIr,
        ctx: &PlanContext<'_>,
    ) -> Result<Self>;

    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_nested_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_substituted(bound: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn resolve_addr_or_bridge(
        type_ref: &LinkedTypeRef,
        addr: TypeAddr,
        ctx: &PlanContext,
    ) -> Result<Self>;

    fn from_linked_declaration(declaration: &TypeDeclIr, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_descriptor(descriptor: &LinkedTypeDescriptor, ctx: &PlanContext)
        -> Result<Self>;

    fn builtin_node(
        name: &str,
        args: &[LinkedTypeRef],
        ctx: &PlanContext,
    ) -> Result<RuntimeTypeNode>;

    fn artifact_builtin_node(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
    ) -> Result<RuntimeTypeNode>;

    fn artifact_builtin_node_in_program(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
        ctx: &PlanContext<'_>,
    ) -> Result<RuntimeTypeNode>;
}

pub trait RuntimeRecoverableExpectedTypePlanLinkedExt: Sized {
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;
}

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
            TypeRefIr::Native { name, args } => Self::artifact_builtin_node(name, args)?,
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
            TypeRefIr::Literal { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
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
            TypeRefIr::Native { name, args } => {
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
            TypeRefIr::Literal { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::LocalType { .. }
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
            LinkedTypeRef::ServiceSymbol { symbol } => {
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
            // A bound type parameter expands to the plan its JSON replacement
            // Value would yield; an unbound one falls through to Unknown via the
            // bridge, exactly as the JSON path leaves it unresolved.
            LinkedTypeRef::TypeParam { name } => {
                if let Some(bound) = ctx.substitution(name) {
                    return Self::from_linked_substituted(bound, ctx);
                }
                return Ok(unknown_plan_for_type_ref(type_ref));
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

    /// Resolves a bound type-parameter to the plan its legacy replacement would
    /// yield.
    ///
    /// `call_type_substitutions` stores each binding as a caller-normalized
    /// `LinkedTypeRef`. The legacy descriptor fallback materializes that ref at
    /// replacement time and clones it in place without recursively applying the
    /// current substitution frame to the replacement internals.
    ///
    /// We reproduce that by recursing `from_linked` on the bound ref at a fresh
    /// depth and without the current call's bindings in scope: the param itself
    /// must not re-expand (so `T -> List<T>` terminates with the inner `T`
    /// unbound -> Unknown), and sibling bindings are not applied inside the
    /// cloned replacement.
    fn from_linked_substituted(bound: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        Self::from_linked(
            bound,
            &PlanContext::from_type_view(ctx.program, ctx.current_addr),
        )
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
        let mut plan = Self::from_linked_descriptor(&declaration.descriptor, ctx)?;
        plan.label = declaration.name.clone();
        plan.named_type_name = Some(declaration.name.clone());
        if let RuntimeTypeNode::Record {
            boundary_record_kind,
            ..
        } = &mut plan.node
        {
            *boundary_record_kind = Some(declaration.name.clone());
        }
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
            // Descriptor `fields` serialise as a JSON object map, which the JSON
            // substitution pass never descends into -> drop substitutions.
            LinkedTypeDescriptor::Record { fields } => RuntimeTypeNode::Record {
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
            // Alias serialises its child under `target`, which the JSON
            // substitution pass does not handle (it only knows `inner`) -> drop.
            LinkedTypeDescriptor::Alias { target } => RuntimeTypeNode::Alias(Box::new(
                Self::from_linked_ref(target, &ctx.without_substitutions().deeper_by(1))?,
            )),
            // Descriptor union serialises its children under `variants`, which
            // the JSON substitution pass does not handle (it knows only the
            // inline-union `items` key) -> drop substitutions.
            LinkedTypeDescriptor::Union { variants } => RuntimeTypeNode::Union(
                variants
                    .iter()
                    .map(|item| {
                        Self::from_linked_ref(item, &ctx.without_substitutions().deeper_by(2))
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            // `external` descriptors are not recognised by from_descriptor.
            LinkedTypeDescriptor::Native { .. } => {
                return Ok(unknown_plan_for_descriptor(descriptor));
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
        if name == "Array" && args.len() == 1 {
            // builtin object -> `args` array -> element value.
            return Ok(RuntimeTypeNode::Array(Box::new(Self::from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?)));
        }
        if name == "Map" && args.len() == 2 {
            return Ok(RuntimeTypeNode::Map {
                key: Box::new(Self::from_linked_ref(&args[0], &ctx.deeper_by(2))?),
                value: Box::new(Self::from_linked_ref(&args[1], &ctx.deeper_by(2))?),
            });
        }
        if bare_type_name(name) == "Stream" && args.len() == 1 {
            return Ok(RuntimeTypeNode::Stream(Box::new(Self::from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?)));
        }
        if let Some(node) = db_result_node_from_linked_parts(name, args, ctx) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node_from_linked_parts(name, args, ctx) {
            return node;
        }
        Ok(match bare_type_name(name) {
            "Json" => RuntimeTypeNode::Json,
            "JsonObject" => RuntimeTypeNode::JsonObject,
            "bytes" => RuntimeTypeNode::Bytes,
            "Date" => RuntimeTypeNode::Date,
            "string" => RuntimeTypeNode::String,
            "bool" | "boolean" => RuntimeTypeNode::Bool,
            "integer" => RuntimeTypeNode::Integer,
            "number" => RuntimeTypeNode::Number,
            "null" | "void" => RuntimeTypeNode::Null,
            _ => RuntimeTypeNode::Unknown,
        })
    }

    fn artifact_builtin_node(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
    ) -> Result<RuntimeTypeNode> {
        if bare_type_name(name) == "Array" && args.len() == 1 {
            return Ok(RuntimeTypeNode::Array(Box::new(
                Self::from_artifact_type_ref(&args[0])?,
            )));
        }
        if bare_type_name(name) == "Map" && args.len() == 2 {
            return Ok(RuntimeTypeNode::Map {
                key: Box::new(Self::from_artifact_type_ref(&args[0])?),
                value: Box::new(Self::from_artifact_type_ref(&args[1])?),
            });
        }
        if bare_type_name(name) == "Stream" && args.len() == 1 {
            return Ok(RuntimeTypeNode::Stream(Box::new(
                Self::from_artifact_type_ref(&args[0])?,
            )));
        }
        if let Some(node) = db_result_node_from_parts(name, args) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node_from_artifact_parts(name, args) {
            return node;
        }
        Ok(match bare_type_name(name) {
            "Json" => RuntimeTypeNode::Json,
            "JsonObject" => RuntimeTypeNode::JsonObject,
            "bytes" => RuntimeTypeNode::Bytes,
            "Date" => RuntimeTypeNode::Date,
            "string" => RuntimeTypeNode::String,
            "bool" | "boolean" => RuntimeTypeNode::Bool,
            "integer" => RuntimeTypeNode::Integer,
            "number" => RuntimeTypeNode::Number,
            "null" | "void" => RuntimeTypeNode::Null,
            _ => RuntimeTypeNode::Unknown,
        })
    }

    fn artifact_builtin_node_in_program(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
        ctx: &PlanContext<'_>,
    ) -> Result<RuntimeTypeNode> {
        if bare_type_name(name) == "Array" && args.len() == 1 {
            return Ok(RuntimeTypeNode::Array(Box::new(
                Self::from_artifact_type_ref_in_program_ref(&args[0], ctx)?,
            )));
        }
        if bare_type_name(name) == "Map" && args.len() == 2 {
            return Ok(RuntimeTypeNode::Map {
                key: Box::new(Self::from_artifact_type_ref_in_program_ref(&args[0], ctx)?),
                value: Box::new(Self::from_artifact_type_ref_in_program_ref(&args[1], ctx)?),
            });
        }
        if bare_type_name(name) == "Stream" && args.len() == 1 {
            return Ok(RuntimeTypeNode::Stream(Box::new(
                Self::from_artifact_type_ref_in_program_ref(&args[0], ctx)?,
            )));
        }
        if let Some(node) = db_result_node_from_artifact_parts_in_program(name, args, ctx) {
            return node;
        }
        if let Some(node) = std_runtime_builtin_node_from_artifact_parts_in_program(name, args, ctx)
        {
            return node;
        }
        Ok(match bare_type_name(name) {
            "Json" => RuntimeTypeNode::Json,
            "JsonObject" => RuntimeTypeNode::JsonObject,
            "bytes" => RuntimeTypeNode::Bytes,
            "Date" => RuntimeTypeNode::Date,
            "string" => RuntimeTypeNode::String,
            "bool" | "boolean" => RuntimeTypeNode::Bool,
            "integer" => RuntimeTypeNode::Integer,
            "number" => RuntimeTypeNode::Number,
            "null" | "void" => RuntimeTypeNode::Null,
            _ => RuntimeTypeNode::Unknown,
        })
    }
}

impl RuntimeRecoverableExpectedTypePlanLinkedExt for RuntimeRecoverableExpectedTypePlan {
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        recoverable_expected_from_linked(type_ref, ctx)
    }

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self> {
        recoverable_expected_from_linked_ref(type_ref, ctx)
    }
}

fn unknown_plan_for_type_ref(type_ref: &LinkedTypeRef) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: linked_type_ref_label(type_ref).to_string(),
        named_type_name: linked_type_ref_named_type_name(type_ref),
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Unknown,
    }
}

fn unknown_plan_for_descriptor(descriptor: &LinkedTypeDescriptor) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: linked_type_descriptor_label(descriptor).to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Unknown,
    }
}

fn unresolved_recoverable_expected_from_type_ref(
    type_ref: &LinkedTypeRef,
) -> RuntimeRecoverableExpectedTypePlan {
    let runtime_plan = unknown_plan_for_type_ref(type_ref);
    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
        &runtime_plan,
    )
}

fn unresolved_recoverable_expected_from_descriptor(
    descriptor: &LinkedTypeDescriptor,
) -> RuntimeRecoverableExpectedTypePlan {
    let runtime_plan = unknown_plan_for_descriptor(descriptor);
    RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
        &runtime_plan,
    )
}

pub fn linked_interface_instantiation_runtime_id(
    interface: &skiff_runtime_linked_program::LinkedInterfaceInstantiationRef,
) -> String {
    if interface.canonical_type_args.is_empty() {
        return interface.interface_abi_id.clone();
    }
    canonical_json_string(
        serde_json::to_value(interface).unwrap_or_else(|_| serde_json::Value::Null),
    )
}

pub fn linked_type_ref_runtime_key(type_ref: &LinkedTypeRef) -> String {
    canonical_json_string(
        serde_json::to_value(type_ref).unwrap_or_else(|_| serde_json::Value::Null),
    )
}

/// Stable recoverable interface projection identity for an expected `any I`.
///
/// Non-generic interfaces intentionally keep the compiler recoverable metadata
/// shape (`interface:{interfaceAbiId}`). Generic instantiations include the
/// canonical instantiation JSON so different `I<T>` projections cannot collide.
pub fn recoverable_interface_projection_identity(
    interface: &skiff_runtime_linked_program::LinkedInterfaceInstantiationRef,
) -> String {
    if interface.canonical_type_args.is_empty() {
        return format!("interface:{}", interface.interface_abi_id);
    }
    format!(
        "interface:{}",
        canonical_json_string(
            serde_json::to_value(interface).unwrap_or_else(|_| serde_json::Value::Null),
        )
    )
}

fn recoverable_expected_from_linked(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    let runtime_plan = RuntimeTypePlan::from_linked(type_ref, ctx)?;
    let mut expected =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
            &runtime_plan,
        );
    expected.node = recoverable_expected_node_from_linked(type_ref, ctx)?;
    if let LinkedTypeRef::AnyInterface { interface } = type_ref {
        let interface_identity = linked_interface_instantiation_runtime_id(interface);
        expected.identity = Some(RuntimeRecoverableTypeIdentityRef::Interface(
            RuntimeRecoverableInterfaceTypeRef {
                interface_identity: interface_identity.clone(),
            },
        ));
        expected.label = format!("any {interface_identity}");
    }
    Ok(expected)
}

fn recoverable_expected_from_linked_ref(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => {
            if let Some(bound) = ctx.substitution(name) {
                return recoverable_expected_from_linked_ref(bound, &ctx.without_substitutions());
            }
        }
        LinkedTypeRef::Address { addr } => {
            return recoverable_expected_resolve_addr_or_bridge(type_ref, addr.clone(), ctx);
        }
        LinkedTypeRef::LocalType { type_index } => {
            let addr = TypeAddr {
                unit: ctx.current_addr.unit.clone(),
                file: ctx.current_addr.file.clone(),
                type_index: *type_index,
            };
            return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
        }
        LinkedTypeRef::ServiceSymbol { symbol } => {
            if let Some(addr) =
                program_service_symbol_type_addr(ctx.program, &ctx.current_addr.unit, symbol)?
            {
                return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
            }
        }
        LinkedTypeRef::PackageSymbol { symbol } => {
            if let Some(addr) = program_package_type_addr(ctx.program, symbol) {
                return recoverable_expected_resolve_addr_or_bridge(type_ref, addr, ctx);
            }
        }
        _ => {}
    }
    recoverable_expected_from_linked(type_ref, ctx)
}

fn recoverable_expected_resolve_addr_or_bridge(
    type_ref: &LinkedTypeRef,
    addr: TypeAddr,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    match ctx.program.types.declaration(&addr) {
        Some(declaration) => {
            recoverable_expected_from_linked_declaration(declaration, &ctx.deeper_by(1))
        }
        None => Ok(unresolved_recoverable_expected_from_type_ref(type_ref)),
    }
}

fn recoverable_expected_from_linked_declaration(
    declaration: &TypeDeclIr,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    let mut expected = recoverable_expected_from_linked_descriptor(&declaration.descriptor, ctx)?;
    expected.label = declaration.name.clone();
    if let RuntimeRecoverableExpectedTypeNode::Record {
        boundary_record_kind,
        ..
    } = &mut expected.node
    {
        *boundary_record_kind = Some(declaration.name.clone());
    }
    Ok(expected)
}

fn recoverable_expected_from_linked_descriptor(
    descriptor: &LinkedTypeDescriptor,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    if ctx.over_depth_cap() {
        return Ok(unresolved_recoverable_expected_from_descriptor(descriptor));
    }

    let node = match descriptor {
        LinkedTypeDescriptor::Record { fields } => RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    Ok(RuntimeRecoverableExpectedRecordFieldPlan {
                        name: name.clone(),
                        ty: recoverable_expected_from_linked_ref(
                            field_ty,
                            &ctx.without_substitutions().deeper_by(2),
                        )?,
                        required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            boundary_record_kind: None,
        },
        LinkedTypeDescriptor::Alias { target } => RuntimeRecoverableExpectedTypeNode::Alias {
            target: Box::new(recoverable_expected_from_linked_ref(
                target,
                &ctx.without_substitutions().deeper_by(1),
            )?),
        },
        LinkedTypeDescriptor::Union { variants } => RuntimeRecoverableExpectedTypeNode::Union {
            items: variants
                .iter()
                .map(|item| {
                    recoverable_expected_from_linked_ref(
                        item,
                        &ctx.without_substitutions().deeper_by(2),
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeDescriptor::Native { .. } => {
            return Ok(unresolved_recoverable_expected_from_descriptor(descriptor));
        }
    };
    Ok(RuntimeRecoverableExpectedTypePlan {
        label: linked_type_descriptor_label(descriptor).to_string(),
        identity: None,
        node,
    })
}

fn recoverable_expected_node_from_linked(
    type_ref: &LinkedTypeRef,
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypeNode> {
    let node = match type_ref {
        LinkedTypeRef::Native { name, args } => recoverable_expected_builtin_node(name, args, ctx)?,
        LinkedTypeRef::Record { fields } => RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    Ok(RuntimeRecoverableExpectedRecordFieldPlan {
                        name: name.clone(),
                        ty: recoverable_expected_from_linked_ref(
                            field_ty,
                            &ctx.without_substitutions().deeper_by(2),
                        )?,
                        required: !matches!(field_ty, LinkedTypeRef::Nullable { .. }),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            boundary_record_kind: None,
        },
        LinkedTypeRef::Union { items } => RuntimeRecoverableExpectedTypeNode::Union {
            items: items
                .iter()
                .map(|item| recoverable_expected_from_linked_ref(item, &ctx.deeper_by(2)))
                .collect::<Result<Vec<_>>>()?,
        },
        LinkedTypeRef::Nullable { inner } => RuntimeRecoverableExpectedTypeNode::Nullable {
            inner: Box::new(recoverable_expected_from_linked_ref(
                inner,
                &ctx.deeper_by(1),
            )?),
        },
        LinkedTypeRef::AnyInterface { interface } => {
            RuntimeRecoverableExpectedTypeNode::AnyInterface {
                expected: RuntimeRecoverableExpectedAnyInterfacePlan::new(
                    linked_interface_instantiation_runtime_id(interface),
                    recoverable_interface_projection_identity(interface),
                ),
            }
        }
        LinkedTypeRef::Literal { value } => match value {
            LiteralIr::String { value } => RuntimeRecoverableExpectedTypeNode::LiteralString {
                value: value.clone(),
            },
            _ => RuntimeRecoverableExpectedTypeNode::Unresolved {
                diagnostic_label: "literal".to_string(),
            },
        },
        LinkedTypeRef::TypeParam { name } => {
            if let Some(bound) = ctx.substitution(name) {
                return Ok(recoverable_expected_from_linked_ref(
                    bound,
                    &ctx.without_substitutions(),
                )?
                .node);
            }
            RuntimeRecoverableExpectedTypeNode::Unresolved {
                diagnostic_label: format!("typeParam {name}"),
            }
        }
        LinkedTypeRef::Function { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. } => {
            let runtime_plan = RuntimeTypePlan::from_linked(type_ref, ctx)?;
            return Ok(
                RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                    &runtime_plan,
                )
                .node,
            );
        }
    };
    Ok(node)
}

fn recoverable_expected_builtin_node(
    name: &str,
    args: &[LinkedTypeRef],
    ctx: &PlanContext,
) -> Result<RuntimeRecoverableExpectedTypeNode> {
    if name == "Array" && args.len() == 1 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Array {
            item: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
        });
    }
    if name == "Map" && args.len() == 2 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Map {
            key: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
            value: Box::new(recoverable_expected_from_linked_ref(
                &args[1],
                &ctx.deeper_by(2),
            )?),
        });
    }
    if bare_type_name(name) == "Stream" && args.len() == 1 {
        return Ok(RuntimeRecoverableExpectedTypeNode::Stream {
            item: Box::new(recoverable_expected_from_linked_ref(
                &args[0],
                &ctx.deeper_by(2),
            )?),
        });
    }

    Ok(match bare_type_name(name) {
        "Json" => RuntimeRecoverableExpectedTypeNode::Json,
        "JsonObject" => RuntimeRecoverableExpectedTypeNode::JsonObject,
        "bytes" => RuntimeRecoverableExpectedTypeNode::Bytes,
        "Date" => RuntimeRecoverableExpectedTypeNode::Date,
        "string" => RuntimeRecoverableExpectedTypeNode::String,
        "bool" | "boolean" => RuntimeRecoverableExpectedTypeNode::Bool,
        "integer" => RuntimeRecoverableExpectedTypeNode::Integer,
        "number" => RuntimeRecoverableExpectedTypeNode::Number,
        "null" | "void" => RuntimeRecoverableExpectedTypeNode::Null,
        _ => RuntimeRecoverableExpectedTypeNode::Unresolved {
            diagnostic_label: name.to_string(),
        },
    })
}

fn canonical_json_string(value: serde_json::Value) -> String {
    let canonical = canonical_json_value(value);
    serde_json::to_string(&canonical).unwrap_or_else(|_| "null".to_string())
}

fn canonical_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical_json_value).collect())
        }
        serde_json::Value::Object(object) => {
            let mut sorted = std::collections::BTreeMap::new();
            for (key, value) in object {
                sorted.insert(key, canonical_json_value(value));
            }
            let mut object = serde_json::Map::new();
            for (key, value) in sorted {
                object.insert(key, value);
            }
            serde_json::Value::Object(object)
        }
        other => other,
    }
}

/// Diagnostic kind label for the top-level "not linked" error, matching
/// `program_type_ref_kind` in `interpreter/program_types.rs`.
fn linked_type_ref_kind(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
        LinkedTypeRef::Address { .. } => "address",
        LinkedTypeRef::Native { .. } => "builtin",
        LinkedTypeRef::Record { .. } => "record",
        LinkedTypeRef::Union { .. } => "union",
        LinkedTypeRef::Nullable { .. } => "nullable",
        LinkedTypeRef::Literal { .. } => "literal",
        LinkedTypeRef::TypeParam { .. } => "typeParam",
        LinkedTypeRef::Function { .. } => "function",
        LinkedTypeRef::DbObjectSymbol { .. } => "dbObjectSymbol",
        LinkedTypeRef::AnyInterface { .. } => "anyInterface",
    }
}

fn linked_type_ref_label(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::Native { .. } => "builtin",
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
        LinkedTypeRef::Address { .. } => "address",
        LinkedTypeRef::Record { .. } => "record",
        LinkedTypeRef::Union { .. } => "union",
        LinkedTypeRef::Nullable { .. } => "nullable",
        LinkedTypeRef::Literal { .. } => "literal",
        LinkedTypeRef::TypeParam { .. } => "typeParam",
        LinkedTypeRef::Function { .. } => "function",
        LinkedTypeRef::DbObjectSymbol { .. } => "dbObjectSymbol",
        LinkedTypeRef::AnyInterface { .. } => "anyInterface",
    }
}

fn linked_type_ref_named_type_name(type_ref: &LinkedTypeRef) -> Option<String> {
    match type_ref {
        LinkedTypeRef::Native { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn linked_type_descriptor_label(descriptor: &LinkedTypeDescriptor) -> &'static str {
    match descriptor {
        LinkedTypeDescriptor::Record { .. } => "record",
        LinkedTypeDescriptor::Alias { .. } => "alias",
        LinkedTypeDescriptor::Union { .. } => "union",
        LinkedTypeDescriptor::Native { .. } => "external",
    }
}

fn artifact_type_ref_label(type_ref: &skiff_artifact_model::TypeRefIr) -> &'static str {
    use skiff_artifact_model::TypeRefIr;
    match type_ref {
        TypeRefIr::Native { .. } => "builtin",
        TypeRefIr::LocalType { .. } => "localType",
        TypeRefIr::ServiceSymbol { .. } => "serviceSymbol",
        TypeRefIr::PackageSymbol { .. } => "packageSymbol",
        TypeRefIr::DbObjectSymbol { .. } => "dbObjectSymbol",
        TypeRefIr::Record { .. } => "record",
        TypeRefIr::Union { .. } => "union",
        TypeRefIr::Nullable { .. } => "nullable",
        TypeRefIr::Literal { .. } => "literal",
        TypeRefIr::TypeParam { .. } => "typeParam",
        TypeRefIr::Function { .. } => "function",
        TypeRefIr::AnyInterface { .. } => "anyInterface",
    }
}

fn artifact_type_ref_named_type_name(type_ref: &skiff_artifact_model::TypeRefIr) -> Option<String> {
    match type_ref {
        skiff_artifact_model::TypeRefIr::Native { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Resolves a package symbol to its interned `TypeAddr` via the link overlay,
/// mirroring `program_package_type_addr` in `interpreter/program_types.rs`.
fn program_package_type_addr(
    program: ProgramTypeView<'_>,
    symbol: &PackageSymbolRef,
) -> Option<TypeAddr> {
    let resolved = match &symbol.package {
        PackageRefIr::PackageId { package_id } => program
            .link_overlay
            .resolved_package_id_symbol(package_id, &symbol.symbol_path),
        PackageRefIr::Dependency { dependency_ref } => program
            .link_overlay
            .resolved_package_dependency_ref_symbol(dependency_ref, &symbol.symbol_path),
    }?;
    match resolved {
        ResolvedSymbol::Type { addr } => Some(addr.clone()),
        _ => None,
    }
}

fn program_db_object_type_addr(
    program: ProgramTypeView<'_>,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    match unit {
        UnitAddr::Service => {
            let local = program_local_type_addr(&program.service_files, unit, symbol)?;
            Ok(local.or_else(|| {
                program
                    .types
                    .exported_service_type(&symbol.module_path, &symbol.symbol)
                    .cloned()
            }))
        }
        UnitAddr::Package(slot) => {
            let Some(files) = program.package_files.get(*slot) else {
                return Ok(None);
            };
            program_local_type_addr(files, unit, symbol)
        }
    }
}

fn program_service_symbol_type_addr(
    program: ProgramTypeView<'_>,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    if let Some(addr) = program
        .types
        .exported_service_type(&symbol.module_path, &symbol.symbol)
        .cloned()
    {
        return Ok(Some(addr));
    }
    let UnitAddr::Package(slot) = unit else {
        return Ok(None);
    };
    let Some(files) = program.package_files.get(*slot) else {
        return Ok(None);
    };
    program_local_type_addr(files, unit, symbol)
}

fn program_local_type_addr(
    files: &[Arc<LinkedFileUnit>],
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    let mut resolved = None;
    for (file_index, file) in files.iter().enumerate() {
        if file.module_path != symbol.module_path {
            continue;
        }
        let file_addr = FileAddr::LoadedFileIndex(file_index);
        if let Some(declaration) = file.declarations.types.get(&symbol.symbol) {
            merge_type_addr(
                &mut resolved,
                TypeAddr {
                    unit: unit.clone(),
                    file: file_addr.clone(),
                    type_index: declaration.type_index,
                },
                unit,
                symbol,
            )?;
        }
        if let Some(type_index) = file.link_targets.types.get(&symbol.symbol) {
            merge_type_addr(
                &mut resolved,
                TypeAddr {
                    unit: unit.clone(),
                    file: file_addr.clone(),
                    type_index: *type_index,
                },
                unit,
                symbol,
            )?;
        }
    }
    Ok(resolved)
}

fn merge_type_addr(
    resolved: &mut Option<TypeAddr>,
    candidate: TypeAddr,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<()> {
    match resolved {
        Some(existing) if *existing != candidate => Err(RuntimeError::InvalidArtifact(format!(
            "ambiguous type symbol {}.{} in {unit}: {existing} and {candidate}",
            symbol.module_path, symbol.symbol
        ))),
        Some(_) => Ok(()),
        None => {
            *resolved = Some(candidate);
            Ok(())
        }
    }
}

/// Inner plan for a synthesized `{kind:"builtin", name, args/fields}` descriptor:
/// `label="builtin"`, `named_type_name=Some(name)`.
fn builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

/// Inner plan for a synthesized leaf builtin (string/integer/bytes/Json).
fn leaf_builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    builtin_plan(name, node)
}

fn std_field(name: &str, ty: RuntimeTypePlan) -> RuntimeRecordFieldPlan {
    let required = !matches!(ty.node, RuntimeTypeNode::Nullable(_));
    RuntimeRecordFieldPlan {
        name: name.to_string(),
        ty,
        required,
        identity: None,
    }
}

enum StdRuntimeTypeArg<'a> {
    Artifact(&'a skiff_artifact_model::TypeRefIr),
    ArtifactInProgram(&'a skiff_artifact_model::TypeRefIr, &'a PlanContext<'a>),
    Linked(&'a LinkedTypeRef, &'a PlanContext<'a>),
}

impl StdRuntimeTypeArg<'_> {
    fn plan(&self) -> Result<RuntimeTypePlan> {
        match self {
            Self::Artifact(type_ref) => RuntimeTypePlan::from_artifact_type_ref(type_ref),
            Self::ArtifactInProgram(type_ref, ctx) => {
                RuntimeTypePlan::from_artifact_type_ref_in_program_ref(type_ref, ctx)
            }
            Self::Linked(type_ref, ctx) => {
                RuntimeTypePlan::from_linked_ref(type_ref, &ctx.deeper_by(2))
            }
        }
    }
}

fn leaf_string_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("string", RuntimeTypeNode::String)
}

fn leaf_integer_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("integer", RuntimeTypeNode::Integer)
}

fn leaf_bytes_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("bytes", RuntimeTypeNode::Bytes)
}

fn std_record_plan(name: &str, fields: Vec<RuntimeRecordFieldPlan>) -> RuntimeTypePlan {
    builtin_plan(
        name,
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(name.to_string()),
        },
    )
}

fn std_union_plan(name: &str, items: Vec<RuntimeTypePlan>) -> RuntimeTypePlan {
    builtin_plan(name, RuntimeTypeNode::Union(items))
}

fn std_nullable_plan(inner: RuntimeTypePlan) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "nullable".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Nullable(Box::new(inner)),
    }
}

fn std_array_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Array", RuntimeTypeNode::Array(Box::new(item)))
}

fn std_stream_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Stream", RuntimeTypeNode::Stream(Box::new(item)))
}

fn std_literal_string_plan(value: &str) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "literal".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::LiteralString(value.to_string()),
    }
}

fn std_http_header_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpHeader",
        vec![
            std_field("name", leaf_string_plan()),
            std_field("value", leaf_string_plan()),
        ],
    )
}

fn std_http_client_request_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientRequest",
        vec![
            std_field("method", leaf_string_plan()),
            std_field("url", leaf_string_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_nullable_plan(leaf_bytes_plan())),
            std_field("timeoutMs", std_nullable_plan(leaf_integer_plan())),
        ],
    )
}

fn std_http_client_response_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientResponse",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", leaf_bytes_plan()),
        ],
    )
}

fn std_http_client_stream_handle_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientStreamHandle",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_stream_plan(leaf_bytes_plan())),
        ],
    )
}

fn std_websocket_connection_plan(context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketConnection",
        vec![
            std_field("id", leaf_string_plan()),
            std_field("businessIdentity", std_nullable_plan(leaf_string_plan())),
            std_field("context", context),
        ],
    )
}

fn std_websocket_text_message_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.TextConnectionMessage",
        vec![
            std_field("tag", std_literal_string_plan("text")),
            std_field("text", leaf_string_plan()),
        ],
    )
}

fn std_websocket_binary_message_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.BinaryConnectionMessage",
        vec![
            std_field("tag", std_literal_string_plan("binary")),
            std_field("base64", leaf_string_plan()),
        ],
    )
}

fn std_websocket_connection_message_plan() -> RuntimeTypePlan {
    builtin_plan(
        "std.websocket.ConnectionMessage",
        RuntimeTypeNode::Representation {
            type_name: "std.websocket.ConnectionMessage".to_string(),
            payload: Box::new(std_union_plan(
                "std.websocket.ConnectionMessage",
                vec![
                    std_websocket_text_message_plan(),
                    std_websocket_binary_message_plan(),
                ],
            )),
        },
    )
}

fn std_websocket_connect_result_plan(name: &str, context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_union_plan(
        name,
        vec![
            std_record_plan(
                "std.websocket.WebSocketConnectAccept",
                vec![
                    std_field("tag", std_literal_string_plan("accept")),
                    std_field("context", context),
                    std_field("businessIdentity", std_nullable_plan(leaf_string_plan())),
                    std_field(
                        "connectionPolicy",
                        std_nullable_plan(std_websocket_connection_policy_plan()),
                    ),
                ],
            ),
            std_record_plan(
                "std.websocket.WebSocketConnectReject",
                vec![
                    std_field("tag", std_literal_string_plan("reject")),
                    std_field("code", leaf_integer_plan()),
                    std_field("reason", leaf_string_plan()),
                ],
            ),
        ],
    )
}

fn std_websocket_connection_policy_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketConnectionPolicy",
        vec![
            std_field("maxConnections", leaf_integer_plan()),
            std_field(
                "overflow",
                std_union_plan(
                    "std.websocket.WebSocketConnectionPolicy.overflow",
                    vec![
                        std_literal_string_plan("close-oldest"),
                        std_literal_string_plan("reject-new"),
                    ],
                ),
            ),
            std_field("closeCode", std_nullable_plan(leaf_integer_plan())),
            std_field("closeReason", std_nullable_plan(leaf_string_plan())),
        ],
    )
}

fn std_websocket_receive_event_plan(context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketReceiveEvent",
        vec![
            std_field("connection", std_websocket_connection_plan(context)),
            std_field("message", std_websocket_connection_message_plan()),
        ],
    )
}

fn std_runtime_builtin_node(
    name: &str,
    args: &[StdRuntimeTypeArg<'_>],
) -> Option<Result<RuntimeTypeNode>> {
    let root = type_name_root(name);
    let bare = bare_type_name(root);
    let node = match bare {
        "HttpClientRequest" if args.is_empty() && root == "std.http.HttpClientRequest" => {
            std_http_client_request_plan().node
        }
        "HttpClientResponse" if args.is_empty() && root == "std.http.HttpClientResponse" => {
            std_http_client_response_plan().node
        }
        "HttpClientStreamHandle"
            if args.is_empty() && root == "std.http.HttpClientStreamHandle" =>
        {
            std_http_client_stream_handle_plan().node
        }
        "ConnectionMessage" if args.is_empty() && root == "std.websocket.ConnectionMessage" => {
            std_websocket_connection_message_plan().node
        }
        "WebSocketConnection"
            if args.len() == 1
                && matches!(
                    root,
                    "WebSocketConnection" | "std.websocket.WebSocketConnection"
                ) =>
        {
            let context = match args[0].plan() {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_connection_plan(context).node
        }
        "WebSocketConnectResult"
            if args.len() == 1
                && matches!(
                    root,
                    "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
                ) =>
        {
            let context = match args[0].plan() {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_connect_result_plan(root, context).node
        }
        "WebSocketReceiveEvent"
            if args.len() == 1
                && matches!(
                    root,
                    "WebSocketReceiveEvent" | "std.websocket.WebSocketReceiveEvent"
                ) =>
        {
            let context = match args[0].plan() {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_receive_event_plan(context).node
        }
        _ => return None,
    };
    Some(Ok(node))
}

fn std_runtime_builtin_node_from_artifact_parts(
    name: &str,
    args: &[skiff_artifact_model::TypeRefIr],
) -> Option<Result<RuntimeTypeNode>> {
    let args = args
        .iter()
        .map(StdRuntimeTypeArg::Artifact)
        .collect::<Vec<_>>();
    std_runtime_builtin_node(name, &args)
}

fn std_runtime_builtin_node_from_artifact_parts_in_program<'a>(
    name: &str,
    args: &'a [skiff_artifact_model::TypeRefIr],
    ctx: &'a PlanContext<'a>,
) -> Option<Result<RuntimeTypeNode>> {
    let args = args
        .iter()
        .map(|arg| StdRuntimeTypeArg::ArtifactInProgram(arg, ctx))
        .collect::<Vec<_>>();
    std_runtime_builtin_node(name, &args)
}

fn std_runtime_builtin_node_from_linked_parts<'a>(
    name: &str,
    args: &'a [LinkedTypeRef],
    ctx: &'a PlanContext<'a>,
) -> Option<Result<RuntimeTypeNode>> {
    let args = args
        .iter()
        .map(|arg| StdRuntimeTypeArg::Linked(arg, ctx))
        .collect::<Vec<_>>();
    std_runtime_builtin_node(name, &args)
}

pub(crate) fn native_builtin_fallback_plan(name: &str) -> Result<RuntimeTypePlan> {
    if name == "Duration" || name == "std.time.Duration" {
        return Ok(RuntimeTypePlan {
            label: "representation".to_string(),
            named_type_name: None,
            identity: RuntimeTypeIdentityPlan::default(),
            node: RuntimeTypeNode::Representation {
                type_name: "std.time.Duration".to_string(),
                payload: Box::new(leaf_integer_plan()),
            },
        });
    }
    if let Some(node) = std_runtime_builtin_node(name, &[]) {
        return Ok(builtin_plan(name, node?));
    }
    Ok(builtin_plan(
        name,
        match bare_type_name(name) {
            "Json" => RuntimeTypeNode::Json,
            "JsonObject" => RuntimeTypeNode::JsonObject,
            "bytes" => RuntimeTypeNode::Bytes,
            "Date" => RuntimeTypeNode::Date,
            "string" => RuntimeTypeNode::String,
            "bool" | "boolean" => RuntimeTypeNode::Bool,
            "integer" => RuntimeTypeNode::Integer,
            "number" => RuntimeTypeNode::Number,
            "null" | "void" => RuntimeTypeNode::Null,
            _ => RuntimeTypeNode::Unknown,
        },
    ))
}

fn db_result_node_from_parts(
    root: &str,
    args: &[skiff_artifact_model::TypeRefIr],
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_artifact_type_ref(&args[0]).map(|value| {
                    RuntimeTypeNode::Record {
                        fields: vec![
                            RuntimeRecordFieldPlan {
                                name: "value".to_string(),
                                ty: value,
                                required: true,
                                identity: None,
                            },
                            std_field("inserted", leaf_builtin_plan("bool", RuntimeTypeNode::Bool)),
                        ],
                        boundary_record_kind: Some(root.to_string()),
                    }
                }),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}

fn db_result_node_from_artifact_parts_in_program(
    root: &str,
    args: &[skiff_artifact_model::TypeRefIr],
    ctx: &PlanContext<'_>,
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_artifact_type_ref_in_program_ref(&args[0], ctx).map(
                    |value| RuntimeTypeNode::Record {
                        fields: vec![
                            RuntimeRecordFieldPlan {
                                name: "value".to_string(),
                                ty: value,
                                required: true,
                                identity: None,
                            },
                            std_field("inserted", leaf_builtin_plan("bool", RuntimeTypeNode::Bool)),
                        ],
                        boundary_record_kind: Some(root.to_string()),
                    },
                ),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}

fn db_result_node_from_linked_parts(
    root: &str,
    args: &[LinkedTypeRef],
    ctx: &PlanContext<'_>,
) -> Option<Result<RuntimeTypeNode>> {
    let node = match bare_type_name(root) {
        "DbInsertManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "insertedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpdateManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![
                std_field(
                    "matchedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
                std_field(
                    "modifiedCount",
                    leaf_builtin_plan("number", RuntimeTypeNode::Number),
                ),
            ],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbDeleteManyResult" if args.is_empty() => RuntimeTypeNode::Record {
            fields: vec![std_field(
                "deletedCount",
                leaf_builtin_plan("number", RuntimeTypeNode::Number),
            )],
            boundary_record_kind: Some(root.to_string()),
        },
        "DbUpsertResult" if args.len() == 1 => {
            return Some(
                RuntimeTypePlan::from_linked_ref(&args[0], &ctx.deeper_by(2)).map(|value| {
                    RuntimeTypeNode::Record {
                        fields: vec![
                            RuntimeRecordFieldPlan {
                                name: "value".to_string(),
                                ty: value,
                                required: true,
                                identity: None,
                            },
                            std_field("inserted", leaf_builtin_plan("bool", RuntimeTypeNode::Bool)),
                        ],
                        boundary_record_kind: Some(root.to_string()),
                    }
                }),
            );
        }
        _ => return None,
    };
    Some(Ok(node))
}

#[cfg(test)]
mod recoverable_expected_plan_tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_runtime_linked_program::{LinkedInterfaceInstantiationRef, PackageUnit};

    use super::*;

    fn empty_ctx<'a>(
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
        addr: &'a ExecutableAddr,
    ) -> PlanContext<'a> {
        PlanContext::from_type_view(
            ProgramTypeView::new(service_files, packages, package_files, link_overlay, types),
            addr,
        )
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }
    }

    #[test]
    fn linked_recoverable_expected_plan_preserves_nested_any_interface() {
        let service_files = Vec::new();
        let packages = Vec::new();
        let package_files = Vec::new();
        let link_overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let addr = ExecutableAddr::service(0, 0);
        let ctx = empty_ctx(
            &service_files,
            &packages,
            &package_files,
            &link_overlay,
            &types,
            &addr,
        );
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: "pkg.ToolProvider".to_string(),
            canonical_type_args: Vec::new(),
        };
        let ty = LinkedTypeRef::Record {
            fields: BTreeMap::from([(
                "provider".to_string(),
                LinkedTypeRef::AnyInterface {
                    interface: interface.clone(),
                },
            )]),
        };

        let expected = RuntimeRecoverableExpectedTypePlan::from_linked(&ty, &ctx)
            .expect("recoverable expected plan should build");

        let RuntimeRecoverableExpectedTypeNode::Record { fields, .. } = expected.node else {
            panic!("expected record node");
        };
        let RuntimeRecoverableExpectedTypeNode::AnyInterface { expected } = &fields[0].ty.node
        else {
            panic!("nested any interface must not collapse to unresolved/unknown");
        };
        assert_eq!(expected.interface_identity, "pkg.ToolProvider");
        assert_eq!(
            expected.method_projection_identity,
            "interface:pkg.ToolProvider"
        );
    }

    #[test]
    fn generic_interface_projection_identity_includes_canonical_args() {
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: "pkg.Provider".to_string(),
            canonical_type_args: vec![string_type()],
        };

        let projection = recoverable_interface_projection_identity(&interface);

        assert_ne!(projection, "interface:pkg.Provider");
        assert!(projection.starts_with("interface:{"));
        assert!(projection.contains("canonicalTypeArgs"));
    }
}
