export function scanRuntimeExecutionBoundarySource(source, language) {
  if (!['rust', 'typescript'].includes(language)) {
    throw new Error(`unsupported execution-boundary lexer language ${String(language)}`);
  }
  const output = source.split('');
  const tokens = [];
  let index = 0;
  while (index < source.length) {
    const comment = commentToken(source, index, language);
    if (comment) {
      tokens.push(comment);
      maskRange(output, comment.start, comment.end);
      index = comment.end;
      continue;
    }
    const literal = language === 'rust'
      ? rustLiteralToken(source, index)
      : typeScriptLiteralToken(source, index)
        ?? typeScriptRegexpToken(source, index, tokens);
    if (literal) {
      const { expressions = [], ...literalEntry } = literal;
      tokens.push(literalEntry);
      maskRange(output, literal.start, literal.end);
      for (const expression of expressions) {
        const nested = scanRuntimeExecutionBoundarySource(
          source.slice(expression.start, expression.end),
          'typescript',
        );
        for (let offset = 0; offset < nested.code.length; offset += 1) {
          output[expression.start + offset] = nested.code[offset];
        }
        tokens.push(...nested.tokens.map((entry) => ({
          ...entry,
          end: entry.end + expression.start,
          start: entry.start + expression.start,
        })));
      }
      index = literal.end;
      continue;
    }
    const identifier = /^[A-Za-z_$][A-Za-z0-9_$]*/.exec(source.slice(index));
    if (identifier) {
      const value = identifier[0];
      tokens.push({
        end: index + value.length,
        kind: KEYWORDS[language].has(value) ? 'keyword' : 'identifier',
        start: index,
        value,
      });
      index += value.length;
      continue;
    }
    const number = /^(?:0[xob][0-9A-Fa-f_]+|[0-9][0-9A-Za-z_.]*)/.exec(source.slice(index));
    if (number) {
      tokens.push({ end: index + number[0].length, kind: 'number', start: index, value: number[0] });
      index += number[0].length;
      continue;
    }
    if (!/\s/.test(source[index])) {
      const value = punctuationAt(source, index);
      tokens.push({ end: index + value.length, kind: 'punctuation', start: index, value });
      index += value.length;
      continue;
    }
    index += 1;
  }
  return { code: output.join(''), tokens: Object.freeze(tokens.map(Object.freeze)) };
}

const KEYWORDS = Object.freeze({
  rust: new Set(['as', 'async', 'await', 'const', 'else', 'enum', 'fn', 'for', 'if', 'impl', 'let', 'match', 'mod', 'pub', 'return', 'self', 'struct', 'trait', 'use', 'where', 'while']),
  typescript: new Set([
    'async', 'await', 'case', 'class', 'const', 'default', 'else', 'export', 'if', 'let',
    'private', 'protected', 'public', 'return', 'static', 'switch', 'throw',
  ]),
});

function commentToken(source, index, language) {
  if (source.startsWith('//', index)) {
    const newline = source.indexOf('\n', index + 2);
    const end = newline === -1 ? source.length : newline;
    return token('comment', source, index, end, 'line-comment');
  }
  if (!source.startsWith('/*', index)) {
    return undefined;
  }
  let cursor = index + 2;
  let depth = 1;
  while (cursor < source.length && depth > 0) {
    if (language === 'rust' && source.startsWith('/*', cursor)) {
      depth += 1;
      cursor += 2;
    } else if (source.startsWith('*/', cursor)) {
      depth -= 1;
      cursor += 2;
    } else {
      cursor += 1;
    }
  }
  return token('comment', source, index, cursor, 'block-comment');
}

