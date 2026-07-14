const CHILD_PROCESS_MODULES = new Set(['node:child_process', 'child_process']);

export function scanCommandExecutionSource(source, path = '<source>') {
  const { tokens, comments } = tokenizeJavaScript(source);
  const bypasses = [];
  const imports = [];
  const importBindingTokens = new Set();

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token.value === 'export') {
      const descriptor = declarationModuleSpecifier(tokens, index, { allowSideEffect: false });
      const module = descriptor?.module;
      if (module?.hasEscape) {
        bypasses.push(`${path}:${token.line} child_process policy forbids escaped re-export module specifiers`);
      }
      if (module?.type === 'string' && CHILD_PROCESS_MODULES.has(module.value)) {
        bypasses.push(`${path}:${token.line} child_process re-export is forbidden`);
      }
      continue;
    }
    if (token.value === 'require' && tokens[index + 1]?.value === '(') {
      const module = tokens[index + 2];
      if (isCallModuleSpecifier(module) && module.hasEscape) {
        bypasses.push(`${path}:${token.line} child_process policy forbids escaped require() module specifiers`);
      } else if (isCallModuleSpecifier(module) && CHILD_PROCESS_MODULES.has(module.value)) {
        bypasses.push(`${path}:${token.line} child_process require() is forbidden`);
      }
      continue;
    }
    if (token.value !== 'import') {
      continue;
    }
    if (tokens[index + 1]?.value === '(') {
      const module = tokens[index + 2];
      if (isCallModuleSpecifier(module) && module.hasEscape) {
        bypasses.push(`${path}:${token.line} child_process policy forbids escaped dynamic import() module specifiers`);
      } else if (isCallModuleSpecifier(module) && CHILD_PROCESS_MODULES.has(module.value)) {
        bypasses.push(`${path}:${token.line} dynamic child_process import() is forbidden`);
      }
      continue;
    }
    if (tokens[index + 1]?.value === '.') {
      continue;
    }

    const descriptor = declarationModuleSpecifier(tokens, index, { allowSideEffect: true });
    if (descriptor === null) {
      continue;
    }
    const { fromIndex, moduleIndex, module } = descriptor;
    if (module.hasEscape) {
      bypasses.push(`${path}:${token.line} child_process policy forbids escaped import module specifiers`);
      index = moduleIndex;
      continue;
    }
    if (module?.type !== 'string' || !CHILD_PROCESS_MODULES.has(module.value)) {
      index = moduleIndex;
      continue;
    }
    if (module.value !== 'node:child_process') {
      bypasses.push(`${path}:${token.line} bare child_process imports are forbidden`);
    }
    if (fromIndex === -1) {
      bypasses.push(`${path}:${token.line} bare child_process side-effect import is forbidden`);
      index = moduleIndex;
      continue;
    }

    const bindings = tokens.slice(index + 1, fromIndex);
    if (bindings[0]?.value !== '{' || bindings.at(-1)?.value !== '}') {
      bypasses.push(`${path}:${token.line} child_process default/namespace import is forbidden`);
      index = moduleIndex;
      continue;
    }
    let bindingIndex = index + 2;
    while (bindingIndex < fromIndex - 1) {
      const imported = tokens[bindingIndex];
      if (imported?.value === ',') {
        bindingIndex += 1;
        continue;
      }
      if (imported?.type !== 'identifier') {
        bypasses.push(`${path}:${imported?.line ?? token.line} invalid child_process import binding`);
        bindingIndex += 1;
        continue;
      }
      let local = imported;
      if (tokens[bindingIndex + 1]?.value === 'as') {
        local = tokens[bindingIndex + 2];
        bindingIndex += 3;
      } else {
        bindingIndex += 1;
      }
      if (local?.type !== 'identifier') {
        bypasses.push(`${path}:${imported.line} invalid child_process local alias`);
        continue;
      }
      imports.push({
        importedSymbol: imported.value,
        localAlias: local.value,
        line: imported.line,
        bindingToken: local.start,
      });
      importBindingTokens.add(local.start);
    }
    index = moduleIndex;
  }

  const functions = namedFunctionRanges(tokens);
  const markers = comments
    .map((comment) => {
      const match = comment.text.match(/^\s*child-process-owner:\s*([a-z0-9][a-z0-9-]*)\s*$/);
      return match ? { ownerId: match[1], line: comment.line, start: comment.start } : null;
    })
    .filter(Boolean);
  const calls = [];
  const references = [];
  const usedMarkers = new Set();
  const importedAliases = new Set(imports.map((entry) => entry.localAlias));

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (
      token.type !== 'identifier'
      || !importedAliases.has(token.value)
      || importBindingTokens.has(token.start)
    ) {
      continue;
    }
    if (tokens[index + 1]?.value !== '(') {
      references.push({ localAlias: token.value, line: token.line });
      continue;
    }
    const marker = markers.find((candidate) => candidate.line === token.line - 1);
    if (marker !== undefined) {
      usedMarkers.add(marker.start);
    }
    calls.push({
      localAlias: token.value,
      line: token.line,
      ownerId: marker?.ownerId ?? null,
      ownerFunction: enclosingFunction(functions, token.start),
    });
  }

  return {
    imports,
    calls,
    references,
    bypasses,
    unusedMarkers: markers.filter((marker) => !usedMarkers.has(marker.start)),
  };
}

