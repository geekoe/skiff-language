#!/usr/bin/env node

import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { captureAttachedCommand } from './lib/command-execution.mjs';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const runtimeDag = new Map([
  [
    'skiff-runtime-host',
    [
      'skiff-runtime-transport',
      'skiff-runtime-config-snapshot',
      'skiff-runtime-request',
      'skiff-runtime-package-test',
      'skiff-runtime-loader',
      'skiff-runtime-linker',
      'skiff-runtime-bytecode-verifier',
      'skiff-runtime-deployment-image',
      'skiff-runtime-activation',
      'skiff-runtime-capability-context',
      'skiff-runtime-native',
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
      'skiff-runtime-service-db',
    ],
  ],
  [
    'skiff-runtime-service-db',
    [
      'skiff-runtime-capability-context',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-bytecode-verifier',
    [
      'skiff-runtime-linked-bytecode',
      'skiff-runtime-loader',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-vm',
    [
      'skiff-runtime-bytecode-verifier',
      'skiff-runtime-deployment-image',
      'skiff-runtime-linker',
      'skiff-runtime-linked-bytecode',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-scheduler',
    [
      'skiff-runtime-model',
      'skiff-runtime-vm',
    ],
  ],
  ['skiff-runtime-config-snapshot', []],
  ['skiff-runtime-deployment-image', []],
  ['skiff-runtime-linked-bytecode', []],
  ['skiff-runtime-transport', ['skiff-runtime-request-contract']],
  [
    'skiff-runtime-package-test',
    [
      'skiff-runtime-loader',
      'skiff-runtime-linker',
      'skiff-runtime-bytecode-verifier',
    ],
  ],
  [
    'skiff-runtime-request',
    [
      'skiff-runtime-boundary',
      'skiff-runtime-capability-context',
      'skiff-runtime-linked-bytecode',
      'skiff-runtime-linker',
      'skiff-runtime-model',
      'skiff-runtime-request-contract',
      'skiff-runtime-scheduler',
      'skiff-runtime-vm',
    ],
  ],
  [
    'skiff-runtime-native',
    [
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-capability-context',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-capability-context',
    [
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
      'skiff-runtime-request-contract',
    ],
  ],
  ['skiff-runtime-activation', ['skiff-runtime-model']],
  [
    'skiff-runtime-linked-type-plan',
    [
      'skiff-runtime-boundary',
      'skiff-runtime-model',
      'skiff-runtime-native-contract',
    ],
  ],
  [
    'skiff-runtime-linker',
    [
      'skiff-runtime-bytecode-verifier',
      'skiff-runtime-deployment-image',
      'skiff-runtime-linked-bytecode',
      'skiff-runtime-loader',
      'skiff-runtime-native-contract',
    ],
  ],
  ['skiff-runtime-request-contract', []],
  ['skiff-runtime-native-contract', ['skiff-runtime-model']],
  ['skiff-runtime-loader', ['skiff-runtime-model']],
  ['skiff-runtime-boundary', ['skiff-runtime-model']],
  ['skiff-runtime-model', ['skiff-runtime-request-contract']],
]);

const expectedPromotedRuntimePackages = new Set([
  'skiff-runtime-activation',
  'skiff-runtime-boundary',
  'skiff-runtime-bytecode-verifier',
  'skiff-runtime-capability-context',
  'skiff-runtime-config-snapshot',
  'skiff-runtime-deployment-image',
  'skiff-runtime-host',
  'skiff-runtime-linked-bytecode',
  'skiff-runtime-linked-type-plan',
  'skiff-runtime-linker',
  'skiff-runtime-loader',
  'skiff-runtime-model',
  'skiff-runtime-native',
  'skiff-runtime-native-contract',
  'skiff-runtime-package-test',
  'skiff-runtime-request',
  'skiff-runtime-request-contract',
  'skiff-runtime-scheduler',
  'skiff-runtime-service-db',
  'skiff-runtime-transport',
  'skiff-runtime-vm',
]);

const hostBoundaryTarget = {
  hostPackageName: 'skiff-runtime-host',
  docs: [
    'doc/architecture/bytecode-vm.md',
  ],
  allowedRuntimeDeps: [
    'skiff-runtime-transport',
    'skiff-runtime-config-snapshot',
    'skiff-runtime-request',
    'skiff-runtime-package-test',
    'skiff-runtime-loader',
    'skiff-runtime-linker',
    'skiff-runtime-bytecode-verifier',
    'skiff-runtime-deployment-image',
    'skiff-runtime-activation',
    'skiff-runtime-capability-context',
    'skiff-runtime-model',
    'skiff-runtime-service-db',
  ],
  temporaryDebtRationales: new Map([
    [
      'skiff-runtime-boundary',
      'host still calls other boundary utilities/conversions after request_mapper, control_mapper, and control_response_mapper moved router-session request/control/control-response frame mappings to transport',
    ],
    [
      'skiff-runtime-native',
      'host still reaches native dispatch wiring that should be hidden behind eval/request composition',
    ],
    [
      'skiff-runtime-native-contract',
      'host still consumes native contract metadata during current request and test-service assembly',
    ],
  ]),
};

const expectedHostBoundaryTargetDebts = [
  'skiff-runtime-boundary',
  'skiff-runtime-native',
  'skiff-runtime-native-contract',
];

const executableFactsBoundary = {
  owner: 'runtime/bytecode-verifier/src/verifier.rs',
  ownerRoot: 'runtime/bytecode-verifier/',
  consumer: 'runtime/linker/src/bytecode/execution_image.rs',
  symbols: ['ExecutableFacts', 'verify_executable_facts'],
};

try {
  const cliOptions = parseArgs(process.argv.slice(2));

  validateEncodedDag(runtimeDag);
  validateHostBoundaryTarget();

  if (cliOptions.help) {
    printUsage();
  } else if (cliOptions.selfTest) {
    runSelfTests();
  } else {
    const metadata = await cargoMetadata();
    const dagResult = checkRuntimeDag(metadata);
    printRuntimeDagResult(dagResult);
    const boundaryResult = checkExecutableFactsBoundary(loadRuntimeRustSources());
    printExecutableFactsBoundaryResult(boundaryResult);

    let exitCode = dagResult.violations.length > 0 || boundaryResult.violations.length > 0 ? 1 : 0;

    if (cliOptions.hostBoundary !== null) {
      const hostBoundaryResult = checkHostBoundaryTarget(metadata);
      printHostBoundaryResult(hostBoundaryResult, cliOptions.hostBoundary);
      exitCode = Math.max(exitCode, hostBoundaryExitCode(hostBoundaryResult, cliOptions.hostBoundary));
    }

    if (exitCode !== 0) {
      process.exitCode = exitCode;
    }
  }
} catch (error) {
  console.error(`ERROR ${error.message}`);
  process.exitCode = 1;
}

function checkRuntimeDag(metadata) {
  const workspacePackages = workspaceMemberPackages(metadata);
  const workspacePackageNames = new Set(workspacePackages.map((pkg) => pkg.name));
  const promotedRuntimePackages = workspacePackages
    .filter((pkg) => isRuntimePackageName(pkg.name))
    .sort((left, right) => left.name.localeCompare(right.name));
  const violations = [];

  for (const packageName of expectedPromotedRuntimePackages) {
    if (!workspacePackageNames.has(packageName)) {
      violations.push({
        packageName,
        manifestPath: '(workspace)',
        message:
          'expected promoted runtime crate is not a workspace member; add its manifest to Cargo.toml members before relying on DAG checks',
      });
    }
  }

  for (const pkg of promotedRuntimePackages) {
    const allowedRuntimeDeps = runtimeDag.get(pkg.name);
    if (!allowedRuntimeDeps) {
      violations.push({
        packageName: pkg.name,
        manifestPath: pkg.manifest_path,
        message:
          'no runtime DAG rule is encoded for this promoted crate; add the architecture rule before adding the crate to the workspace',
      });
      continue;
    }

    const allowed = new Set(allowedRuntimeDeps);
    for (const dependency of pkg.dependencies ?? []) {
      if (!isRuntimePackageName(dependency.name)) {
        continue;
      }

      const kind = dependencyKind(dependency);
      if (!isProductionDependency(dependency)) {
        continue;
      }

      if (!workspacePackageNames.has(dependency.name)) {
        violations.push({
          packageName: pkg.name,
          manifestPath: pkg.manifest_path,
          message: `${kind} dependency ${dependency.name} is a skiff-runtime-* crate but is not a workspace member`,
        });
        continue;
      }

      if (!allowed.has(dependency.name)) {
        violations.push({
          packageName: pkg.name,
          manifestPath: pkg.manifest_path,
          message: `${kind} dependency ${dependency.name} is not allowed by the runtime crate DAG; allowed skiff-runtime-* dependencies: ${formatAllowed(allowedRuntimeDeps)}`,
        });
      }
    }
  }

  return { promotedRuntimePackages, violations };
}

function checkExecutableFactsBoundary(sources) {
  const violations = [];
  const ownerTokens = sourceTokens(sources, executableFactsBoundary.owner, violations);
  const consumerTokens = sourceTokens(sources, executableFactsBoundary.consumer, violations);
  if (ownerTokens === null || consumerTokens === null) {
    return { violations };
  }

  expectSequenceCount(
    ownerTokens,
    ['pub', 'struct', 'ExecutableFacts'],
    1,
    `${executableFactsBoundary.owner} public ExecutableFacts declaration`,
    violations,
  );
  expectSequenceCount(
    ownerTokens,
    ['pub', 'fn', 'verify_executable_facts'],
    1,
    `${executableFactsBoundary.owner} public verify_executable_facts declaration`,
    violations,
  );
  expectExecutableFactsIntoParts(ownerTokens, violations);

  for (const symbol of executableFactsBoundary.symbols) {
    const consumers = [...sources]
      .filter(([path, source]) =>
        !path.startsWith(executableFactsBoundary.ownerRoot)
        && tokenizeRust(source, path).includes(symbol))
      .map(([path]) => path)
      .sort();
    if (
      consumers.length !== 1
      || consumers[0] !== executableFactsBoundary.consumer
    ) {
      violations.push(
        `${symbol} external source owner must be exactly ${executableFactsBoundary.consumer}; found ${consumers.length === 0 ? '(none)' : consumers.join(', ')}`,
      );
    }
  }

  expectConstructionFunctionOwnership(consumerTokens, violations);
  return { violations };
}

function sourceTokens(sources, path, violations) {
  const source = sources.get(path);
  if (source === undefined) {
    violations.push(`required executable-facts boundary source is missing: ${path}`);
    return null;
  }
  return tokenizeRust(source, path);
}

function expectExecutableFactsIntoParts(tokens, violations) {
  const implStart = sequenceIndexes(tokens, ['impl', 'ExecutableFacts', '{']);
  if (implStart.length !== 1) {
    violations.push(
      `${executableFactsBoundary.owner} must contain exactly one direct impl ExecutableFacts block; found ${implStart.length}`,
    );
    return;
  }
  const openBrace = implStart[0] + 2;
  const closeBrace = matchingTokenBrace(tokens, openBrace);
  if (closeBrace === -1) {
    violations.push(`${executableFactsBoundary.owner} has an unterminated impl ExecutableFacts block`);
    return;
  }
  expectSequenceCount(
    tokens.slice(openBrace + 1, closeBrace),
    ['pub', 'fn', 'into_parts', '('],
    1,
    `${executableFactsBoundary.owner} public ExecutableFacts::into_parts declaration`,
    violations,
  );
}

function expectConstructionFunctionOwnership(tokens, violations) {
  const functionName = 'link_deployment_execution_image';
  const functionBody = rustFunctionBody(tokens, functionName, violations);
  if (functionBody === null) return;

  const body = tokens.slice(functionBody.openBrace + 1, functionBody.closeBrace);
  const outside = [
    ...tokens.slice(0, functionBody.openBrace + 1),
    ...tokens.slice(functionBody.closeBrace),
  ];
  for (const symbol of executableFactsBoundary.symbols) {
    expectSequenceCount(
      body,
      [symbol],
      1,
      `${executableFactsBoundary.consumer} ${functionName} body ${symbol} ownership`,
      violations,
    );
    expectSequenceCount(
      outside,
      [symbol],
      1,
      `${executableFactsBoundary.consumer} ${symbol} import-only use outside ${functionName}`,
      violations,
    );
  }
  expectSequenceCount(
    body,
    ['verify_executable_facts', '('],
    1,
    `${executableFactsBoundary.consumer} ${functionName} verifier call`,
    violations,
  );
  expectSequenceCount(
    body,
    ['facts', '.', 'into_parts', '('],
    1,
    `${executableFactsBoundary.consumer} ${functionName} ExecutableFacts::into_parts call`,
    violations,
  );
  expectSequenceCount(
    tokens,
    ['facts', '.', 'into_parts', '('],
    1,
    `${executableFactsBoundary.consumer} ExecutableFacts::into_parts sole consumer`,
    violations,
  );
}

function rustFunctionBody(tokens, name, violations) {
  const declarations = sequenceIndexes(tokens, ['pub', 'fn', name]);
  if (declarations.length !== 1) {
    violations.push(
      `${executableFactsBoundary.consumer} must define exactly one public ${name}; found ${declarations.length}`,
    );
    return null;
  }
  const declaration = declarations[0];
  let openBrace = declaration + 3;
  while (openBrace < tokens.length && tokens[openBrace] !== '{') {
    if (tokens[openBrace] === ';') {
      violations.push(`${executableFactsBoundary.consumer} ${name} has no function body`);
      return null;
    }
    openBrace += 1;
  }
  if (openBrace === tokens.length) {
    violations.push(`${executableFactsBoundary.consumer} ${name} has no function body`);
    return null;
  }
  const closeBrace = matchingTokenBrace(tokens, openBrace);
  if (closeBrace === -1) {
    violations.push(`${executableFactsBoundary.consumer} ${name} has an unterminated function body`);
    return null;
  }
  return { openBrace, closeBrace };
}

function expectSequenceCount(tokens, sequence, expected, label, violations) {
  const actual = sequenceIndexes(tokens, sequence).length;
  if (actual !== expected) {
    violations.push(`${label} must occur exactly ${expected} time(s); found ${actual}`);
  }
}

function sequenceIndexes(tokens, sequence) {
  const indexes = [];
  for (let index = 0; index <= tokens.length - sequence.length; index += 1) {
    if (sequence.every((token, offset) => tokens[index + offset] === token)) {
      indexes.push(index);
    }
  }
  return indexes;
}

function matchingTokenBrace(tokens, openBrace) {
  let depth = 0;
  for (let index = openBrace; index < tokens.length; index += 1) {
    if (tokens[index] === '{') depth += 1;
    if (tokens[index] === '}') depth -= 1;
    if (depth === 0) return index;
  }
  return -1;
}

function loadRuntimeRustSources() {
  const sources = new Map();
  const ignoredDirectories = new Set([
    '.git', '.skiff-dev', '.stack', 'build', 'node_modules', 'target',
  ]);
  visit(root);
  return sources;

  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink() || (entry.isDirectory() && ignoredDirectories.has(entry.name))) {
        continue;
      }
      const absolute = join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile() && entry.name.endsWith('.rs')) {
        sources.set(toRepoRelative(absolute), readFileSync(absolute, 'utf8'));
      }
    }
  }
}

