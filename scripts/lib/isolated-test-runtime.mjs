import { mkdir, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import {
  isolatedInstanceOperations,
  isolatedTestInstanceConfigText,
  isolatedTestInstanceConstants,
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
  dependencies = {},
}) {
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
}) {
  const portLease = await ops.leasePorts();
  let tempRoot;
  let ownershipReceipt;
  let supervisor;
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
    const basePort = portLease.ports[0];
    const controlPort = basePort + 1;
    const config = isolatedTestInstanceConfigText({
      devHome,
      cargoTarget,
      basePort,
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
    const bootstrap = await ops.seedBootstrap({
      skiffRoot,
      artifactRoot,
      environment,
      env: isolatedEnv,
      signal,
    });
    validateBootstrapReceipt?.(bootstrap);
    signal.throwIfAborted();
    supervisorAttempted = true;
    supervisor = ops.spawnSupervisor({ skiffRoot, configPath, env: isolatedEnv });
    const controlUrl = `http://127.0.0.1:${controlPort}`;
    const routerHttpUrl = `http://127.0.0.1:${basePort}`;
    await ops.waitReady({
      controlUrl,
      routerHttpUrl,
      artifactRoot,
      bootstrap,
      supervisor,
      signal,
    });
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
      tempRoot,
    };
    try {
      await cleanupIsolatedTestRuntime(partial, ops);
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

function isolatedRuntimeOperations(overrides, skiffRoot, baseEnv) {
  return {
    ...isolatedInstanceOperations({ skiffRoot, baseEnv }),
    leasePorts: () => leaseConsecutiveLocalPorts({
      rangeStart: ISOLATED_PORT_MIN,
      rangeEnd: ISOLATED_PORT_MAX,
      count: 3,
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
  mongoPort: isolatedTestInstanceConstants.mongoPort,
};
