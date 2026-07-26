import { spawn as spawnAdditionalRuntimeChild } from 'node:child_process';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import {
  isolatedInstanceOperations,
  isolatedTestInstanceConfigText,
  isolatedTestRunnerEnvironment,
} from './isolated-test-runtime-instance.mjs';
import { assertPortsClosed, leaseConsecutiveLocalPorts } from './local-port-lease.mjs';
import {
  captureIsolatedTestConfig,
  claimIsolatedTestWorkspace,
  removeOwnedIsolatedTestWorkspace,
} from './isolated-test-runtime-workspace.mjs';
import {
  ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY,
  retainIsolatedRuntimeLogEvidence,
} from './isolated-test-runtime-log-evidence.mjs';
import { runtimeBinaryName } from './dev-runtime-paths.mjs';
import { renderRuntimeConfig } from './runtime-stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');
const ISOLATED_PORT_MIN = 46000;
const ISOLATED_PORT_MAX = 46999;

export function shouldUseIsolatedTestRuntime(live) {
  return !live;
}

export async function runInIsolatedTestRuntime({
  runTest,
  skiffRoot = defaultSkiffRoot,
  baseEnv = process.env,
  environment = 'skiff-test',
  signalTarget = process,
  validateBootstrapReceipt,
  runtimeReplicas = 1,
  dependencies = {},
}) {
  if (!Number.isSafeInteger(runtimeReplicas) || runtimeReplicas < 1 || runtimeReplicas > 2) {
    throw new Error('isolated test runtimeReplicas must be 1 or 2');
  }
  const absoluteSkiffRoot = resolve(skiffRoot);
  const absoluteCargoTarget = cargoTargetDir(absoluteSkiffRoot, baseEnv);
  const isolatedBaseEnv = {
    ...baseEnv,
    CARGO_TARGET_DIR: absoluteCargoTarget,
    SKIFF_TEST_PLATFORM_SOURCE_ROOT: absoluteSkiffRoot,
  };
  const ops = isolatedRuntimeOperations(dependencies, absoluteSkiffRoot, isolatedBaseEnv);
  const abortController = new AbortController();
  let stack;
  let interruptedBy;
  const signalHandlers = new Map(
    ['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
      interruptedBy ??= signal;
      abortController.abort(new Error(`skiff test interrupted by ${signal}`));
    }]),
  );
  for (const [signal, handler] of signalHandlers) {
    signalTarget.on(signal, handler);
  }

  let value;
  let testError;
  try {
    stack = await startIsolatedTestRuntime({
      skiffRoot: absoluteSkiffRoot,
      cargoTarget: absoluteCargoTarget,
      baseEnv: isolatedBaseEnv,
      environment,
      ops,
      signal: abortController.signal,
      validateBootstrapReceipt,
      runtimeReplicas,
    });
    value = await runTest(stack.testRunnerEnv, abortController.signal, stack);
  } catch (error) {
    testError = error;
  }

  let cleanupError;
  if (stack !== undefined) {
    try {
      await cleanupIsolatedTestRuntime(stack, ops, testError);
    } catch (error) {
      cleanupError = error;
    }
  }
  for (const [signal, handler] of signalHandlers) {
    signalTarget.off(signal, handler);
  }
  if (interruptedBy !== undefined && testError === undefined) {
    testError = new Error(`skiff test interrupted by ${interruptedBy}`);
  }
  if (testError !== undefined && cleanupError !== undefined) {
    const combinedError = new Error(
      `${errorMessage(testError)}; isolated runtime cleanup failed: ${errorMessage(cleanupError)}`,
      { cause: new AggregateError([testError, cleanupError]) },
    );
    if (Object.hasOwn(testError, ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY)) {
      Object.defineProperty(combinedError, ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY, {
        value: testError[ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY],
        enumerable: true,
        writable: false,
        configurable: false,
      });
    }
    throw combinedError;
  }
  if (testError !== undefined) {
    throw testError;
  }
  if (cleanupError !== undefined) {
    throw cleanupError;
  }
  return value;
}

