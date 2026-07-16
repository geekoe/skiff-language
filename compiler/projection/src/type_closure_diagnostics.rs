use skiff_compiler_core::type_closure::{TypeClosureTrace, TypeClosureTraceSegment};

pub(crate) fn type_closure_trace_segments(trace: &TypeClosureTrace) -> Vec<String> {
    trace
        .segments()
        .iter()
        .map(|segment| match segment {
            TypeClosureTraceSegment::NativeArg { name, index } => {
                format!("{name} type argument {index}")
            }
            TypeClosureTraceSegment::RecordField { name }
            | TypeClosureTraceSegment::DeclarationField { name } => format!("field {name}"),
            TypeClosureTraceSegment::UnionItem { index } => format!("union item {index}"),
            TypeClosureTraceSegment::NullableInner => "nullable inner".to_string(),
            TypeClosureTraceSegment::AnyInterfaceTypeArg { index } => {
                format!("any interface type argument {index}")
            }
            TypeClosureTraceSegment::FunctionParam { name, index } => {
                format!("function param {name}#{index}")
            }
            TypeClosureTraceSegment::FunctionReturn => "function return".to_string(),
            TypeClosureTraceSegment::Nominal { module_path, name } => {
                if module_path.is_empty() {
                    format!("type {name}")
                } else {
                    format!("type {module_path}.{name}")
                }
            }
            TypeClosureTraceSegment::AliasTarget => "alias target".to_string(),
            TypeClosureTraceSegment::DeclarationVariant { index } => {
                format!("variant {index}")
            }
        })
        .collect()
}

pub(crate) fn type_closure_trace_suffix(trace: &TypeClosureTrace) -> String {
    let segments = type_closure_trace_segments(trace);
    if segments.is_empty() {
        String::new()
    } else {
        format!(" via {}", segments.join(" -> "))
    }
}
