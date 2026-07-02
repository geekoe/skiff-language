use crate::type_syntax::{
    generic_parts, record_type_fields as record_type_field_texts, split_top_level, string_literal,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    EmptyRecord,
    StringLiteral(String),
    AnyInterface {
        interface: Box<TypeExpr>,
    },
    Named {
        name: String,
        args: Vec<TypeExpr>,
    },
    Nullable(Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Record(Vec<RecordTypeField>),
    Function {
        params: Vec<FunctionTypeParam>,
        return_type: Box<TypeExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTypeField {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeParam {
    pub name: String,
    pub ty: TypeExpr,
}

impl TypeExpr {
    pub fn parse(raw: &str) -> Self {
        let ty = raw.trim();
        if ty.is_empty() {
            return Self::opaque(ty);
        }
        if ty == "{}" {
            return Self::EmptyRecord;
        }
        if let Some(value) = string_literal(ty) {
            return Self::StringLiteral(value);
        }
        if let Some((params, return_type)) = function_type_parts(ty) {
            return Self::Function {
                params,
                return_type: Box::new(Self::parse(return_type)),
            };
        }
        let union = split_top_level(ty, '|');
        if union.len() > 1 {
            return Self::Union(union.into_iter().map(Self::parse).collect());
        }
        if let Some(inner) = ty.strip_suffix('?') {
            return Self::Nullable(Box::new(Self::parse(inner)));
        }
        if let Some(interface) = ty.strip_prefix("any ").map(str::trim) {
            return Self::AnyInterface {
                interface: Box::new(Self::parse(interface)),
            };
        }
        if let Some(fields) = record_type_field_texts(ty) {
            if fields.is_empty() {
                return Self::EmptyRecord;
            }
            return Self::Record(
                fields
                    .into_iter()
                    .map(|field| RecordTypeField {
                        name: field.name.to_string(),
                        ty: Self::parse(field.ty),
                    })
                    .collect(),
            );
        }
        if let Some(parts) = generic_parts(ty) {
            return Self::Named {
                name: parts.root.to_string(),
                args: parts.args.into_iter().map(Self::parse).collect(),
            };
        }
        Self::opaque(ty)
    }

    pub fn parse_lossy(raw: &str) -> Self {
        Self::parse(raw)
    }

    pub fn to_type_string(&self) -> String {
        match self {
            Self::EmptyRecord => "{}".to_string(),
            Self::StringLiteral(value) => serde_json::to_string(value).unwrap_or_else(|_| {
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            }),
            Self::AnyInterface { interface } => {
                format!("any {}", interface.to_type_string())
            }
            Self::Named { name, args } if args.is_empty() => name.clone(),
            Self::Named { name, args } => format!(
                "{name}<{}>",
                args.iter()
                    .map(Self::to_type_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Nullable(inner) => format!("{}?", inner.to_type_string()),
            Self::Union(parts) => parts
                .iter()
                .map(Self::to_type_string)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Record(fields) => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, field.ty.to_type_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Function {
                params,
                return_type,
            } => format!(
                "fn({}) -> {}",
                params
                    .iter()
                    .map(|param| format!("{}: {}", param.name, param.ty.to_type_string()))
                    .collect::<Vec<_>>()
                    .join(", "),
                return_type.to_type_string()
            ),
        }
    }

    pub fn map_named_types<F>(&self, mut map: F) -> Self
    where
        F: FnMut(&str) -> String,
    {
        self.map_named_types_inner(&mut map)
    }

    pub fn contains_function_type(&self) -> bool {
        match self {
            Self::Function { .. } => true,
            Self::Named { args, .. } | Self::Union(args) => {
                args.iter().any(Self::contains_function_type)
            }
            Self::AnyInterface { interface } => interface.contains_function_type(),
            Self::Nullable(inner) => inner.contains_function_type(),
            Self::Record(fields) => fields.iter().any(|field| field.ty.contains_function_type()),
            Self::EmptyRecord | Self::StringLiteral(_) => false,
        }
    }

    pub fn for_each_function_type(&self, mut visit: impl FnMut(&TypeExpr)) {
        self.visit_function_type(&mut visit);
    }

    pub fn for_each_named(&self, mut visit: impl FnMut(&str)) {
        self.visit_named(&mut visit);
    }

    pub fn for_each_named_outside_function_types(&self, mut visit: impl FnMut(&str)) {
        self.visit_named_outside_function_types(&mut visit);
    }

    fn visit_named<'a>(&'a self, visit: &mut impl FnMut(&'a str)) {
        match self {
            Self::Named { name, args } => {
                visit(name);
                for arg in args {
                    arg.visit_named(visit);
                }
            }
            Self::AnyInterface { interface } => interface.visit_named(visit),
            Self::Nullable(inner) => inner.visit_named(visit),
            Self::Union(parts) => {
                for part in parts {
                    part.visit_named(visit);
                }
            }
            Self::Record(fields) => {
                for field in fields {
                    field.ty.visit_named(visit);
                }
            }
            Self::Function {
                params,
                return_type,
            } => {
                for param in params {
                    param.ty.visit_named(visit);
                }
                return_type.visit_named(visit);
            }
            Self::EmptyRecord | Self::StringLiteral(_) => {}
        }
    }

    fn map_named_types_inner<F>(&self, map: &mut F) -> Self
    where
        F: FnMut(&str) -> String,
    {
        match self {
            Self::EmptyRecord => Self::EmptyRecord,
            Self::StringLiteral(value) => Self::StringLiteral(value.clone()),
            Self::AnyInterface { interface } => Self::AnyInterface {
                interface: Box::new(interface.map_named_types_inner(map)),
            },
            Self::Named { name, args } => Self::Named {
                name: map(name),
                args: args
                    .iter()
                    .map(|arg| arg.map_named_types_inner(map))
                    .collect(),
            },
            Self::Nullable(inner) => Self::Nullable(Box::new(inner.map_named_types_inner(map))),
            Self::Union(parts) => Self::Union(
                parts
                    .iter()
                    .map(|part| part.map_named_types_inner(map))
                    .collect(),
            ),
            Self::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|field| RecordTypeField {
                        name: field.name.clone(),
                        ty: field.ty.map_named_types_inner(map),
                    })
                    .collect(),
            ),
            Self::Function {
                params,
                return_type,
            } => Self::Function {
                params: params
                    .iter()
                    .map(|param| FunctionTypeParam {
                        name: param.name.clone(),
                        ty: param.ty.map_named_types_inner(map),
                    })
                    .collect(),
                return_type: Box::new(return_type.map_named_types_inner(map)),
            },
        }
    }

    fn visit_named_outside_function_types<'a>(&'a self, visit: &mut impl FnMut(&'a str)) {
        match self {
            Self::Named { name, args } => {
                visit(name);
                for arg in args {
                    arg.visit_named_outside_function_types(visit);
                }
            }
            Self::AnyInterface { interface } => interface.visit_named_outside_function_types(visit),
            Self::Nullable(inner) => inner.visit_named_outside_function_types(visit),
            Self::Union(parts) => {
                for part in parts {
                    part.visit_named_outside_function_types(visit);
                }
            }
            Self::Record(fields) => {
                for field in fields {
                    field.ty.visit_named_outside_function_types(visit);
                }
            }
            Self::Function { .. } => {}
            Self::EmptyRecord | Self::StringLiteral(_) => {}
        }
    }

    fn visit_function_type(&self, visit: &mut impl FnMut(&TypeExpr)) {
        match self {
            Self::Function { .. } => {
                visit(self);
            }
            Self::Named { args, .. } | Self::Union(args) => {
                for arg in args {
                    arg.visit_function_type(visit);
                }
            }
            Self::AnyInterface { interface } => interface.visit_function_type(visit),
            Self::Nullable(inner) => inner.visit_function_type(visit),
            Self::Record(fields) => {
                for field in fields {
                    field.ty.visit_function_type(visit);
                }
            }
            Self::EmptyRecord | Self::StringLiteral(_) => {}
        }
    }

    fn opaque(ty: &str) -> Self {
        Self::Named {
            name: ty.to_string(),
            args: Vec::new(),
        }
    }
}

fn function_type_parts(ty: &str) -> Option<(Vec<FunctionTypeParam>, &str)> {
    let tail = ty.trim().strip_prefix("fn(")?;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in tail.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' if angle_depth == 0 && brace_depth == 0 && paren_depth == 0 => {
                let after_params = tail[index + ch.len_utf8()..].trim_start();
                let return_type = after_params.strip_prefix("->")?.trim();
                if return_type.is_empty() {
                    return None;
                }
                let params = function_type_params(tail[..index].trim())?;
                return Some((params, return_type));
            }
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
    }

    None
}

fn function_type_params(raw: &str) -> Option<Vec<FunctionTypeParam>> {
    if raw.trim().is_empty() {
        return Some(Vec::new());
    }
    split_top_level(raw, ',')
        .into_iter()
        .map(|param| {
            let param = param.trim();
            if param.is_empty() {
                return None;
            }
            let (name, ty) = split_top_level_once(param, ':')?;
            Some(FunctionTypeParam {
                name: name.to_string(),
                ty: TypeExpr::parse(ty),
            })
        })
        .collect()
}

fn split_top_level_once(input: &str, delimiter: char) -> Option<(&str, &str)> {
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ch if ch == delimiter && angle_depth == 0 && brace_depth == 0 && paren_depth == 0 => {
                return Some((input[..index].trim(), input[index + ch.len_utf8()..].trim()));
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests;