async function startIsolatedTestRuntime({
  skiffRoot,
  cargoTarget,
  baseEnv,
  environment,
  ops,
  signal,
  validateBootstrapReceipt,
  runtimeReplicas,
}) {
  const portLease = await ops.leasePorts();
  let tempRoot;
  let ownershipReceipt;
  let supervisor;
  let additionalRuntimes = [];
  let configOwnershipRequired = false;
  let supervisorAttempted = false;
  try {
    tempRoot = await ops.makeTempRoot();
    ownershipReceipt = await ops.claimWorkspace(tempRoot);
    const sourceArtifactRoot = join(tempRoot, 'source-artifacts');
    await ops.createSourceArtifactRoot(sourceArtifactRoot);
    const instanceRoot = join(tempRoot, 'instance');
    const configPath = join(instanceRoot, 'config.yml');
    const devHome = join(instanceRoot, 'dev-home');
    const artifactRoot = join(devHome, 'artifacts');
    const startupGate = join(instanceRoot, 'activation-seeded.ready');
    const startupReady = join(instanceRoot, 'mongo-started.ready');
    const basePort = portLease.ports[0];
    const controlPort = basePort + 1;
    const mongoPort = portLease.ports[3];
    const config = isolatedTestInstanceConfigText({
      devHome,
      cargoTarget,
      basePort,
      mongoPort,
      environment,
    });
    configOwnershipRequired = true;
    await ops.writeConfig(configPath, config, ownershipReceipt);
    ownershipReceipt = await ops.captureConfigOwnership(ownershipReceipt, configPath);
    const isolatedEnv = isolatedTestRunnerEnvironment({
      baseEnv,
      skiffRoot,
      cargoTarget,
      devHome,
      controlPort,
      routerHttpPort: basePort,
      environment,
    });
    signal.throwIfAborted();
    supervisorAttempted = true;
    supervisor = await stageCall('Mongo spawn', () =>
      ops.spawnSupervisor({
        skiffRoot,
        configPath,
        startupGate,
        startupReady,
        env: isolatedEnv,
      }));
    await stageCall('Mongo spawn', () =>
      ops.waitMongoStarted({ startupReady, supervisor, signal }));
    await stageCall('Mongo primary election', () =>
      ops.waitMongoPrimary({ mongoPort, supervisor, signal }));
    const bootstrap = await stageCall('activation seed', async () => {
      const receipt = await ops.seedBootstrap({
        skiffRoot,
        artifactRoot,
        environment,
        env: isolatedEnv,
        signal,
      });
      validateBootstrapReceipt?.(receipt);
      await ops.seedActivationState({ mongoPort, bootstrap: receipt, signal });
      await ops.releaseStartupGate(startupGate, ownershipReceipt);
      return receipt;
    });
    const controlUrl = `http://127.0.0.1:${controlPort}`;
    const routerHttpUrl = `http://127.0.0.1:${basePort}`;
    await stageCall('Router/Runtime readiness', () => ops.waitReady({
      controlUrl,
      routerHttpUrl,
      mongoPort,
      artifactRoot,
      bootstrap,
      supervisor,
      signal,
    }));
    for (let replica = 1; replica < runtimeReplicas; replica += 1) {
      const runtimeHome = join(tempRoot, `runtime-${replica + 1}-home`);
      const runtimeConfig = join(tempRoot, `runtime-${replica + 1}.yml`);
      await mkdir(runtimeHome, { recursive: true });
      await writeFile(runtimeConfig, renderRuntimeConfig({
        routerUrl: `ws://127.0.0.1:${controlPort}/runtime`,
        runtimeHome,
        environment,
      }), { encoding: 'utf8', flag: 'wx', mode: 0o600 });
      // child-process-owner: isolated-additional-runtime
      const child = spawnAdditionalRuntimeChild(
        join(devHome, 'bin', runtimeBinaryName()),
        [runtimeConfig],
        { cwd: skiffRoot, env: isolatedEnv, stdio: 'inherit' },
      );
      additionalRuntimes.push(child);
    }
    if (additionalRuntimes.length > 0) {
      await waitForRuntimeReplicaCount({
        controlUrl,
        expected: runtimeReplicas,
        children: additionalRuntimes,
        signal,
      });
    }
    console.log(`[skiff-test] isolated runtime control: ${controlUrl}`);
    console.log(`[skiff-test] isolated runtime workspace: ${tempRoot}`);
    return {
      artifactRoot,
      sourceArtifactRoot,
      configPath,
      controlUrl,
      routerHttpUrl,
      devHome,
      portLease,
      ports: portLease.ports,
      supervisor,
      additionalRuntimes,
      tempRoot,
      environment,
      instanceOwnership: ownershipReceipt,
      ownershipReceipt,
      testRunnerEnv: isolatedEnv,
    };
  } catch (error) {
    const partial = {
      instanceOwnership: supervisorAttempted ? ownershipReceipt : undefined,
      ownershipReceipt,
      configOwnershipRequired,
      portLease,
      ports: portLease.ports,
      supervisor,
      additionalRuntimes,
      tempRoot,
    };
    try {
      await cleanupIsolatedTestRuntime(partial, ops, error);
    } catch (cleanupError) {
      throw new Error(
        `${errorMessage(error)}; isolated runtime startup cleanup failed: ${errorMessage(cleanupError)}`,
        { cause: new AggregateError([error, cleanupError]) },
      );
    }
    throw error;
  }
}

