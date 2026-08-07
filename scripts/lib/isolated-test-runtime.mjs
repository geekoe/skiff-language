import { spawn as spawnAdditionalRuntimeChild } from 'node:child_process';
import { mkdir, mkdtemp, open, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import {
  isolatedInstanceOperations,
  isolatedTestRunnerEnvironment,
} from './isolated-test-runtime-instance.mjs';
import { assertPortsClosed, leaseConsecutiveLocalPorts } from './local-port-lease.mjs';
import {
  claimIsolatedTestWorkspace,
  removeOwnedIsolatedTestWorkspace,
} from './isolated-test-runtime-workspace.mjs';
import {
  ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY,
  retainIsolatedRuntimeLogEvidence,
} from './isolated-test-runtime-log-evidence.mjs';
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
  profile = 'skiff-test',
  signalTarget = process,
  validateBootstrapReceipt,
  runtimeReplicas = 1,
  dependencies = {},
  ensureRuntimeBinaries = ensureRuntimeStackDebugBinaries,
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
  const effectiveEnsureBinaries = Object.keys(dependencies ?? {}).length === 0
    ? ensureRuntimeBinaries
    : async () => {};
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
      profile,
      ops,
      signal: abortController.signal,
      validateBootstrapReceipt,
      runtimeReplicas,
      ensureRuntimeBinaries: effectiveEnsureBinaries,
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
  profile,
  ops,
  signal,
  validateBootstrapReceipt,
  runtimeReplicas,
  ensureRuntimeBinaries,
}) {
  const portLease = await ops.leasePorts();
  let tempRoot;
  let ownershipReceipt;
  const children = [];
  const additionalRuntimes = [];
  const additionalRuntimeLogFiles = [];
  let spawnAttempted = false;
  try {
    tempRoot = await ops.makeTempRoot();
    ownershipReceipt = await ops.claimWorkspace(tempRoot);
    const sourceArtifactRoot = join(tempRoot, 'source-artifacts');
    await ops.createSourceArtifactRoot(sourceArtifactRoot);
    const instanceRoot = join(tempRoot, 'instance');
    const devHome = join(instanceRoot, 'dev-home');
    const artifactRoot = join(devHome, 'artifacts');
    const basePort = portLease.ports[0];
    const controlPort = basePort + 1;
    const mongoPort = portLease.ports[3];
    const routerBinary = join(skiffRoot, 'build', 'bin', 'skiff-router');
    const runtimeBinary = join(skiffRoot, 'build', 'bin', 'runtime');
    await ensureRuntimeBinaries({ routerBinary, runtimeBinary });
    await ops.initializeInstance({
      profile,
      devHome,
      basePort,
      mongoPort,
      ownershipReceipt,
    });
    const isolatedEnv = isolatedTestRunnerEnvironment({
      baseEnv,
      skiffRoot,
      cargoTarget,
      devHome,
      controlPort,
      routerHttpPort: basePort,
      profile,
    });
    signal.throwIfAborted();
    spawnAttempted = true;
    const mongoChild = await stageCall('Mongo spawn', () =>
      ops.spawnMongo({
        mongoBinary: 'mongod',
        mongoPort,
        mongoDataDir: join(devHome, 'mongo-data'),
        cwd: instanceRoot,
        env: isolatedEnv,
        logDir: join(devHome, 'logs'),
      }));
    children.push(mongoChild);
    await stageCall('Mongo primary election', () =>
      ops.waitMongoPrimary({ mongoPort, child: mongoChild, signal }));
    const bootstrap = await stageCall('bootstrap seed', async () => {
      const receipt = await ops.seedBootstrap({
        skiffRoot,
        artifactRoot,
        profile,
        env: isolatedEnv,
        signal,
      });
      validateBootstrapReceipt?.(receipt);
      return receipt;
    });
    const controlUrl = `http://127.0.0.1:${controlPort}`;
    const routerHttpUrl = `http://127.0.0.1:${basePort}`;
    const routerChild = await stageCall('Router spawn', () =>
      ops.spawnRouter({
        routerBinary,
        routerConfigPath: join(devHome, 'router.yml'),
        cwd: instanceRoot,
        env: isolatedEnv,
        logDir: join(devHome, 'logs'),
      }));
    children.push(routerChild);
    const runtimeChild = await stageCall('Runtime spawn', () =>
      ops.spawnRuntime({
        runtimeBinary,
        runtimeConfigPath: join(devHome, 'runtime.yml'),
        cwd: instanceRoot,
        env: isolatedEnv,
        logDir: join(devHome, 'logs'),
      }));
    children.push(runtimeChild);
    await stageCall('Router/Runtime readiness', () => ops.waitReady({
      controlUrl,
      routerHttpUrl,
      mongoPort,
      artifactRoot,
      bootstrap,
      children,
      signal,
    }));
    for (let replica = 1; replica < runtimeReplicas; replica += 1) {
      const runtimeHome = join(tempRoot, `runtime-${replica + 1}-home`);
      const runtimeConfig = join(tempRoot, `runtime-${replica + 1}.yml`);
      await mkdir(runtimeHome, { recursive: true });
      const logsDir = join(instanceRoot, 'logs');
      await mkdir(logsDir, { recursive: true });
      const stdoutLogPath = join(logsDir, `runtime-${replica + 1}.log`);
      const stderrLogPath = join(logsDir, `runtime-${replica + 1}.err.log`);
      const stdoutLog = await open(stdoutLogPath, 'w');
      const stderrLog = await open(stderrLogPath, 'w');
      additionalRuntimeLogFiles.push({
        stdoutLogPath,
        stderrLogPath,
        stdoutLog,
        stderrLog,
      });
      await writeFile(runtimeConfig, renderRuntimeConfig({
        routerUrl: `ws://127.0.0.1:${controlPort}/runtime`,
        runtimeHome,
      }), { encoding: 'utf8', flag: 'wx', mode: 0o600 });
      // child-process-owner: isolated-additional-runtime
      const child = spawnAdditionalRuntimeChild(
        runtimeBinary,
        [runtimeConfig],
        {
          cwd: skiffRoot,
          env: isolatedEnv,
          stdio: ['ignore', stdoutLog.fd, stderrLog.fd],
        },
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
      controlUrl,
      routerHttpUrl,
      devHome,
      portLease,
      ports: portLease.ports,
      children,
      additionalRuntimes,
      additionalRuntimeLogFiles,
      tempRoot,
      profile,
      ownershipReceipt,
      testRunnerEnv: isolatedEnv,
    };
  } catch (error) {
    const partial = {
      ownershipReceipt,
      portLease,
      ports: portLease.ports,
      children,
      additionalRuntimes,
      additionalRuntimeLogFiles,
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

async function ensureRuntimeStackDebugBinaries({ routerBinary, runtimeBinary }) {
  const { access } = await import('node:fs/promises');
  const missing = [];
  for (const file of [routerBinary, runtimeBinary]) {
    try {
      await access(file);
    } catch {
      missing.push(file);
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `debug binaries missing (${missing.join(', ')}); run "skiff build router runtime" first`,
    );
  }
}

async function cleanupIsolatedTestRuntime(stack, ops, testError) {
  const errors = [];
  for (const child of stack.additionalRuntimes ?? []) {
    await settleCleanupStep(errors, `stop additional Runtime ${child.pid}`, () =>
      stopAdditionalRuntime(child));
  }
  for (const logFile of stack.additionalRuntimeLogFiles ?? []) {
    await settleCleanupStep(
      errors,
      `close additional Runtime log ${logFile.stdoutLogPath}`,
      async () => {
        await logFile.stdoutLog.close();
        await logFile.stderrLog.close();
      }
    );
  }
  if ((stack.children ?? []).length > 0) {
    await settleCleanupStep(errors, 'stop isolated stack', () => ops.stopProcesses(stack.children));
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