function tokenizeRust(source, label = '(fixture)') {
  const tokens = [];
  let index = 0;
  while (index < source.length) {
    if (/\s/.test(source[index])) {
      index += 1;
      continue;
    }
    if (source.startsWith('//', index)) {
      index = nextLineIndex(source, index + 2);
      continue;
    }
    if (source.startsWith('/*', index)) {
      index = rustBlockCommentEnd(source, index, label);
      continue;
    }
    const rawEnd = rustRawStringEnd(source, index, label);
    if (rawEnd !== null) {
      index = rawEnd;
      continue;
    }
    const quoteIndex = source[index] === '"'
      ? index
      : ['b', 'c'].includes(source[index]) && source[index + 1] === '"'
        ? index + 1
        : -1;
    if (quoteIndex !== -1) {
      index = rustQuotedEnd(source, quoteIndex, '"', label);
      continue;
    }
    const charIndex = source[index] === '\''
      ? index
      : source[index] === 'b' && source[index + 1] === '\''
        ? index + 1
        : -1;
    const charEnd = charIndex === -1 ? null : rustCharacterEnd(source, charIndex);
    if (charEnd !== null) {
      index = charEnd;
      continue;
    }
    if (/[A-Za-z_]/.test(source[index])) {
      const start = index;
      index += 1;
      while (index < source.length && /[A-Za-z0-9_]/.test(source[index])) index += 1;
      tokens.push(source.slice(start, index));
      continue;
    }
    tokens.push(source[index]);
    index += 1;
  }
  return tokens;
}

