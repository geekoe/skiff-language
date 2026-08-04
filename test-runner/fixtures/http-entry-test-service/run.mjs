import { access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  captureAttachedCommand,
  captureCheckedCommand,
} from '../../../scripts/lib/command-execution.mjs';
import { runInIsolatedTestRuntime } from '../../../scripts/lib/isolated-test-runtime.mjs';
import { assertPortsClosed } from '../../../scripts/lib/local-port-lease.mjs';

const skiffRoot = requiredAbsolutePath('SKIFF_HTTP_ENTRY_PROBE_ROOT');
const testRunner = requiredAbsolutePath('SKIFF_HTTP_ENTRY_PROBE_TEST_RUNNER');
const bootstrap = requiredAbsolutePath('SKIFF_HTTP_ENTRY_PROBE_BOOTSTRAP');
const fixtureRoot = resolve(skiffRoot, 'test-runner/fixtures/http-entry-test-service');

const cleanup = await runInIsolatedTestRuntime({
  skiffRoot,
  dependencies: {
    seedBootstrap: async ({ artifactRoot, profile, env, signal }) => {
      signal.throwIfAborted();
      const result = await captureCheckedCommand(
        bootstrap,
        seedCommittedArgs(artifactRoot, profile),
        { cwd: skiffRoot, env },
      );
      signal.throwIfAborted();
      return JSON.parse(result.stdout);
    },
  },
  runTest: async (isolatedEnv, signal, stack) => {
    assertLoopbackStack(stack, isolatedEnv);
    await captureCheckedCommand(
      bootstrap,
      bootstrapArgs(stack.sourceArtifactRoot, stack.profile),
      { cwd: skiffRoot, env: isolatedEnv },
    );

    const rejected = await captureAttachedCommand(
      testRunner,
      testRunnerArgs(
        join(fixtureRoot, 'active'),
        stack.sourceArtifactRoot,
      ),
      {
        cwd: skiffRoot,
        env: {
          ...isolatedEnv,
          SKIFF_TEST_EXPECTED_GENERATION: '0',
        },
      },
    );
    const rejectionOutput = `${rejected.stdout}\n${rejected.stderr}`;
    if (
      rejected.code !== 1
      || rejected.signal !== null
      || !rejectionOutput.includes('already has an active self-ingress')
    ) {
      throw new Error(
        `second active self-ingress did not fail with the canonical rejection:\n${rejectionOutput}`,
      );
    }
    console.log(`EXPECTED_CONCURRENCY_REJECTION\n${rejectionOutput.trim()}`);
    await assertAssemblyReady(stack.controlUrl, 'expected-rejection');

    signal.throwIfAborted();
    const happy = await captureAttachedCommand(
      testRunner,
      testRunnerArgs(
        join(fixtureRoot, 'happy'),
        stack.sourceArtifactRoot,
      ),
      {
        cwd: skiffRoot,
        env: {
          ...isolatedEnv,
          SKIFF_TEST_EXPECTED_GENERATION: '1',
        },
      },
    );
    if (happy.code !== 0 || happy.signal !== null) {
      throw new Error(
        `happy HTTP entry fixture failed:\n${happy.stdout}\n${happy.stderr}`,
      );
    }
    if (!happy.stdout.includes('test result: ok. 5 passed; 0 failed')) {
      throw new Error(`happy HTTP entry fixture omitted exact pass count:\n${happy.stdout}`);
    }
    console.log(`HAPPY_HTTP_ENTRY_PASS\n${happy.stdout.trim()}`);
    await assertAssemblyReady(stack.controlUrl, 'happy');

    return {
      ports: [...stack.ports],
      tempRoot: stack.tempRoot,
    };
  },
});

await assertPortsClosed(cleanup.ports);
await assertMissing(cleanup.tempRoot, 'isolated workspace');
for (const port of cleanup.ports) {
  await assertMissing(
    join(tmpdir(), 'skiff-local-port-leases', `${port}.lock`),
    `port lease ${port}`,
  );
}
console.log(JSON.stringify({
  event: 'ISOLATED_CLEANUP_PASS',
  ports: cleanup.ports,
  workspaceRemoved: cleanup.tempRoot,
}));

function bootstrapArgs(artifactRoot, profile) {
  return [
    '--bootstrap-only',
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    skiffRoot,
    '--profile',
    profile,
  ];
}

function seedCommittedArgs(artifactRoot, profile) {
  return [
    '--seed-committed',
    join(fixtureRoot, 'active'),
    '--artifact-root',
    artifactRoot,
    '--profile',
    profile,
    '--platform-source-root',
    skiffRoot,
  ];
}

function testRunnerArgs(root, artifactRoot) {
  return [
    root,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    skiffRoot,
    '--deny-skips',
    '--require-tests',
  ];
}

async function assertAssemblyReady(controlUrl, label) {
  const response = await fetch(`${controlUrl}/__router/health`);
  if (!response.ok) {
    throw new Error(`${label} Router health returned ${response.status}`);
  }
  const health = await response.json();
  if (
    health?.ok !== true
    || health.pendingActivation !== null
    || !Array.isArray(health.capabilityConnections)
    || health.capabilityConnections.length === 0
    || health.capabilityConnections.some((connection) => connection?.connected !== true)
  ) {
    throw new Error(
      `${label} Router/Runtime assembly was not ready after the case: ${JSON.stringify(health)}`,
    );
  }
  console.log(`ASSEMBLY_READY ${label} ${JSON.stringify(health)}`);
}

function assertLoopbackStack(stack, isolatedEnv) {
  for (const [label, value] of [
    ['control URL', stack.controlUrl],
    ['business ingress URL', stack.routerHttpUrl],
    ['runner activation URL', isolatedEnv.SKIFF_TEST_ACTIVATION_URL],
    ['runner ingress URL', isolatedEnv.SKIFF_TEST_INGRESS_URL],
  ]) {
    if (typeof value !== 'string' || !value.startsWith('http://127.0.0.1:')) {
      throw new Error(`${label} must be an isolated loopback URL`);
    }
  }
  if (
    stack.ports.some((port) => !Number.isInteger(port) || port < 46000 || port > 46999)
  ) {
    throw new Error(`isolated stack used ports outside 46000-46999: ${stack.ports.join(',')}`);
  }
}

async function assertMissing(path, label) {
  try {
    await access(path);
  } catch (error) {
    if (error?.code === 'ENOENT') return;
    throw error;
  }
  throw new Error(`${label} was not cleaned up: ${path}`);
}

function requiredAbsolutePath(name) {
  const value = requiredText(name);
  const absolute = resolve(value);
  if (absolute !== value) {
    throw new Error(`${name} must be absolute`);
  }
  return absolute;
}

function requiredText(name) {
  const value = process.env[name];
  if (typeof value !== 'string' || value.length === 0 || value.trim() !== value) {
    throw new Error(`${name} must be a non-empty trimmed string`);
  }
  return value;
}
