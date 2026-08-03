import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import {
  access,
  chmod,
  mkdtemp,
  mkdir,
  rm,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { CHECKER_REGISTRY } from '../lib/verify-checkers.mjs';
import {
  assertPlanIntegrity,
  buildVerifyPlan,
  PUBLIC_SELECTORS,
} from '../lib/verify-plan.mjs';
import { runVerifyPlan } from '../lib/verify-runner.mjs';
import {
  assertVerifyCatalogComplete,
} from '../lib/verify-live-catalog.mjs';
import {
  assertLiveRegistryIntegrity,
  LIVE_OWNERSHIP,
  LIVE_PLAN_TYPES,
  LIVE_REGISTRY,
  LIVE_SELECTORS,
  LIVE_TIERS,
  renderLiveSelectorHelp,
} from '../lib/verify-live-registry.mjs';
import { ORDINARY_SELECTOR_NAMES } from '../lib/verify-selector-graph.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('live registry is the single declaration for current selectors, policies, and prerequisites', () => {
  assert.doesNotThrow(() => assertLiveRegistryIntegrity(LIVE_REGISTRY));
  assert.deepEqual(LIVE_SELECTORS, [
    'runtime-live',
    'db-encrypted-storage-live',
    'router-live:bootstrap',
    'router-live:session',
    'router-live:dispatch',
    'router-live:activation-full-chain',
    'router-live:ws',
    'router-live:actor',
    'durable-task-e2e-live',
    'router-live:http',
    'router-live:chat',
    'router-live:clean-host',
    'loop-risk-health-live',
    'loop-risk-stress-live',
  ]);
  assert.deepEqual(
    PUBLIC_SELECTORS.filter((selector) => LIVE_SELECTORS.includes(selector)),
    LIVE_SELECTORS,
  );

  const runtime = invocation('runtime-live');
  assert.equal(runtime.entry.source.type, 'discovery');
  assert.equal(runtime.entry.source.discovery, 'runtime-live-tests');
  assert.equal(runtime.value.plan, LIVE_PLAN_TYPES.RUNTIME_FIXTURES);
  assert.equal(runtime.value.idPrefix, 'live:runtime:');
  assert.equal(runtime.value.ownership, LIVE_OWNERSHIP.EXTERNAL);
  assert.equal(runtime.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.deepEqual(runtime.value.requiredInputs, [
    'runtimeActivationUrl',
    'runtimeIngressUrl',
    'runtimeArtifactRoot',
    'runtimeEnvironment',
    'runtimeExpectedGeneration',
  ]);
  assert.deepEqual(runtime.value.requiredExecutables, ['cargo', 'node']);
  assert.deepEqual(runtime.value.canonicalPolicy, {
    forbidSkips: true,
    forbidUnchecked: true,
  });

  const database = invocation('db-encrypted-storage-live');
  assert.equal(database.entry.source.type, 'script');
  assert.equal(
    database.entry.source.path,
    'scripts/check-db-encrypted-storage-live.mjs',
  );
  assert.equal(database.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(database.value.id, 'live:db-encrypted-storage');
  assert.equal(database.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(database.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.deepEqual(database.value.requiredInputs, []);
  assert.deepEqual(database.value.requiredExecutables, [
    'node',
    'cargo',
    'pnpm',
    'mongod',
    'mongosh',
  ]);
  assert.deepEqual(database.value.canonicalPolicy, {
    forbidSkips: false,
    forbidUnchecked: true,
  });

  const dispatch = invocation('router-live:dispatch');
  assert.equal(dispatch.entry.key, 'router-rust-dispatch-live');
  assert.equal(dispatch.entry.source.type, 'script');
  assert.equal(
    dispatch.entry.source.path,
    'scripts/check-router-dispatch-live.mjs',
  );
  assert.equal(dispatch.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(dispatch.value.id, 'live:router-rust-dispatch');
  assert.equal(dispatch.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(dispatch.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.deepEqual(dispatch.value.requiredInputs, []);
  assert.deepEqual(dispatch.value.requiredExecutables, [
    'node',
    'cargo',
    'mongod',
    'mongosh',
  ]);
  assert.deepEqual(dispatch.value.canonicalPolicy, {
    forbidSkips: false,
    forbidUnchecked: true,
  });

  const ws = invocation('router-live:ws');
  assert.equal(ws.entry.key, 'router-rust-ws-live');
  assert.equal(ws.entry.source.type, 'script');
  assert.equal(ws.entry.source.path, 'scripts/check-router-ws-live.mjs');
  assert.equal(ws.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(ws.value.id, 'live:router-rust-ws');
  assert.equal(ws.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(ws.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.deepEqual(ws.value.requiredInputs, []);
  assert.deepEqual(ws.value.requiredExecutables, [
    'node',
    'cargo',
    'mongod',
    'mongosh',
  ]);
  assert.deepEqual(ws.value.canonicalPolicy, {
    forbidSkips: false,
    forbidUnchecked: true,
  });

  const actor = invocation('router-live:actor');
  assert.equal(actor.entry.key, 'router-rust-actor-live');
  assert.equal(actor.entry.source.type, 'script');
  assert.equal(
    actor.entry.source.path,
    'scripts/check-router-actor-live.mjs',
  );
  assert.equal(actor.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(actor.value.id, 'live:router-rust-actor');
  assert.equal(actor.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(actor.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.match(actor.value.description, /Rust-only two-replica actor full chain/);
  assert.deepEqual(actor.value.requiredInputs, []);
  assert.deepEqual(actor.value.requiredExecutables, [
    'node',
    'cargo',
    'mongod',
    'mongosh',
  ]);
  assert.deepEqual(actor.value.requiredModules, []);
  assert.deepEqual(actor.value.canonicalPolicy, {
    forbidSkips: false,
    forbidUnchecked: true,
  });

  const durableTask = invocation('durable-task-e2e-live');
  assert.equal(durableTask.entry.key, 'durable-task-e2e-live');
  assert.equal(durableTask.entry.source.type, 'script');
  assert.equal(
    durableTask.entry.source.path,
    'scripts/check-durable-task-e2e-live.mjs',
  );
  assert.equal(durableTask.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(durableTask.value.id, 'live:durable-task-e2e');
  assert.equal(durableTask.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(durableTask.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.match(durableTask.value.description, /durable task dispatch vertical chain/);
  assert.deepEqual(durableTask.value.requiredInputs, []);
  assert.deepEqual(durableTask.value.requiredExecutables, [
    'node',
    'cargo',
    'mongosh',
  ]);
  assert.deepEqual(durableTask.value.requiredModules, []);
  assert.deepEqual(durableTask.value.canonicalPolicy, {
    forbidSkips: false,
    forbidUnchecked: true,
  });

  const cleanHost = invocation('router-live:clean-host');
  assert.equal(cleanHost.entry.key, 'router-rust-clean-host-live');
  assert.equal(cleanHost.entry.source.type, 'script');
  assert.equal(
    cleanHost.entry.source.path,
    'scripts/check-router-clean-host-live.mjs',
  );
  assert.equal(cleanHost.value.plan, LIVE_PLAN_TYPES.FIXED_COMMAND);
  assert.equal(cleanHost.value.id, 'live:router-rust-clean-host');
  assert.equal(cleanHost.value.ownership, LIVE_OWNERSHIP.MANAGED);
  assert.equal(cleanHost.value.tier, LIVE_TIERS.LIVE_MANUAL);
  assert.match(cleanHost.value.description, /Rust-only clean-host release rehearsal/);
  assert.deepEqual(cleanHost.value.requiredInputs, []);
  assert.deepEqual(cleanHost.value.requiredExecutables, [
    'node',
    'cargo',
    'mongod',
    'mongosh',
  ]);
  assert.deepEqual(cleanHost.value.requiredModules, []);
  assert.deepEqual(cleanHost.value.canonicalPolicy, {
    forbidSkips: true,
    forbidUnchecked: true,
  });

  const healthSelfTest = invocation('checks-default');
  assert.equal(healthSelfTest.entry.source.path, 'scripts/check-loop-risk-health.mjs');
  assert.equal(healthSelfTest.value.id, 'checks:loop-risk-health:self-test');
  assert.equal(healthSelfTest.value.ownership, LIVE_OWNERSHIP.NONE);
  assert.equal(healthSelfTest.value.tier, LIVE_TIERS.SELF_TEST);
  assert.deepEqual(healthSelfTest.value.args, ['--self-test']);

  const health = invocation('loop-risk-health-live');
  assert.equal(health.entry.source.path, 'scripts/check-loop-risk-health.mjs');
  assert.equal(health.value.configProfile, 'health');
  assert.deepEqual(health.value.requiredInputs, ['loopRiskConfig']);
  assert.deepEqual(health.value.requiredExecutables, ['node']);

  const stress = invocation('loop-risk-stress-live');
  assert.equal(stress.entry.source.path, 'scripts/check-loop-risk-stress-live.mjs');
  assert.equal(stress.value.configProfile, 'stress');
  assert.deepEqual(stress.value.requiredExecutables, ['node', 'ps']);
  assert.deepEqual(stress.value.requiredModules, [
    { specifier: 'ws', from: 'scripts/package.json' },
  ]);
});

test('live selector help is rendered from registry invocation descriptions', async () => {
  const rendered = renderLiveSelectorHelp();
  for (const { selector, description, tier } of LIVE_REGISTRY.flatMap((entry) => entry.invocations)) {
    if (tier !== LIVE_TIERS.LIVE_MANUAL) {
      continue;
    }
    assert.match(rendered, new RegExp(`${escapeRegExp(selector)}.*${escapeRegExp(description)}`));
  }
  const result = await runProcess(process.execPath, [verifyPath, '--help'], {
    cwd: root,
  });
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, new RegExp(escapeRegExp(rendered)));
});

test('live registry schema rejects incomplete entries, duplicate selectors, and invalid source shapes', () => {
  const noInvocation = cloneRegistry();
  noInvocation[0].invocations = [];
  assert.throws(
    () => assertLiveRegistryIntegrity(noInvocation),
    /must declare at least one invocation/,
  );

  const duplicateSelector = cloneRegistry();
  duplicateSelector[1].invocations[0].selector = 'runtime-live';
  assert.throws(
    () => assertLiveRegistryIntegrity(duplicateSelector),
    /duplicate live registry selector/,
  );

  const missingSource = cloneRegistry();
  delete missingSource[0].source;
  assert.throws(
    () => assertLiveRegistryIntegrity(missingSource),
    /requires a source owner/,
  );

  const unsupportedDiscovery = cloneRegistry();
  unsupportedDiscovery[0].source.discovery = 'unknown-discovery';
  assert.throws(
    () => assertLiveRegistryIntegrity(unsupportedDiscovery),
    /invalid discovery source/,
  );

  const wrongPlanSource = cloneRegistry();
  wrongPlanSource[0].invocations[0].plan = LIVE_PLAN_TYPES.FIXED_COMMAND;
  assert.throws(
    () => assertLiveRegistryIntegrity(wrongPlanSource),
    /fixed command invocation .* invalid shape/,
  );
});

test('live registry schema rejects invalid ownership-tier pairs and prerequisite declarations', () => {
  for (const mutate of [
    (registry) => {
      registry[0].invocations[0].ownership = LIVE_OWNERSHIP.NONE;
    },
    (registry) => {
      registry[1].invocations[0].tier = LIVE_TIERS.SELF_TEST;
    },
    (registry) => {
      registry[1].invocations[0].ownership = 'unknown-owner';
    },
  ]) {
    const registry = cloneRegistry();
    mutate(registry);
    assert.throws(() => assertLiveRegistryIntegrity(registry), /ownership/);
  }

  const duplicateExecutable = cloneRegistry();
  duplicateExecutable[0].invocations[0].requiredExecutables = ['cargo', 'cargo'];
  assert.throws(
    () => assertLiveRegistryIntegrity(duplicateExecutable),
    /requiredExecutables must be a unique string array/,
  );

  const unknownInput = cloneRegistry();
  unknownInput[0].invocations[0].requiredInputs.push('unknownInput');
  assert.throws(
    () => assertLiveRegistryIntegrity(unknownInput),
    /unknown required input/,
  );

  const missingPolicy = cloneRegistry();
  delete missingPolicy[1].invocations[0].canonicalPolicy;
  assert.throws(
    () => assertLiveRegistryIntegrity(missingPolicy),
    /requires a canonical policy/,
  );
});

test('live registry and global catalog reject id-prefix and cross-catalog id conflicts', async () => {
  const prefixConflict = cloneRegistry();
  prefixConflict[1].invocations[0].id = 'live:runtime:fixed-conflict';
  assert.throws(
    () => assertLiveRegistryIntegrity(prefixConflict),
    /task id\/idPrefix conflict/,
  );

  const ordinaryIdConflict = cloneRegistry();
  ordinaryIdConflict[1].invocations[0].id = 'checks:compiler-boundaries';
  await assert.rejects(
    assertVerifyCatalogComplete(root, { liveRegistry: ordinaryIdConflict }),
    /task id\/idPrefix conflict/,
  );
});

test('live selectors cannot collide with public, composite, internal leaf, or builder selectors', async () => {
  assert.ok(ORDINARY_SELECTOR_NAMES.includes('compiler'));
  assert.ok(ORDINARY_SELECTOR_NAMES.includes('checks'));
  assert.ok(ORDINARY_SELECTOR_NAMES.includes('compiler-rust-tests'));

  for (const selector of ['compiler', 'checks', 'compiler-rust-tests']) {
    const conflicting = cloneRegistry();
    conflicting[0].invocations[0].selector = selector;
    await assert.rejects(
      assertVerifyCatalogComplete(root, { liveRegistry: conflicting }),
      new RegExp(`(?:live selector conflicts|duplicate live registry selector).*${escapeRegExp(selector)}`),
    );
    await assert.rejects(
      buildVerifyPlan({
        root,
        selectors: ['verify'],
        liveRegistry: conflicting,
      }),
      new RegExp(`(?:live selector conflicts|duplicate live registry selector).*${escapeRegExp(selector)}`),
    );
  }
});

test('registry-derived self-test injection follows leaf selector drift and rejects composite drift', async () => {
  const leafDrift = cloneRegistry();
  registryInvocation(leafDrift, 'checks-default').selector = 'compiler-boundaries';
  const movedPlan = await buildVerifyPlan({
    root,
    selectors: ['compiler-boundaries'],
    liveRegistry: leafDrift,
  });
  assert.equal(
    movedPlan.tasks.filter((task) => task.id === 'checks:loop-risk-health:self-test').length,
    1,
  );
  const oldLeafPlan = await buildVerifyPlan({
    root,
    selectors: ['checks-default'],
    liveRegistry: leafDrift,
  });
  assert.equal(
    oldLeafPlan.tasks.some((task) => task.id === 'checks:loop-risk-health:self-test'),
    false,
  );

  const compositeDrift = cloneRegistry();
  registryInvocation(compositeDrift, 'checks-default').selector = 'checks';
  await assert.rejects(
    buildVerifyPlan({ root, selectors: ['verify'], liveRegistry: compositeDrift }),
    /registry self-test must target ordinary leaf selector: checks/,
  );
});

test('global catalog counts every discovered checker path exactly once across registries', async () => {
  await assertVerifyCatalogComplete(root);
  const paths = [
    ...CHECKER_REGISTRY.map((entry) => entry.path),
    ...LIVE_REGISTRY
      .filter((entry) => entry.source.type === 'script')
      .map((entry) => entry.source.path),
  ];
  const counts = new Map();
  for (const path of paths) {
    counts.set(path, (counts.get(path) ?? 0) + 1);
  }
  assert.ok([...counts.values()].every((count) => count === 1));
  assert.equal(
    CHECKER_REGISTRY.some((entry) =>
      entry.path === 'scripts/check-db-encrypted-storage-live.mjs'),
    false,
  );
  assert.equal(
    LIVE_REGISTRY.some((entry) =>
      entry.source.path === 'scripts/check-db-encrypted-storage-live.mjs'),
    true,
  );
  for (const path of [
    'scripts/check-loop-risk-health.mjs',
    'scripts/check-loop-risk-stress-live.mjs',
    'scripts/check-router-clean-host-live.mjs',
  ]) {
    assert.equal(CHECKER_REGISTRY.some((entry) => entry.path === path), false);
    assert.equal(LIVE_REGISTRY.some((entry) => entry.source.path === path), true);
  }
  assert.equal(
    paths.includes('scripts/stress-loop-risk-websocket-cancel.mjs'),
    false,
  );

  const duplicatePath = cloneRegistry();
  duplicatePath[1].source.path = CHECKER_REGISTRY[0].path;
  await assert.rejects(
    assertVerifyCatalogComplete(root, { liveRegistry: duplicatePath }),
    /checker path count must be exactly one/,
  );

  const missingScript = cloneRegistry();
  missingScript[1].source.path = 'scripts/check-missing-live-script.mjs';
  await assert.rejects(
    assertVerifyCatalogComplete(root, { liveRegistry: missingScript }),
    /missing registered checker.*check-missing-live-script/,
  );
});

test('registry-derived task metadata is mandatory and matches ownership-tier rules', () => {
  const base = {
    id: 'live:test',
    kind: LIVE_TIERS.LIVE_MANUAL,
    tier: LIVE_TIERS.LIVE_MANUAL,
    ownership: LIVE_OWNERSHIP.EXTERNAL,
    command: 'node',
    args: ['script.mjs'],
    cwd: root,
  };
  assert.doesNotThrow(() => assertPlanIntegrity([base]));
  assert.throws(
    () => assertPlanIntegrity([{ ...base, tier: undefined }]),
    /requires ownership and tier metadata/,
  );
  assert.throws(
    () => assertPlanIntegrity([{ ...base, kind: 'other' }]),
    /kind must match tier/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      tier: LIVE_TIERS.SELF_TEST,
      kind: LIVE_TIERS.SELF_TEST,
    }]),
    /ownership none for tier self-test/,
  );
});

test('runtime invocation blocks each missing executable and never invents DB cleanup tools', async () => {
  const fixture = await runtimeFixture('skiff-live-registry-runtime-path-');
  const required = invocation('runtime-live').value.requiredExecutables;
  try {
    for (const missing of required) {
      const bin = await fakeExecutablePath(
        fixture.root,
        required.filter((candidate) => candidate !== missing),
        `runtime-missing-${missing}`,
      );
      const plan = await buildVerifyPlan({
        root: fixture.root,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...fixture.inputs,
        env: { PATH: bin },
      });
      assert.equal(plan.tasks.length, 1);
      assert.match(plan.tasks[0].preconditionError, new RegExp(`PATH: ${missing}`));
      assert.equal(plan.tasks[0].tier, LIVE_TIERS.LIVE_MANUAL);
      assert.equal(plan.tasks[0].ownership, LIVE_OWNERSHIP.EXTERNAL);
    }

    const bin = await fakeExecutablePath(fixture.root, required, 'runtime-complete');
    const plan = await buildVerifyPlan({
      root: fixture.root,
      catalogRoot: root,
      selectors: ['runtime-live'],
      ...fixture.inputs,
      env: { PATH: bin },
    });
    assert.equal(plan.tasks.length, 1);
    assert.equal(plan.tasks[0].command, 'cargo');
    assert.equal(plan.tasks[0].ownership, LIVE_OWNERSHIP.EXTERNAL);
    assert.doesNotMatch(
      JSON.stringify(invocation('runtime-live').value.requiredExecutables),
      /mongosh|(?:^|\W)sh(?:\W|$)/,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('runtime structural and provided-input errors are never hidden by missing inputs or PATH', async () => {
  const emptyRoot = await mkdtemp(join(tmpdir(), 'skiff-live-registry-empty-discovery-'));
  const fixture = await runtimeFixture('skiff-live-registry-partial-invalid-');
  try {
    await assert.rejects(
      buildVerifyPlan({
        root: emptyRoot,
        catalogRoot: root,
        selectors: ['runtime-live'],
        env: { PATH: '' },
      }),
      /runtime-live found no \*\.live\.test\.skiff fixtures/,
    );

    const cases = [
      {
        input: {
          runtimeLiveArtifactRoot: join(fixture.root, 'missing-artifact-root'),
        },
        expected: /runtime-live artifact root must be an existing directory/,
      },
      {
        input: {
          runtimeLiveActivationUrl: 'https://router.test:4101/secret?token=hidden',
        },
        expected: /must point exactly to \/__skiff\/activate-assembly/,
      },
      {
        input: {
          runtimeLiveEnvironment: 'not canonical/environment',
        },
        expected: /environment must be a canonical ASCII token/,
      },
      {
        input: {
          runtimeLiveExpectedGeneration: '-1',
        },
        expected: /expected generation must be a non-negative integer/,
      },
    ];
    for (const { input, expected } of cases) {
      await assert.rejects(
        buildVerifyPlan({
          root: fixture.root,
          catalogRoot: root,
          selectors: ['runtime-live'],
          env: { PATH: '' },
          ...input,
        }),
        expected,
      );
    }
  } finally {
    await rm(emptyRoot, { recursive: true, force: true });
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('managed DB invocation blocks each exact executable and full PATH generates one command', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-live-registry-db-path-'));
  const required = invocation('db-encrypted-storage-live').value.requiredExecutables;
  try {
    for (const missing of required) {
      const bin = await fakeExecutablePath(
        fixture,
        required.filter((candidate) => candidate !== missing),
        `db-missing-${missing}`,
      );
      const plan = await buildVerifyPlan({
        root,
        selectors: ['db-encrypted-storage-live'],
        env: { PATH: bin },
      });
      assert.equal(plan.tasks.length, 1);
      assert.match(plan.tasks[0].preconditionError, new RegExp(`PATH: ${missing}`));
      assert.equal(plan.tasks[0].tier, LIVE_TIERS.LIVE_MANUAL);
      assert.equal(plan.tasks[0].ownership, LIVE_OWNERSHIP.MANAGED);
    }

    const bin = await fakeExecutablePath(fixture, required, 'db-complete');
    const plan = await buildVerifyPlan({
      root,
      selectors: ['db-encrypted-storage-live'],
      env: { PATH: bin },
    });
    assert.deepEqual(
      plan.tasks.map(({ id, command, args, tier, ownership }) => ({
        id,
        command,
        args,
        tier,
        ownership,
      })),
      [{
        id: 'live:db-encrypted-storage',
        command: 'node',
        args: ['scripts/check-db-encrypted-storage-live.mjs'],
        tier: LIVE_TIERS.LIVE_MANUAL,
        ownership: LIVE_OWNERSHIP.MANAGED,
      }],
    );
    assert.equal(typeof plan.tasks[0].executionPreflight, 'function');
    assert.equal(await plan.tasks[0].executionPreflight(), undefined);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('DB list is blocked and execute starts no command when a declared tool is absent', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-live-registry-db-cli-'));
  const marker = join(fixture, 'fake-node-ran');
  const required = invocation('db-encrypted-storage-live').value.requiredExecutables;
  try {
    const emptyBin = await fakeExecutablePath(fixture, [], 'db-cli-all-missing');
    const allMissing = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'db-encrypted-storage-live', '--list'],
      { cwd: root, env: { ...process.env, PATH: emptyBin } },
    );
    assert.equal(allMissing.code, 0, allMissing.stderr);
    for (const executable of required) {
      assert.match(allMissing.stdout, new RegExp(`\\b${escapeRegExp(executable)}\\b`));
    }

    const bin = await fakeExecutablePath(
      fixture,
      required.filter((candidate) => candidate !== 'mongod'),
      'db-cli',
      { nodeMarker: marker },
    );
    const env = {
      ...process.env,
      PATH: bin,
      SKIFF_FAKE_NODE_MARKER: marker,
    };
    const listed = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'db-encrypted-storage-live', '--list'],
      { cwd: root, env },
    );
    assert.equal(listed.code, 0, listed.stderr);
    assert.match(listed.stdout, /\[blocked: .*PATH: mongod/);

    const executed = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'db-encrypted-storage-live'],
      { cwd: root, env },
    );
    assert.notEqual(executed.code, 0);
    assert.match(
      `${executed.stdout}\n${executed.stderr}`,
      /live:db-encrypted-storage: blocked: .*PATH: mongod/,
    );
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('actual canonical runtime discovery composes with the managed live task', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-live-registry-actual-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    const executables = new Set([
      ...invocation('runtime-live').value.requiredExecutables,
      ...invocation('db-encrypted-storage-live').value.requiredExecutables,
    ]);
    const bin = await fakeExecutablePath(fixture, [...executables], 'combined');
    const options = {
      root,
      selectors: ['runtime-live', 'db-encrypted-storage-live'],
      runtimeLiveActivationUrl:
        'http://router.test:4101/__skiff/activate-assembly',
      runtimeLiveIngressUrl: 'http://router.test:4100',
      runtimeLiveArtifactRoot: artifactRoot,
      runtimeLiveEnvironment: 'runtime-live',
      runtimeLiveExpectedGeneration: '0',
      env: { PATH: bin },
    };
    const plan = await buildVerifyPlan(options);
    assert.equal(plan.tasks.length, 5);
    assert.deepEqual(
      plan.tasks.filter((task) => task.id.startsWith('live:runtime:'))
        .map((task) => task.args[task.args.indexOf('--expected-generation') + 1]),
      ['0', '1', '2', '3'],
    );
    assert.equal(
      plan.tasks.filter((task) => task.id === 'live:db-encrypted-storage').length,
      1,
    );
    const deduplicated = await buildVerifyPlan({
      ...options,
      selectors: ['runtime-live', 'runtime-live'],
    });
    assert.deepEqual(
      deduplicated.tasks.map((task) => task.id),
      plan.tasks.slice(0, 4).map((task) => task.id),
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('loop-risk selectors consume one canonical config path without expanding target contents', async () => {
  const fixture = await loopRiskConfigFixture();
  try {
    for (const selector of ['loop-risk-health-live', 'loop-risk-stress-live']) {
      const missing = await buildVerifyPlan({
        root,
        selectors: [selector],
        env: { ...process.env, SKIFF_LOOP_RISK_CONFIG: '' },
      });
      assert.equal(missing.tasks.length, 1);
      assert.match(missing.tasks[0].preconditionError, /loop-risk config/);
    }

    const plans = await Promise.all([
      buildVerifyPlan({ root, selectors: ['loop-risk-health-live'], loopRiskConfig: fixture.configPath }),
      buildVerifyPlan({ root, selectors: ['loop-risk-stress-live'], loopRiskConfig: fixture.configPath }),
      buildVerifyPlan({
        root,
        selectors: ['loop-risk-health-live'],
        env: { ...process.env, SKIFF_LOOP_RISK_CONFIG: fixture.configPath },
      }),
    ]);
    assert.deepEqual(plans[0].tasks[0].args, [
      'scripts/check-loop-risk-health.mjs',
      '--config',
      fixture.configPath,
    ]);
    assert.deepEqual(plans[1].tasks[0].args, [
      'scripts/check-loop-risk-stress-live.mjs',
      '--config',
      fixture.configPath,
    ]);
    assert.deepEqual(plans[2].tasks[0].args, plans[0].tasks[0].args);
    assert.equal(await plans[1].tasks[0].executionPreflight(), undefined);
    for (const plan of plans) {
      const rendered = JSON.stringify(plan.tasks.map(({ executionPreflight, ...task }) => task));
      assert.doesNotMatch(rendered, /registry-target-secret|service-token/);
    }

    const listed = await runProcess(process.execPath, [
      verifyPath,
      '--only',
      'loop-risk-health-live,loop-risk-stress-live',
      '--loop-risk-config',
      fixture.configPath,
      '--list',
    ], { cwd: root });
    assert.equal(listed.code, 0, listed.stderr);
    assert.doesNotMatch(listed.stdout, /registry-target-secret|service-token/);
    assert.equal((listed.stdout.match(/--loop-risk-config/g) ?? []).length, 0);
    assert.equal((listed.stdout.match(/--config/g) ?? []).length, 2);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('loop-risk registry derives exact executable and module prerequisites', async () => {
  const fixture = await loopRiskConfigFixture();
  try {
    for (const [selector, required] of [
      ['loop-risk-health-live', ['node']],
      ['loop-risk-stress-live', ['node', 'ps']],
    ]) {
      for (const missing of required) {
        const bin = await fakeExecutablePath(
          fixture.root,
          required.filter((candidate) => candidate !== missing),
          `${selector}-${missing}`,
        );
        const plan = await buildVerifyPlan({
          root,
          selectors: [selector],
          loopRiskConfig: fixture.configPath,
          env: { PATH: bin },
        });
        assert.match(plan.tasks[0].preconditionError, new RegExp(`PATH: ${missing}`));
      }
    }

    const isolatedRoot = join(fixture.root, 'module-missing-root');
    await mkdir(join(isolatedRoot, 'scripts'), { recursive: true });
    await writeFile(join(isolatedRoot, 'scripts', 'check-loop-risk-stress-live.mjs'), '');
    await writeFile(join(isolatedRoot, 'scripts', 'package.json'), '{}\n');
    const bin = await fakeExecutablePath(fixture.root, ['node', 'ps'], 'module-missing');
    const moduleMissing = await buildVerifyPlan({
      root: isolatedRoot,
      catalogRoot: root,
      selectors: ['loop-risk-stress-live'],
      loopRiskConfig: fixture.configPath,
      env: { PATH: bin },
    });
    assert.match(moduleMissing.tasks[0].preconditionError, /ws from scripts\/package.json/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('loop-risk stress execution preflight fails only its task without stopping neighbors', async () => {
  const fixture = await loopRiskConfigFixture({ pid: 2_147_483_647 });
  const marker = join(fixture.root, 'command-ran');
  try {
    const plan = await buildVerifyPlan({
      root,
      selectors: ['loop-risk-stress-live'],
      loopRiskConfig: fixture.configPath,
    });
    await unlink(fixture.logPath);
    const markerTask = (id) => ({
      id,
      kind: 'test',
      command: process.execPath,
      args: ['--eval', 'require("node:fs").writeFileSync(process.argv[1], "ran")', marker],
      cwd: fixture.root,
    });
    const summary = await runVerifyPlan({
      selectors: ['test'],
      tasks: [markerTask('before'), ...plan.tasks, markerTask('after')],
    }, fixture.root);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'before', status: 'passed' },
        { id: 'live:loop-risk-stress', status: 'failed' },
        { id: 'after', status: 'passed' },
      ],
    );
    const failed = summary.results[1];
    assert.match(failed.reason, /runtime log must be an existing readable file/);
    assert.match(failed.reason, /runtime PID is not alive/);
    await access(marker);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('default verify never includes registry live/manual invocations', async () => {
  const plan = await buildVerifyPlan({ root });
  assert.equal(
    plan.tasks.some((task) => task.tier === LIVE_TIERS.LIVE_MANUAL),
    false,
  );
  assert.equal(
    plan.tasks.some((task) => LIVE_SELECTORS.includes(task.id)),
    false,
  );
  assert.equal(
    plan.tasks.filter((task) => task.id === 'checks:loop-risk-health:self-test').length,
    1,
  );
});

test('checks plus health live keeps self-test and network invocation unique', async () => {
  const fixture = await loopRiskConfigFixture();
  try {
    const plan = await buildVerifyPlan({
      root,
      selectors: ['checks', 'loop-risk-health-live', 'checks'],
      loopRiskConfig: fixture.configPath,
    });
    assert.equal(
      plan.tasks.filter((task) => task.id === 'checks:loop-risk-health:self-test').length,
      1,
    );
    assert.equal(
      plan.tasks.filter((task) => task.id === 'live:loop-risk-health').length,
      1,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('blocked live prerequisites never stop marker tasks before or after the task', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-live-registry-blocked-plan-'));
  const marker = join(fixture, 'command-ran');
  try {
    const bin = await fakeExecutablePath(fixture, ['node'], 'blocked-plan');
    const blocked = await buildVerifyPlan({
      root,
      selectors: ['db-encrypted-storage-live'],
      env: { PATH: bin },
    });
    const markerTask = (id) => ({
      id,
      kind: 'test',
      command: process.execPath,
      args: [
        '--eval',
        'require("node:fs").writeFileSync(process.argv[1], "ran")',
        marker,
      ],
      cwd: fixture,
    });
    const summary = await runVerifyPlan({
      selectors: ['test'],
      tasks: [
        markerTask('earlier-runs'),
        ...blocked.tasks,
        markerTask('later-runs'),
      ],
    }, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'earlier-runs', status: 'passed' },
        { id: 'live:db-encrypted-storage', status: 'blocked' },
        { id: 'later-runs', status: 'passed' },
      ],
    );
    assert.match(summary.results[1].reason, /PATH: .*mongod/);
    await access(marker);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

function invocation(selector) {
  for (const entry of LIVE_REGISTRY) {
    const value = entry.invocations.find((candidate) => candidate.selector === selector);
    if (value !== undefined) {
      return { entry, value };
    }
  }
  throw new Error(`missing test invocation ${selector}`);
}

function registryInvocation(registry, selector) {
  for (const entry of registry) {
    const value = entry.invocations.find((candidate) => candidate.selector === selector);
    if (value !== undefined) {
      return value;
    }
  }
  throw new Error(`missing registry invocation ${selector}`);
}

function cloneRegistry() {
  return structuredClone(LIVE_REGISTRY);
}

async function runtimeFixture(prefix) {
  const fixtureRoot = await mkdtemp(join(tmpdir(), prefix));
  const artifactRoot = join(fixtureRoot, 'artifacts');
  const packageRoot = join(fixtureRoot, 'runtime', 'live-tests');
  const testFile = join(packageRoot, 'example.live.test.skiff');
  await mkdir(dirname(testFile), { recursive: true });
  await mkdir(artifactRoot);
  await writeFile(
    join(packageRoot, 'package.yml'),
    'id: example.com/runtime-live-fixture\nversion: 1.0.0\n',
  );
  await writeFile(
    join(packageRoot, 'config.skiff-test.yml'),
    '"example.com/runtime-live-fixture": {}\n',
  );
  await writeFile(testFile, 'test defaultRun false\n');
  return {
    root: fixtureRoot,
    inputs: {
      runtimeLiveActivationUrl:
        'http://router.test:4101/__skiff/activate-assembly',
      runtimeLiveIngressUrl: 'http://router.test:4100',
      runtimeLiveArtifactRoot: artifactRoot,
      runtimeLiveEnvironment: 'runtime-live',
      runtimeLiveExpectedGeneration: '0',
    },
  };
}

async function loopRiskConfigFixture({ pid = process.pid } = {}) {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-risk-registry-'));
  const configPath = join(fixtureRoot, 'loop-risk.json');
  const logPath = join(fixtureRoot, 'runtime.log');
  await writeFile(logPath, '');
  await writeFile(configPath, JSON.stringify({
    healthUrl:
      'http://registry.test:4101/__router/health?detail=loop-risk',
    runtimeIds: ['registry-runtime'],
    stress: {
      wsUrl: 'ws://registry.test:4101/registry-target-secret?token=service-token',
      runtimePids: [pid],
      runtimeLogs: [logPath],
    },
  }));
  return { root: fixtureRoot, configPath, logPath };
}

async function fakeExecutablePath(
  fixture,
  executables,
  label,
  { nodeMarker } = {},
) {
  const bin = join(fixture, `bin-${label}`);
  await mkdir(bin, { recursive: true });
  for (const executable of executables) {
    const path = join(bin, executable);
    const contents = executable === 'node' && nodeMarker !== undefined
      ? [
        '#!/bin/sh',
        'printf ran > "$SKIFF_FAKE_NODE_MARKER"',
        '',
      ].join('\n')
      : '#!/bin/sh\nexit 0\n';
    await writeFile(path, contents);
    await chmod(path, 0o755);
  }
  return bin;
}

function runProcess(command, args, { cwd, env = process.env }) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd, env });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