function nextLineIndex(source, index) {
  const newline = source.indexOf('\n', index);
  return newline === -1 ? source.length : newline + 1;
}

function rustBlockCommentEnd(source, start, label) {
  let depth = 1;
  let index = start + 2;
  while (index < source.length && depth > 0) {
    if (source.startsWith('/*', index)) {
      depth += 1;
      index += 2;
    } else if (source.startsWith('*/', index)) {
      depth -= 1;
      index += 2;
    } else {
      index += 1;
    }
  }
  if (depth !== 0) throw new Error(`${label}: unterminated Rust block comment`);
  return index;
}

function rustRawStringEnd(source, start, label) {
  let index = start;
  if (['b', 'c'].includes(source[index])) index += 1;
  if (source[index] !== 'r') return null;
  index += 1;
  let hashes = 0;
  while (source[index] === '#') {
    hashes += 1;
    index += 1;
  }
  if (source[index] !== '"') return null;
  const terminator = `"${'#'.repeat(hashes)}`;
  const end = source.indexOf(terminator, index + 1);
  if (end === -1) throw new Error(`${label}: unterminated Rust raw string`);
  return end + terminator.length;
}

function rustQuotedEnd(source, quoteIndex, quote, label) {
  let index = quoteIndex + 1;
  while (index < source.length) {
    if (source[index] === '\\') {
      index += 2;
    } else if (source[index] === quote) {
      return index + 1;
    } else {
      index += 1;
    }
  }
  throw new Error(`${label}: unterminated Rust quoted literal`);
}

