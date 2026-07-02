#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParts<'a> {
    pub root: &'a str,
    pub inner: &'a str,
    pub args: Vec<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordTypeFieldText<'a> {
    pub name: &'a str,
    pub ty: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTypeFieldParseError<'a> {
    NotRecordType,
    InvalidField(&'a str),
}

pub fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
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
                parts.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(input[start..].trim());
    parts
}

pub fn parse_record_type_fields(
    ty: &str,
) -> Result<Vec<RecordTypeFieldText<'_>>, RecordTypeFieldParseError<'_>> {
    let Some(inner) = ty
        .trim()
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .map(str::trim)
    else {
        return Err(RecordTypeFieldParseError::NotRecordType);
    };
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    split_top_level(inner, ',')
        .into_iter()
        .map(|field| {
            split_record_type_field(field).ok_or(RecordTypeFieldParseError::InvalidField(field))
        })
        .collect()
}

pub fn record_type_fields(ty: &str) -> Option<Vec<RecordTypeFieldText<'_>>> {
    parse_record_type_fields(ty).ok()
}

pub fn split_record_type_field(field: &str) -> Option<RecordTypeFieldText<'_>> {
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in field.char_indices() {
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
            ':' if angle_depth == 0 && brace_depth == 0 => {
                return Some(RecordTypeFieldText {
                    name: field[..index].trim(),
                    ty: field[index + 1..].trim(),
                });
            }
            _ => {}
        }
    }

    None
}

pub fn generic_parts(input: &str) -> Option<GenericParts<'_>> {
    let input = input.trim();
    let mut open = None;
    let mut close = None;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
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

        if ch == '>' && input[..index].ends_with('-') {
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '<' if brace_depth == 0 => {
                if angle_depth == 0 {
                    open.get_or_insert(index);
                }
                angle_depth += 1;
            }
            '>' if brace_depth == 0 && angle_depth > 0 => {
                angle_depth -= 1;
                if angle_depth == 0 {
                    close = Some(index);
                }
            }
            _ => {}
        }
    }

    let open = open?;
    let close = close?;
    if angle_depth != 0 || close + 1 != input.len() || close <= open {
        return None;
    }

    let inner = input[open + 1..close].trim();
    Some(GenericParts {
        root: input[..open].trim(),
        inner,
        args: split_top_level(inner, ','),
    })
}

pub fn generic_args(input: &str) -> Option<Vec<&str>> {
    generic_parts(input).map(|parts| parts.args)
}

pub fn generic_inner<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let parts = generic_parts(input)?;
    (parts.root == name).then_some(parts.inner)
}

pub fn string_literal(input: &str) -> Option<String> {
    let input = input.trim();
    if input.starts_with('"') && input.ends_with('"') {
        serde_json::from_str::<String>(input).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