function namedFunctionRanges(tokens) {
  const bracePairs = matchingPairs(tokens, '{', '}');
  const parenthesisPairs = matchingPairs(tokens, '(', ')');
  const ranges = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value !== 'function') {
      continue;
    }
    let nameIndex = index + 1;
    if (tokens[nameIndex]?.value === '*') {
      nameIndex += 1;
    }
    if (tokens[nameIndex]?.type !== 'identifier') {
      continue;
    }
    const parametersIndex = findToken(tokens, '(', nameIndex + 1, tokens.length);
    const parametersEnd = parenthesisPairs.get(parametersIndex);
    if (parametersIndex === -1 || parametersEnd === undefined) {
      continue;
    }
    const bodyIndex = findToken(tokens, '{', parametersEnd + 1, tokens.length);
    const closeIndex = bracePairs.get(bodyIndex);
    if (bodyIndex === -1 || closeIndex === undefined) {
      continue;
    }
    ranges.push({
      name: tokens[nameIndex].value,
      start: tokens[bodyIndex].start,
      end: tokens[closeIndex].end,
    });
  }
  return ranges;
}

function enclosingFunction(functions, offset) {
  const matches = functions.filter((range) => range.start < offset && offset < range.end);
  matches.sort((left, right) => (left.end - left.start) - (right.end - right.start));
  return matches[0]?.name ?? null;
}

function matchingPairs(tokens, open, close) {
  const stack = [];
  const pairs = new Map();
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value === open) {
      stack.push(index);
    } else if (tokens[index].value === close && stack.length > 0) {
      pairs.set(stack.pop(), index);
    }
  }
  return pairs;
}

function declarationModuleSpecifier(tokens, start, { allowSideEffect }) {
  const direct = tokens[start + 1];
  if (allowSideEffect && direct?.type === 'string') {
    return { fromIndex: -1, moduleIndex: start + 1, module: direct };
  }

  let braceDepth = 0;
  for (let index = start + 1; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (braceDepth === 0 && token.value === 'from') {
      const module = tokens[index + 1];
      return module?.type === 'string'
        ? { fromIndex: index, moduleIndex: index + 1, module }
        : null;
    }
    if (braceDepth === 0 && token.value === ';') {
      return null;
    }
    if (
      braceDepth === 0
      && index > start + 1
      && (token.value === 'import' || token.value === 'export')
    ) {
      return null;
    }
    if (token.value === '{') braceDepth += 1;
    if (token.value === '}' && braceDepth > 0) braceDepth -= 1;
  }
  return null;
}

function isCallModuleSpecifier(token) {
  return token?.type === 'string' || token?.type === 'static-template';
}

function findToken(tokens, value, start, end) {
  for (let index = start; index < end; index += 1) {
    if (tokens[index].value === value) {
      return index;
    }
  }
  return -1;
}

