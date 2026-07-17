const devSyncPath = 'scripts/skiff-dev-sync.mjs';
const pathOwner = 'scripts/lib/artifact-identity-dev-sync-paths.mjs';
const ownerFunction = 'assertValidatedArtifactClosureFiles';

const artifactReferenceSource = /(?:\.(?:assemblyPath|unitPath)\b|\[\s*['"](?:assemblyPath|unitPath)['"]\s*\]|(?:\{|,)\s*(?:assemblyPath|unitPath)\s*(?::\s*[A-Za-z_$][\w$]*)?\s*(?=[,}]))/;
const filesystemSink = /\b(?:access|isFile|join|lstat|mkdir|open|readFile|readdir|realpath|rename|resolve|rm|stat|writeFile)\b/;

export function collectDevSyncArtifactPathFailures(mainText, ownerText, productionFiles = []) {
  const failures = collectFunctionSourceSinkFailures(mainText, devSyncPath);
  for (const file of productionFiles) {
    if (file.relPath === devSyncPath || file.relPath === pathOwner) {
      continue;
    }
    failures.push(...collectFunctionSourceSinkFailures(file.text, file.relPath));
  }
  const pointerContract = functionDeclarations(mainText)
    .find((declaration) => declaration.name === 'assertDevReloadPointerContract');
  if (pointerContract === undefined) {
    failures.push(`${devSyncPath} is missing assertDevReloadPointerContract`);
  } else {
    if (!new RegExp(`\\b${ownerFunction}\\s*\\(`).test(pointerContract.code)) {
      failures.push(`${devSyncPath} assertDevReloadPointerContract must delegate artifact access to ${pathOwner}`);
    }
    if (filesystemSink.test(pointerContract.code)) {
      failures.push(`${devSyncPath} assertDevReloadPointerContract must not access the filesystem directly`);
    }
  }

  if (!new RegExp(`export\\s+async\\s+function\\s+${ownerFunction}\\b`).test(ownerText)) {
    failures.push(`${pathOwner} must export ${ownerFunction}`);
  }
  if (!/\bassertArtifactReferencesMatchValidated\s*\(/.test(ownerText)) {
    failures.push(`${pathOwner} must exact-match references before filesystem access`);
  }
  if (!artifactReferenceSource.test(maskNonCode(ownerText)) || !filesystemSink.test(maskNonCode(ownerText))) {
    failures.push(`${pathOwner} must own validated artifact reference filesystem access`);
  }
  return failures;
}

export function collectFunctionSourceSinkFailures(text, relPath) {
  const failures = [];
  for (const declaration of functionDeclarations(text)) {
    if (artifactReferenceSource.test(declaration.code) && filesystemSink.test(declaration.code)) {
      failures.push(
        `${relPath}:${declaration.line} ${declaration.name} combines raw artifact references with filesystem access; delegate to ${pathOwner}`,
      );
    }
  }
  return failures;
}

function functionDeclarations(text) {
  const code = maskNonCode(text);
  const pattern = /\b(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/g;
  const declarations = [];
  for (const match of code.matchAll(pattern)) {
    const openParen = (match.index ?? 0) + match[0].lastIndexOf('(');
    const closeParen = matchingDelimiterIndex(code, openParen, '(', ')');
    if (closeParen === -1) {
      continue;
    }
    let openBrace = closeParen + 1;
    while (/\s/.test(code[openBrace] ?? '')) {
      openBrace += 1;
    }
    if (code[openBrace] !== '{') {
      continue;
    }
    const closeBrace = matchingBraceIndex(code, openBrace);
    if (closeBrace === -1) {
      continue;
    }
    declarations.push({
      name: match[1],
      code: code.slice(match.index ?? 0, closeBrace + 1),
      line: lineNumberAt(code, match.index ?? 0),
    });
  }
  return declarations;
}

function matchingDelimiterIndex(code, open, openChar, closeChar) {
  let depth = 0;
  for (let index = open; index < code.length; index += 1) {
    if (code[index] === openChar) {
      depth += 1;
    } else if (code[index] === closeChar) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function matchingBraceIndex(code, openBrace) {
  let depth = 0;
  for (let index = openBrace; index < code.length; index += 1) {
    if (code[index] === '{') {
      depth += 1;
    } else if (code[index] === '}') {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function maskNonCode(text) {
  const output = [...text];
  let state = 'code';
  for (let index = 0; index < output.length; index += 1) {
    const char = text[index];
    const next = text[index + 1];
    if (state === 'code') {
      if (char === "'" || char === '"' || char === '`') {
        state = char;
        output[index] = ' ';
      } else if (char === '/' && next === '/') {
        state = 'line-comment';
        output[index] = ' ';
        output[index + 1] = ' ';
        index += 1;
      } else if (char === '/' && next === '*') {
        state = 'block-comment';
        output[index] = ' ';
        output[index + 1] = ' ';
        index += 1;
      }
      continue;
    }
    if (state === 'line-comment') {
      if (char === '\n') {
        state = 'code';
      } else {
        output[index] = ' ';
      }
      continue;
    }
    output[index] = char === '\n' ? '\n' : ' ';
    if (state === 'block-comment') {
      if (char === '*' && next === '/') {
        output[index + 1] = ' ';
        index += 1;
        state = 'code';
      }
    } else if (char === '\\') {
      if (index + 1 < output.length) {
        output[index + 1] = text[index + 1] === '\n' ? '\n' : ' ';
        index += 1;
      }
    } else if (char === state) {
      state = 'code';
    }
  }
  return output.join('');
}

function lineNumberAt(text, index) {
  let line = 1;
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (text[cursor] === '\n') {
      line += 1;
    }
  }
  return line;
}
