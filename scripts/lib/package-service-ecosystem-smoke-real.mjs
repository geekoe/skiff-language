import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { loadRouterWebSocket } from './loop-risk-stress-node.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import { retainFixtureCargoDiagnostic } from './package-service-ecosystem-smoke-diagnostic.mjs';
import {
  readPackageServiceFixtureReceipt,
  validatePackageServiceActivationReceipt,
  validatePackageServiceBootstrapReceipt,
  waitForPackageServiceAssemblyReady,
} from './package-service-ecosystem-smoke-oracle.mjs';

const FIXTURE_RELATIVE_ROOT = join(
  'test-runner',
  'fixtures',
  'package-service-websocket-smoke',
);
const EXPECTED_MARKER = 'P5-F23D-REAL-COMPONENT-MARKER';
const DEFAULT_IO_TIMEOUT_MS = 60_000;

export async function runPackageServiceEcosystemSmoke({
  checkout,
  replicaCount,
  environment,
}, dependencies = {}) {
  assert.equal(
    replicaCount,
    1,
    'the F23D production-component smoke owns one committed generation and one runtime replica',
  );
  const runtimeOwner = dependencies.runtimeOwner ?? runInIsolatedTestRuntime;
  const runCommand = dependencies.runCommand ?? captureCheckedCommand;
  const activate = dependencies.activate ?? requestAssemblyActivation;
  const readHealth = dependencies.readHealth;
  const loadWebSocket = dependencies.loadWebSocket ?? (() =>
    loadRouterWebSocket(new URL('../run-package-service-ecosystem-smoke.mjs', import.meta.url)));

  return runtimeOwner({
    skiffRoot: checkout,
    environment,
    validateBootstrapReceipt: (receipt) =>
      validatePackageServiceBootstrapReceipt(receipt, environment),
    runTest: async (isolatedEnv, signal, stack) => {
      const fixtureRoot = packageServiceEcosystemSmokeFixtureRoot(checkout);
      let fixtureOutcome;
      try {
        fixtureOutcome = await runCommand(
          'cargo',
          packageServiceEcosystemSmokeFixtureCargoArgs({
            checkout,
            fixtureRoot,
            artifactRoot: stack.artifactRoot,
            environment,
          }),
          { cwd: checkout, env: isolatedEnv, signal },
        );
      } catch (error) {
        throw retainFixtureCargoDiagnostic(error);
      }
      const receipt = readPackageServiceFixtureReceipt(fixtureOutcome.stdout, environment);
      const lifecycle = createSmokeIoLifecycle(signal, dependencies.ioTimeoutMs);
      let client;
      let WebSocket;
      let failed = false;
      let primaryError;
      let result;
      try {
        const activation = await lifecycle.wait((ioSignal) => activate({
          activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
          expectedGeneration: 0,
          environment,
          assembly: receipt.candidate.assembly,
          signal: ioSignal,
        }));
        const activationResponse = validatePackageServiceActivationReceipt(activation, {
          environment,
          assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
        });
        await waitForPackageServiceAssemblyReady({
          healthUrl: `${stack.controlUrl}/__router/health`,
          environment,
          generation: activationResponse.activeAssembly.generation,
          assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
          signal: lifecycle.signal,
          readHealth,
          now: dependencies.readinessNow,
          sleep: dependencies.readinessSleep,
          timeoutMs: dependencies.readinessTimeoutMs,
          intervalMs: dependencies.readinessIntervalMs,
        });
        const websocket = receipt.candidate.entrypoints[2];
        WebSocket = await loadWebSocket();
        client = new WebSocket(
          `${stack.routerHttpUrl.replace(/^http:/, 'ws:')}${websocket.path}`,
          { headers: { Host: websocket.host } },
        );
        await opened(client, WebSocket, lifecycle.signal);
        const marker = nextMessage(client, lifecycle.signal);
        try {
          client.send('production-component-probe');
          assert.equal(await marker, EXPECTED_MARKER);
        } catch (error) {
          marker.catch(() => {});
          throw error;
        }

        result = {
          status: 'PASS',
          probe: 'skiff-cutover-production-websocket-component',
          replicas: 1,
          generation: activationResponse.activeAssembly.generation,
          assembly: receipt.candidate.assembly.assemblyIdentity,
          sourceFixture: FIXTURE_RELATIVE_ROOT,
          productionPath: [
            'compiler',
            'deployment',
            'runtimeAssembly',
            'routerRegistry',
            'runtimeDispatcher',
            'runtimeProtocolPeer',
            'clientMarker',
          ],
          websocket: {
            host: websocket.host,
            path: websocket.path,
            operation: websocket.operation,
            marker: EXPECTED_MARKER,
          },
        };
      } catch (error) {
        failed = true;
        primaryError = error;
      } finally {
        if (client !== undefined) {
          try {
            await closeWebSocket(client, WebSocket, lifecycle.signal);
          } catch (error) {
            if (!failed) {
              failed = true;
              primaryError = error;
            }
          }
        }
        lifecycle.dispose();
      }
      if (failed) throw primaryError;
      return result;
    },
  });
}

export function packageServiceEcosystemSmokeFixtureRoot(checkout) {
  return resolve(checkout, FIXTURE_RELATIVE_ROOT);
}

