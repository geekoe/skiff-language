import { isAbsolute, join, resolve } from 'node:path';

import { checkerTasks } from './verify-checkers.mjs';
import { assertVerifyCatalogComplete } from './verify-live-catalog.mjs';
import {
  LIVE_REGISTRY,
  LIVE_SELECTORS,
  LIVE_TIERS,
  liveInvocationSelectors,
} from './verify-live-registry.mjs';
import {
  assertRegistryTaskMetadata,
  liveSelectorTasks,
} from './verify-live-plan.mjs';
import {
  discoverScriptTests,
} from './verify-discovery.mjs';
import {
  ORDINARY_LEAF_SELECTORS,
  ORDINARY_PUBLIC_SELECTORS,
  VERIFY_SELECTOR_GRAPH,
  assertOrdinaryTaskBuilderCoverage,
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
  const builders = taskBuilders({
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
  const tasks = [];
  for (const leaf of leaves) {
    const builder = builders[leaf];
    if (!builder) {
      throw new Error(
        `invalid selector ${leaf}; expected one of ${PUBLIC_SELECTORS.join(', ')}`,
      );
    }
    const leafTasks = await builder();
    assertNonEmptyLeaf(leaf, leafTasks);
    tasks.push(...leafTasks);
  }
  assertPlanIntegrity(tasks);
  return { selectors: [...selectors], tasks };
}

export function expandSelectors(selectors) {
  const leaves = [];
  const seenLeaves = new Set();
  for (const selector of selectors) {
    expandSelector(selector, leaves, seenLeaves, new Set());
  }
  return leaves;
}

export function assertPlanIntegrity(tasks) {
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error('verify plan must contain at least one task');
  }
  const seenIds = new Set();
  const seenExecutions = new Map();
  for (const task of tasks) {
    if (
      !isNonEmptyString(task.id)
      || !isNonEmptyString(task.kind)
      || !isNonEmptyString(task.cwd)
    ) {
      throw new Error(`invalid verify task: ${JSON.stringify(task)}`);
    }
    if (seenIds.has(task.id)) {
      throw new Error(`duplicate verify task id: ${task.id}`);
    }
    seenIds.add(task.id);
    assertRegistryTaskMetadata(task);
    assertTaskSchedulingMetadata(task);

    if (task.preconditionError !== undefined) {
      if (
        !isNonEmptyString(task.preconditionError)
        || task.command !== undefined
        || task.args !== undefined
        || task.displayArgs !== undefined
        || task.executionPreflight !== undefined
        || task.mutation !== undefined
      ) {
        throw new Error(`invalid blocked verify task: ${JSON.stringify(task)}`);
      }
      continue;
    }
    if (!isNonEmptyString(task.command) || !Array.isArray(task.args)) {
      throw new Error(`invalid executable verify task: ${JSON.stringify(task)}`);
    }
    if (!task.args.every((arg) => typeof arg === 'string')) {
      throw new Error(`invalid executable verify task args: ${JSON.stringify(task)}`);
    }
    if (task.displayArgs !== undefined && (
      !Array.isArray(task.displayArgs)
      || task.displayArgs.length !== task.args.length
      || !task.displayArgs.every((arg) => typeof arg === 'string')
    )) {
      throw new Error(`invalid verify task displayArgs: ${JSON.stringify(task)}`);
    }
    if (
      task.executionPreflight !== undefined
      && typeof task.executionPreflight !== 'function'
    ) {
      throw new Error(`invalid verify task executionPreflight: ${task.id}`);
    }

    const execution = JSON.stringify([resolve(task.cwd), task.command, task.args]);
    const previousId = seenExecutions.get(execution);
    if (previousId) {
      throw new Error(
        `duplicate verify task execution: ${previousId} and ${task.id}`,
      );
    }
    seenExecutions.set(execution, task.id);
  }
}

export function assertNonEmptyLeaf(leaf, tasks) {
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error(`verify selector leaf ${leaf} produced no tasks`);
  }
}

function assertTaskSchedulingMetadata(task) {
  if (task.slots !== undefined && (!Number.isInteger(task.slots) || task.slots < 1)) {
    throw new Error(`invalid verify task slots for ${task.id}: ${JSON.stringify(task.slots)}`);
  }
  if (task.exclusive !== undefined && typeof task.exclusive !== 'boolean') {
    throw new Error(`invalid verify task exclusive for ${task.id}: ${JSON.stringify(task.exclusive)}`);
  }
  if (task.mutation !== undefined) {
    assertMutationShape(task);
    if (task.exclusive !== true) {
      throw new Error(`mutating verify task must be exclusive: ${task.id}`);
    }
  }
}

