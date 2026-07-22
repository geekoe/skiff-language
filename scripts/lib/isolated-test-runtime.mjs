import { mkdir, mkdtemp, rm } from 'node:fs/promises';
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
    });
    value = await runTest(stack.testRunnerEnv, abortController.signal, stack);
  } catch (error) {
    testError = error;
  }

  let cleanupError;
  if (stack !== undefined) {
    try {
      await cleanupIsolatedTestRuntime(stack, ops);
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
    throw new Error(
      `${errorMessage(testError)}; isolated runtime cleanup failed: ${errorMessage(cleanupError)}`,
      { cause: new AggregateError([testError, cleanupError]) },
    );
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
}) {
  const portLease = await ops.leasePorts();
  let tempRoot;
  let supervisor;
  let configWritten = false;
  let supervisorAttempted = false;
  try {
    tempRoot = await ops.makeTempRoot();
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
    await ops.writeConfig(configPath, config);
    configWritten = true;
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
      testRunnerEnv: isolatedEnv,
    };
  } catch (error) {
    const partial = {
      configPath: configWritten && supervisorAttempted
        ? join(tempRoot, 'instance', 'config.yml')
        : undefined,
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

async function cleanupIsolatedTestRuntime(stack, ops) {
  const errors = [];
  if (stack.supervisor !== undefined) {
    try {
      await ops.stopSupervisor(stack.supervisor);
    } catch (error) {
      errors.push(cleanupStepError('stop supervisor', error));
    }
  }
  if (stack.configPath !== undefined) {
    try {
      await ops.stopOwnedInstance(stack.configPath);
    } catch (error) {
      errors.push(cleanupStepError('stop owned instance', error));
    }
  }
  if (stack.configPath !== undefined) {
    try {
      await ops.verifyInstanceStopped(stack.configPath);
    } catch (error) {
      errors.push(cleanupStepError('verify instance stopped', error));
    }
  }
  try {
    await ops.assertPortsClosed(stack.ports);
  } catch (error) {
    errors.push(cleanupStepError('verify ports closed', error));
  }
  try {
    await stack.portLease.release();
  } catch (error) {
    errors.push(cleanupStepError('release port lease', error));
  }
  if (errors.length === 0 && stack.tempRoot !== undefined) {
    try {
      await ops.removeTempRoot(stack.tempRoot);
    } catch (error) {
      errors.push(cleanupStepError('remove temp workspace', error));
    }
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
    createSourceArtifactRoot: (path) => mkdir(path, { recursive: true }),
    assertPortsClosed,
    removeTempRoot: (path) => rm(path, { recursive: true, force: true }),
    ...overrides,
  };
}

function errorMessage(error) {
  return error?.message || String(error);
}

function cleanupStepError(step, error) {
  return new Error(`${step}: ${errorMessage(error)}`, { cause: error });
}

export const isolatedTestRuntimeConstants = {
  portMin: ISOLATED_PORT_MIN,
  portMax: ISOLATED_PORT_MAX,
  mongoPort: isolatedTestInstanceConstants.mongoPort,
};