async function cleanupIsolatedTestRuntime(stack, ops, testError) {
  const errors = [];
  for (const child of stack.additionalRuntimes ?? []) {
    await settleCleanupStep(errors, `stop additional Runtime ${child.pid}`, () =>
      stopAdditionalRuntime(child));
  }
  if (stack.supervisor !== undefined) {
    await settleCleanupStep(errors, 'stop supervisor', async () => {
      const stopped = await ops.stopSupervisor(stack.supervisor);
      if (stopped?.stopped === false) {
        throw new Error('isolated runtime supervisor reported stopped:false');
      }
    });
  }
  if (stack.instanceOwnership !== undefined) {
    await settleCleanupStep(
      errors,
      'stop owned instance',
      () => ops.stopOwnedInstance(stack.instanceOwnership),
    );
  }
  if (stack.instanceOwnership !== undefined) {
    await settleCleanupStep(
      errors,
      'verify instance stopped',
      () => ops.verifyInstanceStopped(stack.instanceOwnership),
    );
  }
  if (testError !== undefined && stack.tempRoot !== undefined) {
    await retainIsolatedRuntimeLogEvidence(testError, stack.tempRoot, {
      read: ops.readFailureLog,
    });
  }
  await settleCleanupStep(errors, 'verify ports closed', () => ops.assertPortsClosed(stack.ports));
  await settleCleanupStep(errors, 'release port lease', () => stack.portLease.release());
  if (stack.tempRoot !== undefined && stack.ownershipReceipt === undefined) {
    errors.push(cleanupStepError(
      'preserve unowned temp workspace',
      new Error(`isolated workspace ownership receipt was not established for ${stack.tempRoot}`),
    ));
  }
  if (stack.configOwnershipRequired && stack.ownershipReceipt?.config === undefined) {
    errors.push(cleanupStepError(
      'preserve workspace with uncaptured config',
      new Error(`instance config ownership was not captured for ${stack.tempRoot}`),
    ));
  }
  if (errors.length === 0 && stack.ownershipReceipt !== undefined) {
    await settleCleanupStep(
      errors,
      'remove temp workspace',
      () => ops.removeOwnedWorkspace(stack.ownershipReceipt),
    );
  }
  if (errors.length > 0) {
    const evidence = stack.tempRoot === undefined
      ? ''
      : `; preserving isolated runtime workspace ${stack.tempRoot}`;
    const details = errors.map(errorMessage).join('; ');
    throw new AggregateError(errors, `isolated runtime cleanup failed: ${details}${evidence}`);
  }
}

async function waitForRuntimeReplicaCount({
  controlUrl,
  expected,
  children,
  signal,
}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < 120_000) {
    signal.throwIfAborted();
    const exited = children.find(
      (child) => child.exitCode !== null || child.signalCode !== null,
    );
    if (exited !== undefined) {
      throw new Error(
        `additional Runtime ${exited.pid} exited before readiness with ${
          exited.signalCode ?? exited.exitCode
        }`,
      );
    }
    try {
      const response = await fetch(`${controlUrl}/__router/health`, { signal });
      if (response.ok) {
        const health = await response.json();
        const replicas = (health.replicas ?? []).filter(
          (replica) => replica?.connected === true && replica?.state === 'healthy',
        );
        const connections = (health.capabilityConnections ?? []).filter(
          (connection) => connection?.connected === true,
        );
        if (
          new Set(replicas.map((replica) => replica.replicaId)).size >= expected
          && new Set(connections.map((connection) => connection.runtimeId)).size >= expected
        ) {
          return;
        }
      }
    } catch {
      // The Router health endpoint may be between assembly transitions.
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
  }
  throw new Error(`isolated runtime did not reach ${expected} healthy replicas`);
}

async function stopAdditionalRuntime(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolvePromise, reject) => {
    child.once('error', reject);
    child.once('exit', resolvePromise);
  });
  child.kill('SIGTERM');
  const stopped = await Promise.race([
    exited.then(() => true),
    new Promise((resolvePromise) => setTimeout(() => resolvePromise(false), 20_000)),
  ]);
  if (stopped) return;
  child.kill('SIGKILL');
  await exited;
}

function isolatedRuntimeOperations(overrides, skiffRoot, baseEnv) {
  return {
    ...isolatedInstanceOperations({ skiffRoot, baseEnv }),
    leasePorts: () => leaseConsecutiveLocalPorts({
      rangeStart: ISOLATED_PORT_MIN,
      rangeEnd: ISOLATED_PORT_MAX,
      count: 4,
    }),
    makeTempRoot: () => mkdtemp(join(tmpdir(), 'skiff-test-runtime-')),
    claimWorkspace: claimIsolatedTestWorkspace,
    createSourceArtifactRoot: (path) => mkdir(path, { recursive: true }),
    captureConfigOwnership: captureIsolatedTestConfig,
    assertPortsClosed,
    removeOwnedWorkspace: removeOwnedIsolatedTestWorkspace,
    readFailureLog: undefined,
    ...overrides,
  };
}

function errorMessage(error) {
  return error?.message || String(error);
}

async function stageCall(stage, operation) {
  try {
    return await operation();
  } catch (error) {
    throw new Error(`isolated ${stage} failed: ${errorMessage(error)}`, { cause: error });
  }
}

function cleanupStepError(step, error) {
  return new Error(`${step}: ${errorMessage(error)}`, { cause: error });
}

async function settleCleanupStep(errors, step, operation) {
  try {
    await operation();
  } catch (error) {
    errors.push(cleanupStepError(step, error));
  }
}

export const isolatedTestRuntimeConstants = {
  portMin: ISOLATED_PORT_MIN,
  portMax: ISOLATED_PORT_MAX,
};