function assertMutationShape(task) {
  const mutation = task.mutation;
  if (!mutation || typeof mutation !== 'object' || Array.isArray(mutation)) {
    throw new Error(`invalid verify task mutation for ${task.id}: ${JSON.stringify(mutation)}`);
  }
  const keys = Object.keys(mutation).sort();
  if (keys.join(',') !== 'paths,redirect') {
    throw new Error(`invalid verify task mutation for ${task.id}: ${JSON.stringify(mutation)}`);
  }
  const { paths, redirect } = mutation;
  if (
    !Array.isArray(paths)
    || paths.length === 0
    || !paths.every(isRepoRelativePath)
    || new Set(paths).size !== paths.length
  ) {
    throw new Error(`invalid verify task mutation paths for ${task.id}: ${JSON.stringify(paths)}`);
  }
  if (
    !redirect
    || typeof redirect !== 'object'
    || Array.isArray(redirect)
    || Object.keys(redirect).length === 0
  ) {
    throw new Error(`invalid verify task mutation redirect for ${task.id}: ${JSON.stringify(redirect)}`);
  }
  for (const [name, path] of Object.entries(redirect)) {
    if (!isEnvVarName(name)) {
      throw new Error(`invalid verify task mutation redirect key for ${task.id}: ${JSON.stringify(name)}`);
    }
    if (typeof path !== 'string' || !paths.includes(path)) {
      throw new Error(`invalid verify task mutation redirect value for ${task.id}: ${JSON.stringify(path)}`);
    }
  }
}

function isRepoRelativePath(value) {
  return typeof value === 'string'
    && value.trim().length > 0
    && !isAbsolute(value)
    && !value.split('/').includes('..');
}

function isEnvVarName(value) {
  return typeof value === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(value);
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

function taskBuilders({
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
      task(root, 'skiff-tests:canonical', 'skiff-tests', 'node', [
        'scripts/run-skiff-tests.mjs',
      ]),
    ],
    ...rustSubjectTaskBuilders(root),
    'rust-quality': async () => [
      task(root, 'rust-quality:format', 'rust-quality', 'cargo', [
        'fmt',
        '--all',
        '--',
        '--check',
      ]),
      ...await checkerTasks(root, 'rust-quality'),
    ],
    'router-type-check': async () => [
      packageTask(root, 'router:type-check', 'router', 'router', ['run', 'type-check']),
    ],
    'router-rust-process-smoke': async () => [
      task(root, 'router-rust:process-smoke', 'implementation:router', 'node', [
        'scripts/run-router-process-smoke.mjs',
      ]),
    ],
    'telemetry-type-check': async () => [
      packageTask(
        root,
        'telemetry:type-check',
        'telemetry',
        'telemetry',
        ['run', 'type-check'],
      ),
    ],
    'telemetry-tests': async () => [
      packageTask(
        root,
        'implementation:telemetry',
        'implementation:telemetry',
        'telemetry',
        ['test'],
      ),
    ],
    'scripts-syntax': async () => checkerTasks(root, 'scripts-syntax', {
      kind: 'scripts',
    }),
    'scripts-tests': async () => scriptTestTasks(root),
    'scripts-dev-sync': async () => checkerTasks(root, 'scripts-dev-sync', {
      kind: 'implementation:tooling',
    }),
    'vscode-type-check': async () => [
      packageTask(root, 'vscode:type-check', 'vscode', 'vscode', ['run', 'type-check']),
    ],
    'vscode-grammar': async () => [
      packageTask(
        root,
        'implementation:tooling:vscode-grammar',
        'implementation:tooling',
        'vscode',
        ['run', 'test:grammar'],
      ),
    ],
    'checks-default': async () => checkerTasks(root, 'checks'),
    'compiler-boundaries': async () => checkerTasks(root, 'compiler-boundaries'),
    'runtime-execution-boundaries': async () =>
      checkerTasks(root, 'runtime-execution-boundaries', {
        kind: 'implementation:runtime',
      }),
    'runtime-eval-error-boundary': async () =>
      checkerTasks(root, 'runtime-eval-error-boundary', {
        kind: 'implementation:runtime',
      }),
  };
  assertOrdinaryTaskBuilderCoverage(builders);
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
      ...await liveSelectorTasks(root, selector, registryOptions),
    ];
  }
  for (const selector of liveSelectors) {
    if (Object.hasOwn(builders, selector)) {
      throw new Error(`live selector conflicts with verify task builder: ${selector}`);
    }
    builders[selector] = async () =>
      liveSelectorTasks(root, selector, registryOptions);
  }
  return builders;
}

function rustSubjectTaskBuilders(root) {
  return Object.fromEntries(
    RUST_IMPLEMENTATION_SUBJECTS.map((subject) => [
      subject.leafSelector,
      async () => [
        ...(await checkerTasks(root, subject.selector, {
          kind: `implementation:${subject.selector}`,
        })),
        task(
          root,
          subject.taskId,
          `implementation:${subject.selector}`,
          'cargo',
          rustSubjectTestArgs(subject),
        ),
      ],
    ]),
  );
}

async function scriptTestTasks(root) {
  const files = await discoverScriptTests(root);
  if (files.length === 0) {
    return [];
  }
  return [
    task(
      root,
      'implementation:tooling:scripts-tests',
      'implementation:tooling',
      'node',
      ['--test', ...files],
    ),
  ];
}

function packageTask(root, id, kind, directory, args) {
  return task(join(root, directory), id, kind, 'pnpm', args);
}

function task(cwd, id, kind, command, args, options = {}) {
  return { id, kind, command, args, cwd, ...options };
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}