function rustCharacterEnd(source, quoteIndex) {
  let index = quoteIndex + 1;
  if (source[index] === '\\') {
    index += 2;
  } else {
    const codePoint = source.codePointAt(index);
    if (codePoint === undefined) return null;
    index += codePoint > 0xffff ? 2 : 1;
  }
  return source[index] === '\'' ? index + 1 : null;
}

function checkHostBoundaryTarget(metadata) {
  const failures = [];
  const workspacePackages = workspaceMemberPackages(metadata);
  const workspacePackageNames = new Set(workspacePackages.map((pkg) => pkg.name));
  const workspacePackageByName = new Map(workspacePackages.map((pkg) => [pkg.name, pkg]));
  const hostPackage = workspacePackageByName.get(hostBoundaryTarget.hostPackageName);
  const allowedTargetDeps = new Set(hostBoundaryTarget.allowedRuntimeDeps);
  const allowed = [];
  const debts = [];
  const unregisteredDebts = [];
  const ignoredNonProductionDeps = [];

  if (hostPackage === undefined) {
    failures.push(`${hostBoundaryTarget.hostPackageName} is not a workspace member`);
    return { failures, allowed, debts, unregisteredDebts, ignoredNonProductionDeps, retiredExpectedDebts: [] };
  }

  for (const dependency of hostPackage.dependencies ?? []) {
    if (!isRuntimePackageName(dependency.name)) {
      continue;
    }

    const edge = {
      packageName: hostPackage.name,
      dependencyName: dependency.name,
      kind: dependencyKind(dependency),
      manifestPath: hostPackage.manifest_path,
    };

    if (!workspacePackageNames.has(dependency.name)) {
      failures.push(
        `${hostPackage.name} ${edge.kind} dependency ${dependency.name} is a skiff-runtime-* crate but is not a workspace member`,
      );
      continue;
    }

    if (!isProductionDependency(dependency)) {
      ignoredNonProductionDeps.push(edge);
      continue;
    }

    if (allowedTargetDeps.has(dependency.name)) {
      allowed.push(edge);
      continue;
    }

    const rationale = hostBoundaryTarget.temporaryDebtRationales.get(dependency.name);
    if (rationale === undefined) {
      unregisteredDebts.push({
        ...edge,
        message:
          'target host boundary does not allow this direct production runtime dependency and no Stage 1 temporary debt rationale is registered',
      });
      continue;
    }

    debts.push({
      ...edge,
      rationale,
    });
  }

  const presentProductionDeps = new Set([
    ...allowed.map((edge) => edge.dependencyName),
    ...debts.map((edge) => edge.dependencyName),
    ...unregisteredDebts.map((edge) => edge.dependencyName),
  ]);
  const retiredExpectedDebts = expectedHostBoundaryTargetDebts.filter(
    (dependencyName) => !presentProductionDeps.has(dependencyName),
  );

  debts.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  unregisteredDebts.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  allowed.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  ignoredNonProductionDeps.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));

  return { failures, allowed, debts, unregisteredDebts, ignoredNonProductionDeps, retiredExpectedDebts };
}

