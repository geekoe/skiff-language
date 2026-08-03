use super::*;

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
    pub packages: &'a [Arc<RuntimeExecutionPackage>],
    pub link_overlay: &'a LinkOverlay,
    pub types: &'a RuntimeTypeContext,
}

impl<'a> ProgramTypeView<'a> {
    pub fn new(
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<RuntimeExecutionPackage>],
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
    ) -> Self {
        Self {
            service_files,
            packages,
            link_overlay,
            types,
        }
    }

    pub fn from_linked_image(program: &'a LinkedProgramImage) -> Self {
        Self::new(
            &program.service_files,
            &program.packages,
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

impl<'a> ProgramTypeView<'a> {
    pub(super) fn package_files(self, slot: usize) -> Option<&'a [Arc<LinkedFileUnit>]> {
        self.packages.get(slot).map(|package| package.files())
    }
}

pub struct PlanContext<'a> {
    pub program: ProgramTypeView<'a>,
    pub current_addr: &'a ExecutableAddr,
    pub depth: usize,
    /// Generic bindings in effect, keyed by type-parameter name. `None` means
    /// "no substitutions" (the common non-generic case) and is allocation-free.
    pub substitutions: Option<&'a BTreeMap<String, LinkedTypeRef>>,
}

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
    pub(super) fn substitution(&self, name: &str) -> Option<&'a LinkedTypeRef> {
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
    pub(super) fn deeper_by(&self, by: usize) -> PlanContext<'a> {
        PlanContext {
            program: self.program,
            current_addr: self.current_addr,
            depth: self.depth + by,
            substitutions: self.substitutions,
        }
    }

    /// Returns a copy of this context with no substitutions in scope. Applied
    /// nominal and type-parameter replacements use it only after recursively
    /// closing the replacement against the current frame.
    pub(super) fn without_substitutions(&self) -> PlanContext<'a> {
        PlanContext {
            program: self.program,
            current_addr: self.current_addr,
            depth: self.depth,
            substitutions: None,
        }
    }

    /// Mirrors `resolve_program_descriptor_refs`'s entry guard: once the JSON
    /// walk passes depth 32 it returns the value unresolved.
    pub(super) fn over_depth_cap(&self) -> bool {
        self.depth > 32
    }
}
