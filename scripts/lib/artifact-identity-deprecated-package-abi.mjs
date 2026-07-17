const deprecatedPackageAbiRustSymbols = [
  'package_abi_hash',
  'package_abi_identity',
  'PACKAGE_ABI_IDENTITY_PREFIX',
];

const deprecatedSymbolPatterns = deprecatedPackageAbiRustSymbols.map((symbol) => ({
  regexp: new RegExp(`\\b${symbol}\\b`, 'g'),
  symbol,
}));

export function collectDeprecatedPackageAbiRustSymbolFailures(files) {
  const failures = [];
  for (const file of files) {
    if (!file.relPath.endsWith('.rs')) {
      continue;
    }
    for (const { regexp, symbol } of deprecatedSymbolPatterns) {
      regexp.lastIndex = 0;
      for (const match of file.text.matchAll(regexp)) {
        failures.push(
          `${file.relPath}:${lineNumberAt(file.text, match.index ?? 0)} deprecated package ABI Rust symbol ${symbol} is forbidden`,
        );
      }
    }
  }
  return failures;
}

export function deprecatedPackageAbiRustSymbolSelfTestFailures() {
  const cases = [
    {
      name: 'allows canonical local ABI symbols and near matches',
      files: [{
        relPath: 'artifact-identity/src/package.rs',
        text: `fn package_local_abi_hash() {}
fn package_local_abi_identity() {}
const PACKAGE_LOCAL_ABI_IDENTITY_PREFIX: &str = "local";
fn package_abi_identity_v2() {}
`,
      }],
      expectedFailures: 0,
    },
    {
      name: 'rejects the deprecated hash definition',
      files: [{
        relPath: 'artifact-identity/src/package.rs',
        text: 'pub fn package_abi_hash() {}\n',
      }],
      expectedFailures: 1,
    },
    {
      name: 'rejects the deprecated identity in cfg-test code',
      files: [{
        relPath: 'runtime/host/src/tests.rs',
        text: '#[cfg(test)]\nfn identity() { package_abi_identity(&unit); }\n',
      }],
      expectedFailures: 1,
    },
    {
      name: 'rejects the deprecated prefix import',
      files: [{
        relPath: 'compiler/projection/src/lib.rs',
        text: 'use skiff_artifact_identity::PACKAGE_ABI_IDENTITY_PREFIX;\n',
      }],
      expectedFailures: 1,
    },
  ];

  const failures = [];
  for (const testCase of cases) {
    const actual = collectDeprecatedPackageAbiRustSymbolFailures(testCase.files);
    if (actual.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} failure(s), got ${actual.length}`,
      );
    }
  }
  return failures;
}

function lineNumberAt(text, index) {
  let line = 1;
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (text.charCodeAt(cursor) === 10) {
      line += 1;
    }
  }
  return line;
}
