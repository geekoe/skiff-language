import { join, resolve } from 'node:path';

import { checkerPhases } from './verify-checkers.mjs';
import { assertVerifyCatalogComplete } from './verify-live-catalog.mjs';
import {
  LIVE_REGISTRY,
  LIVE_SELECTORS,
  LIVE_TIERS,
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
  ORDINARY_LEAF_SELECTORS,
  ORDINARY_PUBLIC_SELECTORS,
  VERIFY_SELECTOR_GRAPH,
  assertOrdinaryPhaseBuilderCoverage,
} from './verify-selector-graph.mjs';
import {
  RUST_IMPLEMENTATION_SUBJECTS,
  rustSubjectTestArgs,
} from './verify-rust-subjects.mjs';

export const PUBLIC_SELECTORS = Object.freeze([
  ...ORDINARY_PUBLIC_SELECTORS,
  ...LIVE_SELECTORS,
]);

const SELECTOR_EXPANSIONS = VERIFY_SELECTOR_GRAPH.expansions;

export async function buildVerifyPlan({
  root,
  selectors = ['verify'],
  runtimeLiveActivationUrl,
  runtimeLiveIngressUrl,
  runtimeLiveArtifactRoot,
  runtimeLiveEnvironment,
  runtimeLiveExpectedGeneration,
  loopRiskConfig,
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
    runtimeLiveActivationUrl,
    runtimeLiveIngressUrl,
    runtimeLiveArtifactRoot,
    runtimeLiveEnvironment,
    runtimeLiveExpectedGeneration,
    loopRiskConfig,
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
  runtimeLiveActivationUrl,
  runtimeLiveIngressUrl,
  runtimeLiveArtifactRoot,
  runtimeLiveEnvironment,
  runtimeLiveExpectedGeneration,
  loopRiskConfig,
  env,
  liveRegistry,
  liveSelectors,
}) {
  const registryOptions = {
    runtimeLiveActivationUrl,
    runtimeLiveIngressUrl,
    runtimeLiveArtifactRoot,
    runtimeLiveEnvironment,
    runtimeLiveExpectedGeneration,
    loopRiskConfig,
    env,
    registry: liveRegistry,
  };
  const builders = {
    'skiff-tests': async () => [
      phase(root, 'skiff-tests:canonical', 'skiff-tests', 'node', [
        'scripts/run-skiff-tests.mjs',
      ]),
    ],
    ...rustSubjectPhaseBuilders(root),
    'rust-quality': async () => [
      phase(root, 'rust-quality:format', 'rust-quality', 'cargo', [
        'fmt',
        '--all',
        '--',
        '--check',
      ]),
      ...await checkerPhases(root, 'rust-quality'),
    ],
    'router-type-check': async () => [
      packagePhase(root, 'router:type-check', 'router', 'router', ['run', 'type-check']),
    ],
    'router-tests': async () => [
      packagePhase(
        root,
        'implementation:router',
        'implementation:router',
        'router',
        ['test'],
      ),
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
    'telemetry-tests': async () => [
      packagePhase(
        root,
        'implementation:telemetry',
        'implementation:telemetry',
        'telemetry',
        ['test'],
      ),
    ],
    'scripts-syntax': async () => javascriptSyntaxPhases(root),
    'scripts-tests': async () => scriptTestPhases(root),
    'scripts-dev-sync': async () => checkerPhases(root, 'scripts-dev-sync', {
      kind: 'implementation:tooling',
    }),
    'vscode-type-check': async () => [
      packagePhase(root, 'vscode:type-check', 'vscode', 'vscode', ['run', 'type-check']),
    ],
    'vscode-grammar': async () => [
      packagePhase(
        root,
        'implementation:tooling:vscode-grammar',
        'implementation:tooling',
        'vscode',
        ['run', 'test:grammar'],
      ),
    ],
    'checks-default': async () => checkerPhases(root, 'checks'),
    'compiler-boundaries': async () => checkerPhases(root, 'compiler-boundaries'),
  };
  assertOrdinaryPhaseBuilderCoverage(builders);
  const ordinaryLeaves = new Set(ORDINARY_LEAF_SELECTORS);
  const selfTestSelectors = liveInvocationSelectors(liveRegistry, {
    tier: LIVE_TIERS.SELF_TEST,
  });
  for (const selector of selfTestSelectors) {
    if (!ordinaryLeaves.has(selector)) {
      throw new Error(`registry self-test must target ordinary leaf selector: ${selector}`);
    }
    const ordinaryBuilder = builders[selector];
    builders[selector] = async () => [
      ...await ordinaryBuilder(),
      ...await liveSelectorPhases(root, selector, registryOptions),
    ];
  }
  for (const selector of liveSelectors) {
    if (Object.hasOwn(builders, selector)) {
      throw new Error(`live selector conflicts with verify phase builder: ${selector}`);
    }
    builders[selector] = async () =>
      liveSelectorPhases(root, selector, registryOptions);
  }
  return builders;
}

function rustSubjectPhaseBuilders(root) {
  return Object.fromEntries(
    RUST_IMPLEMENTATION_SUBJECTS.map((subject) => [
      subject.leafSelector,
      async () => [
        ...(await checkerPhases(root, subject.selector, {
          kind: `implementation:${subject.selector}`,
        })),
        phase(
          root,
          subject.phaseId,
          `implementation:${subject.selector}`,
          'cargo',
          rustSubjectTestArgs(subject),
        ),
      ],
    ]),
  );
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
    phase(
      root,
      `implementation:tooling:${path}`,
      'implementation:tooling',
      'node',
      ['--test', path],
    ),
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