function rustLiteralToken(source, index) {
  const raw = /^(?:(b|c)?r)(#{0,255})"/.exec(source.slice(index));
  if (raw) {
    const terminator = `"${raw[2]}`;
    const contentStart = index + raw[0].length;
    const close = source.indexOf(terminator, contentStart);
    const end = close === -1 ? source.length : close + terminator.length;
    return literalToken(
      source,
      index,
      end,
      raw[1] ? `${raw[1]}-raw-string` : 'raw-string',
      source.slice(contentStart, close === -1 ? source.length : close),
    );
  }
  for (const [prefix, literalKind] of [['b"', 'byte-string'], ['c"', 'c-string'], ['"', 'string']]) {
    if (source.startsWith(prefix, index)) {
      const end = quotedLiteralEnd(source, index + prefix.length - 1, '"');
      return literalToken(source, index, end, literalKind, decodeEscapedLiteral(
        source.slice(index + prefix.length, Math.max(index + prefix.length, end - 1)),
      ));
    }
  }
  if (source.startsWith("b'", index)) {
    const end = rustCharLiteralEnd(source, index + 1);
    return end === undefined
      ? undefined
      : literalToken(source, index, end, 'byte-char', source.slice(index + 2, end - 1));
  }
  if (source[index] === "'") {
    const end = rustCharLiteralEnd(source, index);
    return end === undefined
      ? undefined
      : literalToken(source, index, end, 'char', source.slice(index + 1, end - 1));
  }
  return undefined;
}

function typeScriptLiteralToken(source, index) {
  if (source[index] === "'" || source[index] === '"') {
    const quote = source[index];
    const end = quotedLiteralEnd(source, index, quote);
    return literalToken(
      source,
      index,
      end,
      'string',
      decodeEscapedLiteral(source.slice(index + 1, Math.max(index + 1, end - 1))),
    );
  }
  if (source[index] === '`') {
    const template = typeScriptTemplateRange(source, index);
    return {
      ...literalToken(
        source,
        index,
        template.end,
        'template',
        source.slice(index + 1, Math.max(index + 1, template.end - 1)),
      ),
      expressions: template.expressions,
    };
  }
  return undefined;
}

function typeScriptRegexpToken(source, index, tokens) {
  if (source[index] !== '/' || !typeScriptCanStartRegexp(tokens)) {
    return undefined;
  }
  const end = typeScriptRegexpEnd(source, index);
  return end === undefined
    ? undefined
    : literalToken(source, index, end, 'regexp', source.slice(index + 1, end));
}

function typeScriptCanStartRegexp(tokens) {
  const previous = tokens.findLast(({ kind }) => kind !== 'comment');
  if (!previous) {
    return true;
  }
  if (previous.kind === 'keyword') {
    return TYPESCRIPT_REGEXP_PREFIX_KEYWORDS.has(previous.value);
  }
  if (['identifier', 'literal', 'number'].includes(previous.kind)) {
    return false;
  }
  return ![')', ']', '}', '++', '--', '.', '?.'].includes(previous.value);
}

function typeScriptTemplateRange(source, start) {
  const expressions = [];
  let cursor = start + 1;
  while (cursor < source.length) {
    if (source[cursor] === '\\') {
      cursor += Math.min(2, source.length - cursor);
      continue;
    }
    if (source[cursor] === '`') {
      return { end: cursor + 1, expressions };
    }
    if (source.startsWith('${', cursor)) {
      const expressionStart = cursor + 2;
      const expressionEnd = typeScriptInterpolationEnd(source, expressionStart);
      expressions.push({ end: expressionEnd, start: expressionStart });
      if (expressionEnd >= source.length) {
        return { end: source.length, expressions };
      }
      cursor = expressionEnd + 1;
      continue;
    }
    cursor += 1;
  }
  return { end: source.length, expressions };
}

function typeScriptInterpolationEnd(source, start) {
  let braceDepth = 1;
  let canStartRegexp = true;
  let cursor = start;
  while (cursor < source.length) {
    if (/\s/.test(source[cursor])) {
      cursor += 1;
      continue;
    }
    const comment = commentToken(source, cursor, 'typescript');
    if (comment) {
      cursor = comment.end;
      continue;
    }
    if (source[cursor] === "'" || source[cursor] === '"') {
      cursor = quotedLiteralEnd(source, cursor, source[cursor]);
      canStartRegexp = false;
      continue;
    }
    if (source[cursor] === '`') {
      cursor = typeScriptTemplateRange(source, cursor).end;
      canStartRegexp = false;
      continue;
    }
    if (source[cursor] === '/' && canStartRegexp) {
      const regexpEnd = typeScriptRegexpEnd(source, cursor);
      if (regexpEnd !== undefined) {
        cursor = regexpEnd;
        canStartRegexp = false;
        continue;
      }
    }
    const identifier = /^[A-Za-z_$][A-Za-z0-9_$]*/.exec(source.slice(cursor));
    if (identifier) {
      cursor += identifier[0].length;
      canStartRegexp = TYPESCRIPT_REGEXP_PREFIX_KEYWORDS.has(identifier[0]);
      continue;
    }
    const number = /^(?:0[xob][0-9A-Fa-f_]+|[0-9][0-9A-Za-z_.]*)/.exec(source.slice(cursor));
    if (number) {
      cursor += number[0].length;
      canStartRegexp = false;
      continue;
    }
    if (source[cursor] === '{') {
      braceDepth += 1;
      cursor += 1;
      canStartRegexp = true;
      continue;
    }
    if (source[cursor] === '}') {
      braceDepth -= 1;
      if (braceDepth === 0) {
        return cursor;
      }
      cursor += 1;
      canStartRegexp = false;
      continue;
    }
    const punctuation = punctuationAt(source, cursor);
    cursor += punctuation.length;
    canStartRegexp = ![')', ']', '++', '--', '.', '?.'].includes(punctuation);
  }
  return source.length;
}

const TYPESCRIPT_REGEXP_PREFIX_KEYWORDS = new Set([
  'await', 'case', 'delete', 'do', 'else', 'in', 'instanceof', 'new', 'of', 'return',
  'throw', 'typeof', 'void', 'yield',
]);

function typeScriptRegexpEnd(source, start) {
  let escaped = false;
  let inCharacterClass = false;
  for (let cursor = start + 1; cursor < source.length; cursor += 1) {
    const character = source[cursor];
    if (!escaped && character === '\n') {
      return undefined;
    }
    if (!escaped && character === '[') {
      inCharacterClass = true;
    } else if (!escaped && character === ']') {
      inCharacterClass = false;
    } else if (!escaped && character === '/' && !inCharacterClass) {
      let end = cursor + 1;
      while (/[A-Za-z]/.test(source[end] ?? '')) {
        end += 1;
      }
      return end;
    }
    if (!escaped && character === '\\') {
      escaped = true;
    } else {
      escaped = false;
    }
  }
  return undefined;
}

function quotedLiteralEnd(source, quoteIndex, quote) {
  let escaped = false;
  for (let cursor = quoteIndex + 1; cursor < source.length; cursor += 1) {
    if (!escaped && source[cursor] === quote) {
      return cursor + 1;
    }
    if (!escaped && source[cursor] === '\\') {
      escaped = true;
    } else {
      escaped = false;
    }
  }
  return source.length;
}

function rustCharLiteralEnd(source, quoteIndex) {
  let cursor = quoteIndex + 1;
  if (source[cursor] === '\\') {
    cursor += 2;
    if (source[cursor - 1] === 'u' && source[cursor] === '{') {
      const close = source.indexOf('}', cursor + 1);
      cursor = close === -1 ? source.length : close + 1;
    } else if (source[cursor - 1] === 'x') {
      cursor += 2;
    }
  } else {
    const point = source.codePointAt(cursor);
    if (point === undefined || source[cursor] === '\n') {
      return undefined;
    }
    cursor += point > 0xFFFF ? 2 : 1;
  }
  return source[cursor] === "'" ? cursor + 1 : undefined;
}

function decodeEscapedLiteral(value) {
  return value.replace(/\\([\\'"`nrt])/g, (_match, escaped) => ({
    '\\': '\\',
    "'": "'",
    '"': '"',
    '`': '`',
    n: '\n',
    r: '\r',
    t: '\t',
  })[escaped]);
}

function literalToken(source, start, end, literalKind, value) {
  return { end, kind: 'literal', literalKind, raw: source.slice(start, end), start, value };
}

function token(kind, source, start, end, tokenKind) {
  return { end, kind, start, tokenKind, value: source.slice(start, end) };
}

function punctuationAt(source, index) {
  for (const value of [
    '===', '!==', '**=', '=>', '==', '!=', '&&', '||', '??', '?.', '++', '--',
    '+=', '-=', '*=', '/=', '%=', '**', '::', '->', '..',
  ]) {
    if (source.startsWith(value, index)) {
      return value;
    }
  }
  return source[index];
}

function maskRange(output, start, end) {
  for (let index = start; index < end; index += 1) {
    if (output[index] !== '\n') {
      output[index] = ' ';
    }
  }
}
