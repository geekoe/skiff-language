use skiff_runtime_model::type_plan::RuntimeTypePlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryUse {
    TypedJson,
    JsonValueProjection,
    RuntimeBinary,
    HttpRequest,
    HttpResponse,
    NativeArg,
    NativeReturn,
    #[allow(dead_code)]
    ConfigValue,
    DbResultDecode,
    #[allow(dead_code)]
    DbWriteProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BoundaryDirection {
    Decode,
    Encode,
    Coerce,
    Project,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BoundaryConversionPlan {
    expected: RuntimeTypePlan,
    use_case: BoundaryUse,
    direction: BoundaryDirection,
}

#[allow(dead_code)]
impl BoundaryConversionPlan {
    pub fn new(
        expected: RuntimeTypePlan,
        use_case: BoundaryUse,
        direction: BoundaryDirection,
    ) -> Self {
        Self {
            expected,
            use_case,
            direction,
        }
    }

    pub fn expected(&self) -> &RuntimeTypePlan {
        &self.expected
    }

    pub fn use_case(&self) -> BoundaryUse {
        self.use_case
    }

    pub fn direction(&self) -> BoundaryDirection {
        self.direction
    }
}
