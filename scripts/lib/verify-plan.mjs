import { existsSync, statSync } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';

import { checkerPhases } from './verify-checkers.mjs';
import { parseRuntimeReloadUrl } from './runtime-reload-url.mjs';
import {
  discoverJavaScriptFiles,
  discoverRuntimeLiveTests,
  discoverScriptTests,
  repoRelative,
} from './verify-discovery.mjs';

export const PUBLIC_SELECTORS = Object.freeze([
  'verify',
  'node',
  'rust',
  'router',
  'telemetry',
  'scripts',
  'scripts-dev-sync',
  'scripts-syntax',
  'vscode',
  'checks',
  'type-check',
  'compiler-boundaries',
  'runtime-live',
  'db-encrypted-storage-live',
]);

const SELECTOR_EXPANSIONS = Object.freeze({
  verify: ['rust', 'node', 'checks'],
  checks: ['compiler-boundaries', 'checks-default'],
  node: ['router', 'telemetry', 'scripts', 'vscode'],
  router: ['router-type-check', 'router-test'],
  telemetry: ['telemetry-type-check', 'telemetry-test'],
  scripts: ['scripts-syntax', 'scripts-tests', 'scripts-dev-sync'],
  vscode: ['vscode-type-check', 'vscode-grammar'],
  'type-check': [
    'router-type-check',
    'telemetry-type-check',
    'scripts-syntax',
    'vscode-type-check',
  ],
});

export async function buildVerifyPlan({
  root,
  selectors = ['verify'],
  runtimeLiveConfig,
  runtimeLiveReloadUrl,
  runtimeLiveArtifactRoot,
  env = process.env,
} = {}) {
  if (!root) {
    throw new Error('verify plan requires the repository root');
  }
  const leaves = expandSelectors(selectors);
  const builders = phaseBuilders({
    root,
    runtimeLiveConfig,
    runtimeLiveReloadUrl,
    runtimeLiveArtifactRoot,
    env,
  });
  const phases = [];
  for (const leaf of leaves) {
    const builder = builders[leaf];
    if (!builder) {
      throw new Error(
        `invalid selector ${leaf}; expected one of ${PUBLIC_SELECTORS.join(', ')}`,
      );
    }
    const leafPhases = await builder();
    assertNonEmptyLeaf(leaf, leafPhases);
    phases.push(...leafPhases);
  }
  assertPlanIntegrity(phases);
  return { selectors: [...selectors], phases };
}

export function expandSelectors(selectors) {
  const leaves = [];
  const seenLeaves = new Set();
  for (const selector of selectors) {
    expandSelector(selector, leaves, seenLeaves, new Set());
  }
  return leaves;
}

export function assertPlanIntegrity(phases) {
  if (!Array.isArray(phases) || phases.length === 0) {
    throw new Error('verify plan must contain at least one phase');
  }
  const seenIds = new Set();
  const seenExecutions = new Map();
  for (const phase of phases) {
    if (
      !isNonEmptyString(phase.id)
      || !isNonEmptyString(phase.kind)
      || !isNonEmptyString(phase.cwd)
    ) {
      throw new Error(`invalid verify phase: ${JSON.stringify(phase)}`);
    }
    if (seenIds.has(phase.id)) {
      throw new Error(`duplicate verify phase id: ${phase.id}`);
    }
    seenIds.add(phase.id);

    if (phase.preconditionError !== undefined) {
      if (
        !isNonEmptyString(phase.preconditionError)
        || phase.command !== undefined
        || phase.args !== undefined
        || phase.displayArgs !== undefined
        || phase.executionPreflight !== undefined
      ) {
        throw new Error(`invalid blocked verify phase: ${JSON.stringify(phase)}`);
      }
      continue;
    }
    if (!isNonEmptyString(phase.command) || !Array.isArray(phase.args)) {
      throw new Error(`invalid executable verify phase: ${JSON.stringify(phase)}`);
    }
    if (!phase.args.every((arg) => typeof arg === 'string')) {
      throw new Error(`invalid executable verify phase args: ${JSON.stringify(phase)}`);
    }
    if (phase.displayArgs !== undefined && (
      !Array.isArray(phase.displayArgs)
      || phase.displayArgs.length !== phase.args.length
      || !phase.displayArgs.every((arg) => typeof arg === 'string')
    )) {
      throw new Error(`invalid verify phase displayArgs: ${JSON.stringify(phase)}`);
    }
    if (
      phase.executionPreflight !== undefined
      && typeof phase.executionPreflight !== 'function'
    ) {
      throw new Error(`invalid verify phase executionPreflight: ${phase.id}`);
    }

    const execution = JSON.stringify([resolve(phase.cwd), phase.command, phase.args]);
    const previousId = seenExecutions.get(execution);
    if (previousId) {
      throw new Error(
        `duplicate verify phase execution: ${previousId} and ${phase.id}`,
      );
    }
    seenExecutions.set(execution, phase.id);
  }
}