export function packageServiceEcosystemSmokeFixtureCargoArgs({
  checkout,
  fixtureRoot,
  artifactRoot,
  environment,
}) {
  return [
    'run',
    '--quiet',
    '--locked',
    '--manifest-path',
    join(checkout, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    fixtureRoot,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    checkout,
    '--environment',
    environment,
  ];
}

async function opened(client, WebSocket, signal) {
  if (client.readyState === WebSocket.OPEN) return;
  signal.throwIfAborted();
  await new Promise((resolvePromise, reject) => {
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      client.off('open', onOpen);
      client.off('error', onError);
      client.off('unexpected-response', onUnexpectedResponse);
      signal.removeEventListener('abort', onAbort);
      operation();
    };
    const onOpen = () => finish(resolvePromise);
    const onError = (error) => finish(() => reject(error));
    const onAbort = () => finish(() => reject(signal.reason));
    const onUnexpectedResponse = (_request, response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8').trim();
        finish(() => reject(new Error(
          `production WebSocket upgrade failed with ${response.statusCode}${body === '' ? '' : `: ${body}`}`,
        )));
      });
    };
    client.once('open', onOpen);
    client.once('error', onError);
    client.once('unexpected-response', onUnexpectedResponse);
    signal.addEventListener('abort', onAbort, { once: true });
    if (signal.aborted) onAbort();
  });
}

async function closeWebSocket(client, WebSocket, signal) {
  if (client.readyState === WebSocket.CLOSED) return;
  const ignoreExpectedCleanupError = () => {};
  client.on('error', ignoreExpectedCleanupError);
  let retainErrorGuard = false;
  const retainErrorGuardUntilClose = () => {
    if (retainErrorGuard) return;
    retainErrorGuard = true;
    client.once('close', () => client.off('error', ignoreExpectedCleanupError));
  };
  let terminateRequested = false;
  const terminate = () => {
    if (terminateRequested || client.readyState === WebSocket.CLOSED) return undefined;
    terminateRequested = true;
    retainErrorGuardUntilClose();
    try {
      client.terminate();
      return undefined;
    } catch (error) {
      return error;
    }
  };
  try {
    await new Promise((resolvePromise, reject) => {
      let settled = false;
      const cleanup = () => {
        client.off('close', onClose);
        signal.removeEventListener('abort', onAbort);
      };
      const finish = (operation) => {
        if (settled) return;
        settled = true;
        cleanup();
        operation();
      };
      const onClose = () => finish(resolvePromise);
      const onAbort = () => {
        if (settled) return;
        settled = true;
        cleanup();
        const terminateError = terminate();
        reject(signal.reason ?? terminateError);
      };
      client.once('close', onClose);
      signal.addEventListener('abort', onAbort, { once: true });
      if (signal.aborted) {
        onAbort();
        return;
      }
      try {
        if (client.readyState === WebSocket.CONNECTING) {
          const terminateError = terminate();
          if (terminateError !== undefined) finish(() => reject(terminateError));
        } else {
          client.close();
        }
      } catch (error) {
        terminate();
        finish(() => reject(error));
      }
    });
  } finally {
    if (!retainErrorGuard) client.off('error', ignoreExpectedCleanupError);
  }
}

function nextMessage(client, signal) {
  signal.throwIfAborted();
  return new Promise((resolvePromise, reject) => {
    const cleanup = () => {
      client.off('message', onMessage);
      signal.removeEventListener('abort', onAbort);
    };
    const onMessage = (data) => {
      cleanup();
      resolvePromise(String(data));
    };
    const onAbort = () => {
      cleanup();
      reject(signal.reason);
    };
    client.once('message', onMessage);
    signal.addEventListener('abort', onAbort, { once: true });
    if (signal.aborted) onAbort();
  });
}

function createSmokeIoLifecycle(parentSignal, timeoutMs = DEFAULT_IO_TIMEOUT_MS) {
  assert.ok(
    Number.isSafeInteger(timeoutMs) && timeoutMs >= 0,
    'ecosystem smoke I/O timeout must be a non-negative safe integer',
  );
  const controller = new AbortController();
  const abortFromParent = () => controller.abort(parentSignal.reason);
  if (parentSignal.aborted) abortFromParent();
  else {
    parentSignal.addEventListener('abort', abortFromParent, { once: true });
    if (parentSignal.aborted) abortFromParent();
  }
  const timeout = setTimeout(
    () => controller.abort(new SmokeIoDeadlineError(timeoutMs)),
    timeoutMs,
  );
  return {
    signal: controller.signal,
    wait: (operation) => waitForAbortableOperation(operation, controller.signal),
    dispose: () => {
      clearTimeout(timeout);
      parentSignal.removeEventListener('abort', abortFromParent);
    },
  };
}

async function waitForAbortableOperation(operation, signal) {
  signal.throwIfAborted();
  let onAbort;
  const aborted = new Promise((_resolve, reject) => {
    onAbort = () => reject(signal.reason);
    signal.addEventListener('abort', onAbort, { once: true });
    if (signal.aborted) onAbort();
  });
  try {
    return await Promise.race([
      Promise.resolve().then(() => operation(signal)),
      aborted,
    ]);
  } finally {
    signal.removeEventListener('abort', onAbort);
  }
}

class SmokeIoDeadlineError extends Error {
  constructor(timeoutMs) {
    super(`ecosystem smoke I/O deadline expired after ${timeoutMs}ms`);
    this.name = 'SmokeIoDeadlineError';
  }
}

export const packageServiceEcosystemSmokeExpectedMarker = EXPECTED_MARKER;
export {
  opened as openPackageServiceSmokeWebSocket,
  nextMessage as nextPackageServiceSmokeWebSocketMessage,
  closeWebSocket as closePackageServiceSmokeWebSocket,
  createSmokeIoLifecycle as createPackageServiceSmokeDeadline,
};