function printRuntimeDagResult(result) {
  if (result.violations.length > 0) {
    console.error('\nRuntime crate DAG check failed.\n');
    console.error(
      'Only promoted skiff-runtime-* workspace crates are checked; non-runtime workspace crates are not constrained by this script.\n',
    );
    for (const violation of result.violations) {
      console.error(
        `- ${violation.packageName} (${toRepoRelative(violation.manifestPath)}): ${violation.message}`,
      );
    }
    return;
  }

  console.log(
    `Runtime crate DAG check passed for ${result.promotedRuntimePackages.length} promoted crate${result.promotedRuntimePackages.length === 1 ? '' : 's'}: ${formatCheckedPackages(result.promotedRuntimePackages)}.`,
  );
}

function printExecutableFactsBoundaryResult(result) {
  if (result.violations.length > 0) {
    console.error('\nExecutable facts source-boundary check failed.\n');
    for (const violation of result.violations) console.error(`- ${violation}`);
    return;
  }
  console.log(
    `Executable facts source-boundary check passed: ${executableFactsBoundary.consumer} is the sole external construction-facts consumer.`,
  );
}

function printHostBoundaryResult(result, mode) {
  console.log(`\nRuntime host boundary target debt report (${mode} mode).`);
  console.log(`Docs: ${hostBoundaryTarget.docs.join(', ')}.`);
  console.log(`Target direct runtime deps: ${formatAllowed(hostBoundaryTarget.allowedRuntimeDeps)}.`);
  console.log('Only normal production dependencies are evaluated for target host debt.');

  if (result.failures.length > 0) {
    console.error('\nRuntime host boundary target structural failure(s):');
    for (const failure of result.failures) {
      console.error(`- ${failure}`);
    }
  }

  if (result.debts.length === 0 && result.unregisteredDebts.length === 0) {
    console.log('\nNo runtime host target dependency debt remains.');
  } else if (result.debts.length > 0) {
    console.log(
      `\nRuntime host target debt remains (${result.debts.length} direct production dependenc${result.debts.length === 1 ? 'y' : 'ies'}):`,
    );
    for (const debt of result.debts) {
      console.log(
        `- ${debt.packageName} -> ${debt.dependencyName} (${debt.kind}): ${debt.rationale}; temporarily allowed by the current DAG while Stage 1 tracks the migration.`,
      );
    }
  }

  if (result.unregisteredDebts.length > 0) {
    console.error(
      `\nUnregistered runtime host target debt (${result.unregisteredDebts.length} direct production dependenc${result.unregisteredDebts.length === 1 ? 'y' : 'ies'}):`,
    );
    for (const debt of result.unregisteredDebts) {
      console.error(`- ${debt.packageName} -> ${debt.dependencyName} (${debt.kind}): ${debt.message}.`);
    }
  }

  if (result.retiredExpectedDebts.length > 0) {
    console.log(`\nRetired Stage 1 expected debt not present in Cargo metadata: ${result.retiredExpectedDebts.join(', ')}.`);
  }

  if (result.ignoredNonProductionDeps.length > 0) {
    const ignored = result.ignoredNonProductionDeps.map(
      (edge) => `${edge.dependencyName} (${edge.kind})`,
    );
    console.log(`\nIgnored non-production host runtime deps: ${ignored.join(', ')}.`);
  }

  if (result.failures.length > 0 || result.unregisteredDebts.length > 0) {
    console.error(
      '\nRuntime host boundary target check failed because unregistered or structurally invalid debt must fail closed in every mode.',
    );
  } else if (mode === 'deny' && result.debts.length > 0) {
    console.error(
      '\nRuntime host boundary deny failed because target dependency debt remains. This is the expected Stage 1 failure until the listed host edges are removed or the target is intentionally corrected.',
    );
  } else if (mode === 'report') {
    console.log('\nReport mode is informational and exits 0 when only registered target debt remains.');
  }
}

function hostBoundaryExitCode(result, mode) {
  if (result.failures.length > 0 || result.unregisteredDebts.length > 0) {
    return 1;
  }
  if (mode === 'deny' && result.debts.length > 0) {
    return 1;
  }
  return 0;
}

async function cargoMetadata() {
  const { status, stdout, stderr } = await run('cargo', ['metadata', '--format-version', '1', '--no-deps']);
  if (status !== 0) {
    throw new Error(`cargo metadata failed with exit code ${status}\n${stderr}`.trim());
  }

  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`cargo metadata did not return valid JSON: ${error.message}`);
  }
}

async function run(command, args) {
  const result = await captureAttachedCommand(command, args, { cwd: root });
  if (result.error !== null) {
    throw new Error(result.error.message);
  }
  return { status: result.code, stdout: result.stdout, stderr: result.stderr };
}

function workspaceMemberPackages(metadata) {
  if (!Array.isArray(metadata.packages) || !Array.isArray(metadata.workspace_members)) {
    throw new Error('cargo metadata is missing packages or workspace_members');
  }

  const workspaceMemberIds = new Set(metadata.workspace_members);
  return metadata.packages.filter((pkg) => workspaceMemberIds.has(pkg.id));
}

function isRuntimePackageName(packageName) {
  return packageName.startsWith('skiff-runtime-');
}

function dependencyKind(dependency) {
  return dependency.kind ?? 'normal';
}

function isProductionDependency(dependency) {
  return dependency.kind === null || dependency.kind === undefined || dependency.kind === 'normal';
}

function formatAllowed(allowed) {
  return allowed.length === 0 ? '(none)' : allowed.join(', ');
}