export function assertNonEmptyLeaf(leaf, phases) {
  if (!Array.isArray(phases) || phases.length === 0) {
    throw new Error(`verify selector leaf ${leaf} produced no phases`);
  }
}

function expandSelector(selector, leaves, seenLeaves, active) {
  const expansion = SELECTOR_EXPANSIONS[selector];
  if (!expansion) {
    if (!seenLeaves.has(selector)) {
      seenLeaves.add(selector);
      leaves.push(selector);
    }
    return;
  }
  if (active.has(selector)) {
    throw new Error(`cyclic verify selector expansion: ${[...active, selector].join(' -> ')}`);
  }
  const nextActive = new Set(active).add(selector);
  for (const child of expansion) {
    expandSelector(child, leaves, seenLeaves, nextActive);
  }
}

function phaseBuilders({
  root,
  runtimeLiveConfig,
  runtimeLiveReloadUrl,
  runtimeLiveArtifactRoot,
  env,
}) {
  return {
    rust: async () => [
      phase(root, 'rust:workspace', 'rust', 'cargo', [
        'test',
        '--workspace',
        '--no-fail-fast',
      ]),
    ],
    'router-type-check': async () => [
      packagePhase(root, 'router:type-check', 'router', 'router', ['run', 'type-check']),
    ],
    'router-test': async () => [
      packagePhase(root, 'router:test', 'router', 'router', ['test']),
    ],
    'telemetry-type-check': async () => [
      packagePhase(
        root,
        'telemetry:type-check',
        'telemetry',
        'telemetry',
        ['run', 'type-check'],
      ),
    ],
    'telemetry-test': async () => [
      packagePhase(root, 'telemetry:test', 'telemetry', 'telemetry', ['test']),
    ],
    'scripts-syntax': async () => javascriptSyntaxPhases(root),
    'scripts-tests': async () => scriptTestPhases(root),
    'scripts-dev-sync': async () => [
      phase(root, 'scripts:dev-sync-fixture', 'scripts', 'node', [
        'scripts/skiff-dev-sync.mjs',
        '--check-sync',
        '--root',
        'compiler/tests/fixtures/router-websocket-fixture',
      ]),
    ],
    'vscode-type-check': async () => [
      packagePhase(root, 'vscode:type-check', 'vscode', 'vscode', ['run', 'type-check']),
    ],
    'vscode-grammar': async () => [
      packagePhase(root, 'vscode:grammar', 'vscode', 'vscode', ['run', 'test:grammar']),
    ],
    'checks-default': async () => checkerPhases(root, 'checks'),
    'compiler-boundaries': async () => checkerPhases(root, 'compiler-boundaries'),
    'db-encrypted-storage-live': async () =>
      checkerPhases(root, 'db-encrypted-storage-live'),
    'runtime-live': async () => runtimeLivePhases(root, {
      configuredConfigPath: runtimeLiveConfig,
      configuredReloadUrl: runtimeLiveReloadUrl,
      configuredArtifactRoot: runtimeLiveArtifactRoot,
      env,
    }),
  };
}

async function javascriptSyntaxPhases(root) {
  const files = await discoverJavaScriptFiles(root);
  return files.map((file) => {
    const path = repoRelative(root, file);
    return phase(root, `javascript:syntax:${path}`, 'scripts', 'node', ['--check', path]);
  });
}

async function scriptTestPhases(root) {
  const files = await discoverScriptTests(root);
  return files.map((path) =>
    phase(root, `scripts:test:${path}`, 'scripts', 'node', ['--test', path]),
  );
}