function tokenizeJavaScript(source) {
  const tokens = [];
  const comments = [];
  let index = 0;
  let line = 1;

  function scanCode(stopAtTemplateBrace = false) {
    let braceDepth = 0;
    while (index < source.length) {
      const character = source[index];
      if (stopAtTemplateBrace && character === '}' && braceDepth === 0) {
        index += 1;
        return;
      }
      if (/\s/.test(character)) {
        if (character === '\n') line += 1;
        index += 1;
        continue;
      }
      if (index === 0 && source.startsWith('#!', index)) {
        scanLineComment(2);
        continue;
      }
      if (source.startsWith('//', index)) {
        scanLineComment(2);
        continue;
      }
      if (source.startsWith('/*', index)) {
        scanBlockComment();
        continue;
      }
      if (character === '\'' || character === '"') {
        scanString(character);
        continue;
      }
      if (character === '`') {
        scanTemplate();
        continue;
      }
      if (isIdentifierStart(character)) {
        scanIdentifier();
        continue;
      }
      if (character === '/' && shouldStartRegex(tokens.at(-1))) {
        scanRegex();
        continue;
      }
      const start = index;
      const tokenLine = line;
      const pair = source.slice(index, index + 2);
      if (['=>', '?.', '==', '!=', '&&', '||', '??', '++', '--', '+=', '-=', '**'].includes(pair)) {
        index += 2;
        tokens.push({ type: 'punctuator', value: pair, start, end: index, line: tokenLine });
        continue;
      }
      index += 1;
      tokens.push({ type: 'punctuator', value: character, start, end: index, line: tokenLine });
      if (character === '{') braceDepth += 1;
      if (character === '}' && braceDepth > 0) braceDepth -= 1;
    }
  }

  function scanLineComment(prefixLength) {
    const start = index;
    const tokenLine = line;
    index += prefixLength;
    const contentStart = index;
    while (index < source.length && source[index] !== '\n') index += 1;
    comments.push({ text: source.slice(contentStart, index), start, line: tokenLine });
  }

  function scanBlockComment() {
    const start = index;
    const tokenLine = line;
    index += 2;
    const contentStart = index;
    while (index < source.length && !source.startsWith('*/', index)) {
      if (source[index] === '\n') line += 1;
      index += 1;
    }
    comments.push({ text: source.slice(contentStart, index), start, line: tokenLine });
    index = Math.min(source.length, index + 2);
  }

  function scanString(quote) {
    const start = index;
    const tokenLine = line;
    let hasEscape = false;
    index += 1;
    let value = '';
    while (index < source.length) {
      const character = source[index];
      if (character === '\\') {
        hasEscape = true;
        if (index + 1 < source.length) {
          value += source[index + 1];
          index += 2;
        } else {
          index += 1;
        }
        continue;
      }
      if (character === quote) {
        index += 1;
        break;
      }
      if (character === '\n') line += 1;
      value += character;
      index += 1;
    }
    tokens.push({ type: 'string', value, hasEscape, start, end: index, line: tokenLine });
  }

  function scanTemplate() {
    const start = index;
    const tokenLine = line;
    let hasEscape = false;
    let hasInterpolation = false;
    let value = '';
    index += 1;
    while (index < source.length) {
      if (source[index] === '\\') {
        hasEscape = true;
        if (index + 1 < source.length) {
          value += source[index + 1];
          index += 2;
        } else {
          index += 1;
        }
        continue;
      }
      if (source[index] === '`') {
        index += 1;
        if (!hasInterpolation) {
          tokens.push({
            type: 'static-template',
            value,
            hasEscape,
            start,
            end: index,
            line: tokenLine,
          });
        }
        return;
      }
      if (source.startsWith('${', index)) {
        hasInterpolation = true;
        index += 2;
        scanCode(true);
        continue;
      }
      if (source[index] === '\n') line += 1;
      value += source[index];
      index += 1;
    }
  }

  function scanIdentifier() {
    const start = index;
    const tokenLine = line;
    index += 1;
    while (index < source.length && isIdentifierPart(source[index])) index += 1;
    tokens.push({
      type: 'identifier',
      value: source.slice(start, index),
      start,
      end: index,
      line: tokenLine,
    });
  }

  function scanRegex() {
    const start = index;
    const tokenLine = line;
    index += 1;
    let inClass = false;
    while (index < source.length) {
      const character = source[index];
      if (character === '\\') {
        index += Math.min(2, source.length - index);
        continue;
      }
      if (character === '[') inClass = true;
      if (character === ']') inClass = false;
      index += 1;
      if (character === '/' && !inClass) break;
      if (character === '\n') line += 1;
    }
    while (index < source.length && /[a-z]/i.test(source[index])) index += 1;
    tokens.push({ type: 'regex', value: '<regex>', start, end: index, line: tokenLine });
  }

  scanCode();
  return { tokens, comments };
}

function shouldStartRegex(previous) {
  return previous === undefined || [
    '(', '[', '{', '=', ':', ',', ';', '!', '?', '=>',
    'return', 'case', 'throw', 'else', 'do', 'typeof', 'void', 'delete',
  ].includes(previous.value);
}

function isIdentifierStart(character) {
  return character !== undefined && /[A-Za-z_$]/.test(character);
}

function isIdentifierPart(character) {
  return character !== undefined && /[A-Za-z0-9_$]/.test(character);
}
