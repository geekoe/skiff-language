import { join, resolve } from 'node:path';

import { checkerPhases } from './verify-checkers.mjs';
import { assertVerifyCatalogComplete } from './verify-live-catalog.mjs';
import {
  LIVE_REGISTRY,
  LIVE_SELECTORS,
  liveInvocationSelectors,
} from './verify-live-registry.mjs';
import {
  assertRegistryPhaseMetadata,
  liveSelectorPhases,
} from './verify-live-plan.mjs';
import {
  discoverJavaScriptFiles,
  discoverScriptTests,
  repoRelative,
} from './verify-discovery.mjs';
import {
  ORDINARY_PUBLIC_SELECTORS,
  VERIFY_SELECTOR_GRAPH,
  assertOrdinaryPhaseBuilderCoverage,
} from './verify-selector-graph.mjs';

export const PUBLIC_SELECTORS = Object.freeze([
  ...ORDINARY_PUBLIC_SELECTORS,
  ...LIVE_SELECTORS,
]);

const SELECTOR_EXPANSIONS = VERIFY_SELECTOR_GRAPH.expansions;

export async function buildVerifyPlan({
  root,
  selectors = ['verify'],
  runtimeLiveConfig,
  runtimeLiveReloadUrl,
  runtimeLiveArtifactRoot,
  env = process.env,
  catalogRoot = root,
  liveRegistry = LIVE_REGISTRY,
} = {}) {
  if (!root) {
    throw new Error('verify plan requires the repository root');
  }
  await assertVerifyCatalogComplete(catalogRoot, { liveRegistry });
  const liveSelectors = liveInvocationSelectors(liveRegistry);
  const leaves = expandSelectors(selectors);
  const builders = phaseBuilders({
    root,
    runtimeLiveConfig,
    runtimeLiveReloadUrl,
    runtimeLiveArtifactRoot,
    env,
    liveRegistry,
    liveSelectors,
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
    assertRegistryPhaseMetadata(phase);

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
  liveRegistry,
  liveSelectors,
}) {
  const builders = {
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
  };
  assertOrdinaryPhaseBuilderCoverage(builders);
  for (const selector of liveSelectors) {
    if (Object.hasOwn(builders, selector)) {
      throw new Error(`live selector conflicts with verify phase builder: ${selector}`);
    }
    builders[selector] = async () => liveSelectorPhases(root, selector, {
      runtimeLiveConfig,
      runtimeLiveReloadUrl,
      runtimeLiveArtifactRoot,
      env,
      registry: liveRegistry,
    });
  }
  return builders;
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

function packagePhase(root, id, kind, directory, args) {
  return phase(join(root, directory), id, kind, 'pnpm', args);
}

function phase(cwd, id, kind, command, args, options = {}) {
  return { id, kind, command, args, cwd, ...options };
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}