function formatCheckedPackages(packages) {
  if (packages.length === 0) {
    return '(none)';
  }
  return packages.map((pkg) => pkg.name).join(', ');
}

function toRepoRelative(path) {
  return relative(root, path).split('\\').join('/');
}

function validateEncodedDag(dag) {
  for (const [packageName, allowedDependencies] of dag) {
    for (const dependencyName of allowedDependencies) {
      if (!dag.has(dependencyName)) {
        throw new Error(`${packageName} allows unknown runtime dependency ${dependencyName}`);
      }
    }
  }

  const permanent = new Set();
  const temporary = new Set();
  const stack = [];

  for (const packageName of dag.keys()) {
    visit(packageName);
  }

  function visit(packageName) {
    if (permanent.has(packageName)) {
      return;
    }
    if (temporary.has(packageName)) {
      const cycleStart = stack.indexOf(packageName);
      const cycle = [...stack.slice(cycleStart), packageName].join(' -> ');
      throw new Error(`encoded runtime crate DAG contains a cycle: ${cycle}`);
    }

    temporary.add(packageName);
    stack.push(packageName);
    for (const dependencyName of dag.get(packageName) ?? []) {
      visit(dependencyName);
    }
    stack.pop();
    temporary.delete(packageName);
    permanent.add(packageName);
  }
}

function validateHostBoundaryTarget() {
  if (!runtimeDag.has(hostBoundaryTarget.hostPackageName)) {
    throw new Error(`host boundary target references unknown host package ${hostBoundaryTarget.hostPackageName}`);
  }

  const runtimePackages = new Set(runtimeDag.keys());
  const targetAllowedDeps = new Set(hostBoundaryTarget.allowedRuntimeDeps);

  for (const dependencyName of hostBoundaryTarget.allowedRuntimeDeps) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`host boundary target allows unknown runtime dependency ${dependencyName}`);
    }
  }

  for (const dependencyName of expectedHostBoundaryTargetDebts) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`expected host boundary debt references unknown runtime dependency ${dependencyName}`);
    }
    if (targetAllowedDeps.has(dependencyName)) {
      throw new Error(`expected host boundary debt ${dependencyName} is also target-allowed`);
    }
    if (!hostBoundaryTarget.temporaryDebtRationales.has(dependencyName)) {
      throw new Error(`expected host boundary debt ${dependencyName} is missing a temporary rationale`);
    }
  }

  for (const dependencyName of hostBoundaryTarget.temporaryDebtRationales.keys()) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`host boundary debt rationale references unknown runtime dependency ${dependencyName}`);
    }
    if (targetAllowedDeps.has(dependencyName)) {
      throw new Error(`host boundary debt rationale ${dependencyName} is also target-allowed`);
    }
  }
}