async function runtimeLivePhases(root, {
  configuredConfigPath,
  configuredReloadUrl,
  configuredArtifactRoot,
  env,
}) {
  const rawConfigPath = nonEmptyValue(
    configuredConfigPath,
    env.SKIFF_RUNTIME_LIVE_CONFIG,
  );
  const rawReloadUrl = nonEmptyValue(
    configuredReloadUrl,
    env.SKIFF_RUNTIME_LIVE_RELOAD_URL,
  );
  const rawArtifactRoot = nonEmptyValue(
    configuredArtifactRoot,
    env.SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT,
  );
  const missing = [];
  if (!rawConfigPath) {
    missing.push(
      'runtime config (SKIFF_RUNTIME_LIVE_CONFIG or --runtime-live-config <path>)',
    );
  }
  if (!rawReloadUrl) {
    missing.push(
      'router reload URL (SKIFF_RUNTIME_LIVE_RELOAD_URL or --runtime-live-reload-url <url>)',
    );
  }
  if (!rawArtifactRoot) {
    missing.push(
      'artifact root (SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT or --runtime-live-artifact-root <dir>)',
    );
  }
  if (missing.length > 0) {
    return [
      blockedPhase(
        root,
        'live:runtime:inputs',
        'live/manual',
        `runtime-live is missing required explicit input(s): ${missing.join('; ')}`,
      ),
    ];
  }
  const configPath = resolveInputPath(root, rawConfigPath);
  if (!isFile(configPath)) {
    throw new Error(`runtime-live config path must be an existing file: ${configPath}`);
  }
  const artifactRoot = resolveInputPath(root, rawArtifactRoot);
  if (!isDirectory(artifactRoot)) {
    throw new Error(`runtime-live artifact root must be an existing directory: ${artifactRoot}`);
  }
  const reloadTarget = parseRuntimeReloadUrl(rawReloadUrl);

  const files = await discoverRuntimeLiveTests(root);
  if (files.length === 0) {
    throw new Error('runtime-live found no *.live.test.skiff fixtures under runtime/live-tests');
  }
  const packageStore = join(root, 'runtime', 'live-tests', 'package-store');
  const packageArgs = existsSync(packageStore) ? ['--packages-dir', packageStore] : [];
  const executionPreflight = () => {
    const failures = [];
    if (!isFile(configPath)) {
      failures.push(`runtime-live config path is no longer an existing file: ${configPath}`);
    }
    if (!isDirectory(artifactRoot)) {
      failures.push(
        `runtime-live artifact root is no longer an existing directory: ${artifactRoot}`,
      );
    }
    try {
      parseRuntimeReloadUrl(reloadTarget.normalized);
    } catch (error) {
      failures.push(
        error instanceof Error
          ? error.message
          : 'runtime-live reload URL failed execution preflight validation',
      );
    }
    return failures.length === 0 ? undefined : failures;
  };
  return files.map((file) => {
    const args = [
      'run',
      '--manifest-path',
      'test-runner/Cargo.toml',
      '--',
      file,
      '--live',
      '--allow-network',
      '--config',
      configPath,
      '--router-reload-url',
      reloadTarget.normalized,
      '--artifact-root',
      artifactRoot,
      '--deny-skips',
      '--require-tests',
      ...packageArgs,
    ];
    const displayArgs = [...args];
    displayArgs[displayArgs.indexOf('--router-reload-url') + 1] = reloadTarget.display;
    return phase(
      root,
      `live:runtime:${repoRelative(root, file)}`,
      'live/manual',
      'cargo',
      args,
      { displayArgs, executionPreflight },
    );
  });
}

function packagePhase(root, id, kind, directory, args) {
  return phase(join(root, directory), id, kind, 'pnpm', args);
}

function phase(cwd, id, kind, command, args, options = {}) {
  return { id, kind, command, args, cwd, ...options };
}

function blockedPhase(cwd, id, kind, preconditionError) {
  return { id, kind, cwd, preconditionError };
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

function nonEmptyValue(configured, environment) {
  if (configured !== undefined) {
    return typeof configured === 'string' && configured.length > 0 ? configured : undefined;
  }
  return typeof environment === 'string' && environment.length > 0 ? environment : undefined;
}

function resolveInputPath(root, path) {
  return isAbsolute(path) ? path : resolve(root, path);
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function isDirectory(path) {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}
