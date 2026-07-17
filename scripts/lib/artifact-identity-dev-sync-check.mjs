const devSyncPath = 'scripts/skiff-dev-sync.mjs';

export function collectDevSyncArtifactPathFailures(text) {
  const failures = [];
  const forbidden = [
    {
      regexp: /\bartifactPathKeys\b/,
      message: 'must not maintain a guessed artifact path-key registry',
    },
    {
      regexp: /\b(?:artifactReferencePaths|collectArtifactReferencePaths)\b/,
      message: 'must not recursively guess artifact references from index fields',
    },
    {
      regexp: /\bjoin\(\s*root\s*,\s*(?:serviceUnit|packageUnit)\.unitPath\s*\)/,
      message: 'must not join target pointer unitPath before the validated-reference boundary',
    },
  ];
  for (const restriction of forbidden) {
    const match = restriction.regexp.exec(text);
    if (match !== null) {
      failures.push(
        `${devSyncPath}:${lineNumberAt(text, match.index)} ${restriction.message}`,
      );
    }
  }
  if (!/\bassertArtifactReferencesMatchValidated\s*\(/.test(text)) {
    failures.push(
      `${devSyncPath} must exact-match target references to the CLI-validated closure before artifact access`,
    );
  }
  return failures;
}

export function devSyncArtifactPathSelfTestFailures() {
  const cases = [
    {
      name: 'allows filesystem access through trusted references',
      text: `const trustedReferences = assertArtifactReferencesMatchValidated(actual, validated, label);
join(root, trustedReferences.serviceUnit.unitPath);
`,
      expectedFailures: 0,
    },
    {
      name: 'rejects recursive artifact path guessing',
      text: `const artifactPathKeys = new Set(['path']);
function collectArtifactReferencePaths(value) { return value; }
`,
      expectedFailures: 3,
    },
    {
      name: 'rejects raw target unitPath joins',
      text: `assertArtifactReferencesMatchValidated(actual, validated, label);
join(root, serviceUnit.unitPath);
`,
      expectedFailures: 1,
    },
  ];
  const failures = [];
  for (const testCase of cases) {
    const actual = collectDevSyncArtifactPathFailures(testCase.text);
    if (actual.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} dev-sync path failure(s), got ${actual.length}`,
      );
    }
  }
  return failures;
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