function parseArgs(args) {
  const options = {
    help: false,
    selfTest: false,
    hostBoundary: null,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--self-test' || arg === '--test') {
      options.selfTest = true;
      continue;
    }
    if (arg.startsWith('--host-boundary=')) {
      options.hostBoundary = parseHostBoundaryMode(arg.slice('--host-boundary='.length));
      continue;
    }
    if (arg === '--host-boundary') {
      const value = args[index + 1];
      if (value === undefined) {
        throw new Error('--host-boundary requires report or deny');
      }
      options.hostBoundary = parseHostBoundaryMode(value);
      index += 1;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  return options;
}

function parseHostBoundaryMode(value) {
  if (value === 'report' || value === 'deny') {
    return value;
  }
  throw new Error(`--host-boundary must be report or deny, got ${value}`);
}

function printUsage() {
  console.log(`Usage: node scripts/check-runtime-crate-dag.mjs [--self-test] [--host-boundary=report|deny]

Default mode checks the current promoted skiff-runtime-* crate DAG.
--self-test runs synthetic checks without invoking cargo.
--host-boundary=report prints target host-boundary dependency debt and exits 0 for registered known debt only.
--host-boundary=deny exits non-zero while target host-boundary dependency debt remains.`);
}

function runSelfTests() {
  const cases = [
    {
      name: 'encoded runtime DAG validates',
      run: () => {
        validateEncodedDag(runtimeDag);
      },
    },
    {
      name: 'K1 execution-image edges replace obsolete authority inversions',
      run: () => {
        for (const [owner, dependency] of [
          ['skiff-runtime-linker', 'skiff-runtime-bytecode-verifier'],
          ['skiff-runtime-linker', 'skiff-runtime-deployment-image'],
          ['skiff-runtime-vm', 'skiff-runtime-linker'],
          ['skiff-runtime-request', 'skiff-runtime-linker'],
          ['skiff-runtime-package-test', 'skiff-runtime-bytecode-verifier'],
        ]) {
          assert(runtimeDag.get(owner).includes(dependency), `expected ${owner} -> ${dependency}`);
        }
        for (const [owner, dependency] of [
          ['skiff-runtime-bytecode-verifier', 'skiff-runtime-deployment-image'],
          ['skiff-runtime-request', 'skiff-runtime-bytecode-verifier'],
          ['skiff-runtime-request', 'skiff-runtime-deployment-image'],
        ]) {
          assert(!runtimeDag.get(owner).includes(dependency), `obsolete edge remains: ${owner} -> ${dependency}`);
        }
      },
    },
    {
      name: 'live executable facts boundary has one exact consumer',
      run: () => {
        const result = checkExecutableFactsBoundary(loadRuntimeRustSources());
        assert(
          result.violations.length === 0,
          `expected live source boundary to pass: ${result.violations.join('; ')}`,
        );
      },
    },
    {
      name: 'executable facts boundary ignores comments and string receipts',
      run: () => {
        const result = checkExecutableFactsBoundary(executableFactsBoundaryFixture());
        assert(
          result.violations.length === 0,
          `lexically inert receipts must not affect ownership: ${result.violations.join('; ')}`,
        );
      },
    },
    {
      name: 'executable facts boundary rejects a second source consumer',
      run: () => {
        const sources = executableFactsBoundaryFixture();
        sources.set(
          'runtime/request/src/bypass.rs',
          'use verifier::verify_executable_facts; fn bypass() { verify_executable_facts(); }',
        );
        const result = checkExecutableFactsBoundary(sources);
        assert(
          result.violations.some((violation) =>
            violation.includes('verify_executable_facts external source owner')
            && violation.includes('runtime/request/src/bypass.rs')),
          'expected the second source consumer to be rejected',
        );
      },
    },
    {
      name: 'executable facts boundary rejects a second same-file function consumer',
      run: () => {
        const sources = executableFactsBoundaryFixture();
        sources.set(
          executableFactsBoundary.consumer,
          `${sources.get(executableFactsBoundary.consumer)}\n`
            + 'pub fn leak_executable_facts() -> ExecutableFacts { verify_executable_facts() }\n',
        );
        const result = checkExecutableFactsBoundary(sources);
        assert(
          result.violations.some((violation) =>
            violation.includes('import-only use outside link_deployment_execution_image')),
          'expected a second same-file ExecutableFacts consumer to be rejected',
        );
        assert(
          result.violations.some((violation) =>
            violation.includes('verify_executable_facts import-only use outside')),
          'expected a second same-file verifier call to be rejected',
        );
      },
    },
    {
      name: 'executable facts boundary rejects comment-only into_parts proof',
      run: () => {
        const sources = executableFactsBoundaryFixture();
        sources.set(
          executableFactsBoundary.consumer,
          [
            'use verifier::{ExecutableFacts, verify_executable_facts};',
            'pub fn link_deployment_execution_image() {',
            '  let facts: ExecutableFacts = verify_executable_facts();',
            '  // facts.into_parts();',
            '}',
          ].join('\n'),
        );
        const result = checkExecutableFactsBoundary(sources);
        assert(
          result.violations.some((violation) => violation.includes('into_parts call')),
          'expected comment-only into_parts proof to be rejected',
        );
      },
    },
    {
      name: 'current runtime DAG fixture passes',
      run: () => {
        const result = checkRuntimeDag(metadataFromRuntimeDag());
        assert(result.violations.length === 0, `expected no violations, got ${result.violations.length}`);
      },
    },
    {
      name: 'current DAG rejects an unlisted runtime edge',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const modelPackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-model');
        modelPackage.dependencies.push(runtimeDependency('skiff-runtime-host'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.some(
            (violation) =>
              violation.packageName === 'skiff-runtime-model'
              && violation.message.includes('skiff-runtime-host is not allowed'),
          ),
          'expected skiff-runtime-model -> skiff-runtime-host to be rejected',
        );
      },
    },
    {
      name: 'current DAG rejects request depending on transport',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const requestPackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-request');
        requestPackage.dependencies.push(runtimeDependency('skiff-runtime-transport'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.some(
            (violation) =>
              violation.packageName === 'skiff-runtime-request'
              && violation.message.includes('skiff-runtime-transport is not allowed'),
          ),
          'expected skiff-runtime-request -> skiff-runtime-transport to be rejected',
        );
      },
    },
    {
      name: 'native cannot depend on request execution internals',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const nativePackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-native');
        nativePackage.dependencies.push(runtimeDependency('skiff-runtime-request'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.some(
            (violation) =>
              violation.packageName === 'skiff-runtime-native'
              && violation.message.includes('skiff-runtime-request is not allowed'),
          ),
          'expected skiff-runtime-native -> skiff-runtime-request to be rejected',
        );
      },
    },
    {
      name: 'current DAG ignores dev-only runtime edges',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const modelPackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-model');
        modelPackage.dependencies.push(runtimeDependency('skiff-runtime-host', 'dev'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.length === 0,
          `expected dev-only skiff-runtime-model -> skiff-runtime-host to be ignored, got ${result.violations.length} violations`,
        );
      },
    },
    {
      name: 'host boundary report flags Stage 1 target debt while current DAG passes',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const dagResult = checkRuntimeDag(metadata);
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(dagResult.violations.length === 0, 'expected current DAG fixture to pass');
        assertSameSet(
          hostResult.debts.map((edge) => edge.dependencyName),
          expectedHostBoundaryTargetDebts,
          'expected Stage 1 host target debts',
        );
        assert(
          hostResult.allowed.some(
            (edge) => edge.dependencyName === 'skiff-runtime-config-snapshot',
          ),
          'expected host -> runtime-config-snapshot to be a target-allowed typed storage edge',
        );
        assert(hostResult.unregisteredDebts.length === 0, 'expected no unregistered host target debts');
      },
    },
    {
      name: 'host boundary report and deny modes have staged exit behavior',
      run: () => {
        const hostResult = checkHostBoundaryTarget(metadataFromRuntimeDag());
        assert(hostResult.unregisteredDebts.length === 0, 'expected current fixture debt to be fully registered');
        assert(hostBoundaryExitCode(hostResult, 'report') === 0, 'report mode should exit 0 for debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 1, 'deny mode should exit 1 for debt');
      },
    },
    {
      name: 'host boundary report fails on unregistered target debt',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [
            ...hostBoundaryTarget.allowedRuntimeDeps,
            'skiff-runtime-request-contract',
          ],
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(
          hostResult.unregisteredDebts.some(
            (edge) => edge.dependencyName === 'skiff-runtime-request-contract',
          ),
          'expected skiff-runtime-request-contract to be unregistered host target debt',
        );
        assert(hostBoundaryExitCode(hostResult, 'report') === 1, 'report mode should fail for unregistered debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 1, 'deny mode should fail for unregistered debt');
      },
    },
    {
      name: 'host boundary allows service-db managed-index naming edge',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [...hostBoundaryTarget.allowedRuntimeDeps],
        });
        const dagResult = checkRuntimeDag(metadata);
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(
          dagResult.violations.length === 0,
          `expected service-db edge to pass the DAG, got ${dagResult.violations.length} violations`,
        );
        assert(
          hostResult.allowed.some(
            (edge) => edge.dependencyName === 'skiff-runtime-service-db',
          ),
          'expected skiff-runtime-service-db to be a target-allowed host edge',
        );
        assert(
          hostResult.unregisteredDebts.length === 0,
          'expected no unregistered host target debt',
        );
        assert(hostBoundaryExitCode(hostResult, 'report') === 0, 'report mode should pass for a target-allowed edge');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 0, 'deny mode should pass for a target-allowed edge');
      },
    },
    {
      name: 'host boundary target allow-list has no debt when only target deps remain',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: hostBoundaryTarget.allowedRuntimeDeps,
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(hostResult.debts.length === 0, `expected no host target debt, got ${hostResult.debts.length}`);
        assert(hostResult.unregisteredDebts.length === 0, 'expected no unregistered host target debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 0, 'deny mode should pass without debt');
      },
    },
    {
      name: 'host boundary target ignores dev-only runtime dependencies',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [
            ...hostBoundaryTarget.allowedRuntimeDeps,
            { name: 'skiff-runtime-boundary', kind: 'dev' },
          ],
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(hostResult.debts.length === 0, 'dev-only boundary dependency should not be target debt');
        assert(
          hostResult.ignoredNonProductionDeps.some(
            (edge) => edge.dependencyName === 'skiff-runtime-boundary' && edge.kind === 'dev',
          ),
          'expected dev-only boundary dependency to be reported as ignored',
        );
      },
    },
  ];

  const failures = [];
  for (const testCase of cases) {
    try {
      testCase.run();
      console.log(`PASS ${testCase.name}`);
    } catch (error) {
      failures.push(`${testCase.name}: ${error.message}`);
      console.error(`FAIL ${testCase.name}: ${error.message}`);
    }
  }

  if (failures.length > 0) {
    process.exitCode = 1;
    return;
  }

  console.log(`Runtime crate DAG self-test passed (${cases.length} cases).`);
}

