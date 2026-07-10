export function parseYamlStringScalar(rawValue) {
  const value = rawValue.trim();
  if (!value || value.startsWith('#')) {
    return '';
  }
  if (value.startsWith('"')) {
    return parseDoubleQuotedYamlScalar(value);
  }
  if (value.startsWith("'")) {
    return parseSingleQuotedYamlScalar(value);
  }
  return value.replace(/\s+#.*$/, '').trim();
}

export function yamlStringScalarHasContent(rawValue) {
  return parseYamlStringScalar(rawValue).length > 0;
}

export function parseSimpleYamlObject(source, label) {
  const root = {};
  const stack = [{ indent: -1, value: root }];
  const lines = source.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index];
    const uncommented = stripYamlComment(rawLine);
    if (uncommented.trim().length === 0) {
      continue;
    }
    const indent = uncommented.match(/^ */)[0].length;
    const trimmed = uncommented.trim();
    const match = trimmed.match(/^([A-Za-z][A-Za-z0-9_-]*):(?:\s*(.*))?$/);
    if (!match) {
      throw new Error(`${label}:${index + 1} unsupported YAML syntax`);
    }
    while (stack.length > 1 && indent <= stack[stack.length - 1].indent) {
      stack.pop();
    }
    const parent = stack[stack.length - 1];
    if (indent <= parent.indent) {
      throw new Error(`${label}:${index + 1} invalid indentation`);
    }
    const key = match[1];
    const rawValue = match[2] ?? '';
    if (rawValue.trim().length === 0) {
      const nested = {};
      parent.value[key] = nested;
      stack.push({ indent, value: nested });
      continue;
    }
    parent.value[key] = parseSimpleYamlObjectScalar(rawValue);
  }
  return root;
}

export function stripYamlComment(line) {
  let quote = null;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (quote === '"') {
      if (escaped) {
        escaped = false;
      } else if (char === '\\') {
        escaped = true;
      } else if (char === '"') {
        quote = null;
      }
      continue;
    }
    if (quote === "'") {
      if (char === "'") {
        if (line[index + 1] === "'") {
          index += 1;
        } else {
          quote = null;
        }
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === '#' && (index === 0 || /\s/.test(line[index - 1]))) {
      return line.slice(0, index);
    }
  }
  return line;
}

function parseSimpleYamlObjectScalar(rawValue) {
  const value = rawValue.trim();
  if (value === 'true') {
    return true;
  }
  if (value === 'false') {
    return false;
  }
  if (/^-?\d+$/.test(value)) {
    return Number(value);
  }
  return parseYamlStringScalar(value);
}

function parseDoubleQuotedYamlScalar(value) {
  let result = '';
  let escaped = false;
  for (let index = 1; index < value.length; index += 1) {
    const char = value[index];
    if (escaped) {
      result += ({ n: '\n', r: '\r', t: '\t', '"': '"', '\\': '\\' })[char] ?? char;
      escaped = false;
      continue;
    }
    if (char === '\\') {
      escaped = true;
      continue;
    }
    if (char === '"') {
      return result;
    }
    result += char;
  }
  return result;
}

function parseSingleQuotedYamlScalar(value) {
  let result = '';
  for (let index = 1; index < value.length; index += 1) {
    const char = value[index];
    if (char === "'") {
      if (value[index + 1] === "'") {
        result += "'";
        index += 1;
        continue;
      }
      return result;
    }
    result += char;
  }
  return result;
}