function metadataFromRuntimeDag(options = {}) {
  const packageNames = [...expectedPromotedRuntimePackages].sort();
  const packages = packageNames.map((packageName) => {
    const dependencies =
      packageName === hostBoundaryTarget.hostPackageName && options.hostDependencies !== undefined
        ? options.hostDependencies
        : runtimeDag.get(packageName) ?? [];
    return packageFixture(packageName, dependencies);
  });

  return {
    packages,
    workspace_members: packages.map((pkg) => pkg.id),
  };
}

function executableFactsBoundaryFixture() {
  return new Map([
    [
      executableFactsBoundary.owner,
      [
        'pub struct ExecutableFacts {}',
        'impl ExecutableFacts { pub fn into_parts(self) {} }',
        'pub fn verify_executable_facts() -> ExecutableFacts { ExecutableFacts {} }',
      ].join('\n'),
    ],
    [
      executableFactsBoundary.consumer,
      [
        'use verifier::{ExecutableFacts, verify_executable_facts};',
        'pub fn link_deployment_execution_image() {',
        '  let facts: ExecutableFacts = verify_executable_facts();',
        '  facts.into_parts();',
        '}',
      ].join('\n'),
    ],
    [
      'runtime/request/src/lexically_inert.rs',
      [
        '// ExecutableFacts verify_executable_facts facts.into_parts()',
        'const TEXT: &str = "ExecutableFacts verify_executable_facts";',
        'const RAW: &str = r#"facts.into_parts()"#;',
        '/* nested /* ExecutableFacts */ verify_executable_facts */',
      ].join('\n'),
    ],
  ]);
}

function packageFixture(packageName, dependencies) {
  return {
    id: `${packageName} 0.1.0 (path+file:///workspace/${packageName})`,
    name: packageName,
    manifest_path: `/workspace/${packageName}/Cargo.toml`,
    dependencies: dependencies.map((dependency) =>
      typeof dependency === 'string' ? runtimeDependency(dependency) : runtimeDependency(dependency.name, dependency.kind),
    ),
  };
}

function runtimeDependency(name, kind = null) {
  return { name, kind };
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertSameSet(actual, expected, message) {
  const actualSorted = [...actual].sort();
  const expectedSorted = [...expected].sort();
  if (
    actualSorted.length !== expectedSorted.length
    || actualSorted.some((value, index) => value !== expectedSorted[index])
  ) {
    throw new Error(`${message}: expected ${expectedSorted.join(', ')}, got ${actualSorted.join(', ')}`);
  }
}
