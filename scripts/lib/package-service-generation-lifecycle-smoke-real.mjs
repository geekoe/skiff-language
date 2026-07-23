import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { loadRouterWebSocket } from './loop-risk-stress-node.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import {
  FIXTURE_CARGO_DIAGNOSTIC_PROPERTY,
  retainFixtureCargoDiagnostic,
} from './package-service-ecosystem-smoke-diagnostic.mjs';
import {
  validatePackageServiceActivationReceipt,
  validatePackageServiceBootstrapReceipt,
} from './package-service-ecosystem-smoke-oracle.mjs';
import {
  closePackageServiceSmokeWebSocket,
  createPackageServiceSmokeDeadline,
  nextPackageServiceSmokeWebSocketMessage,
  openPackageServiceSmokeWebSocket,
  packageServiceEcosystemSmokeFixtureCargoArgs,
} from './package-service-ecosystem-smoke-real.mjs';
import {
  readPackageServiceGenerationFixtureReceipt,
  validatePackageServiceGenerationFixturePair,
  validatePackageServiceGenerationUnaryResponse,
  waitForPackageServiceGenerationState,
} from './package-service-generation-lifecycle-smoke-oracle.mjs';

const FIXTURE_RELATIVE_ROOTS = Object.freeze({
  A: join('test-runner', 'fixtures', 'package-service-websocket-generation-a'),
  B: join('test-runner', 'fixtures', 'package-service-websocket-generation-b'),
});
const EXPECTED_MARKERS = Object.freeze({
  A: 'P5-R05-GENERATION-A-MARKER',
  B: 'P5-R05-GENERATION-B-MARKER',
});
const DEFAULT_TRANSCRIPT_TIMEOUT_MS = 120_000;
export const GENERATION_LIFECYCLE_FIXTURE_DIAGNOSTIC_PROPERTY =
  'generationLifecycleFixtureCargoDiagnostic';

export async function runPackageServiceGenerationLifecycleSmoke({
  checkout,
  replicaCount,
  environment,
}, dependencies = {}) {
  assert.equal(
    replicaCount,
    1,
    'the R05 generation lifecycle transcript owns exactly one runtime replica',
  );
  const runtimeOwner = dependencies.runtimeOwner ?? runInIsolatedTestRuntime;
  const runCommand = dependencies.runCommand ?? captureCheckedCommand;
  const activate = dependencies.activate ?? requestAssemblyActivation;
  const loadWebSocket = dependencies.loadWebSocket ?? (() =>
    loadRouterWebSocket(
      new URL('../run-package-service-generation-lifecycle-smoke.mjs', import.meta.url),
    ));
  const requestUnary = dependencies.requestUnary ?? requestGenerationUnary;

  return runtimeOwner({
    skiffRoot: checkout,
    environment,
    validateBootstrapReceipt: (receipt) =>
      validatePackageServiceBootstrapReceipt(receipt, environment),
    runTest: async (isolatedEnv, signal, stack) => {
      const lifecycle = createPackageServiceSmokeDeadline(
        signal,
        dependencies.transcriptTimeoutMs ?? DEFAULT_TRANSCRIPT_TIMEOUT_MS,
      );
      let clientA;
      let clientB;
      let WebSocket;
      let primaryError;
      let result;
      try {
        const receiptA = await lifecycle.wait((ioSignal) => authorCandidate({
          candidate: 'A',
          checkout,
          environment,
          isolatedEnv,
          stack,
          signal: ioSignal,
          runCommand,
        }));
        const activationA = await activateCandidate({
          activate,
          lifecycle,
          stack,
          environment,
          receipt: receiptA,
          expectedGeneration: 0,
        });
        await waitForState({
          dependencies,
          lifecycle,
          stack,
          environment,
          generation: activationA.activeAssembly.generation,
          assemblyIdentity: receiptA.candidate.assembly.assemblyIdentity,
          connectionPinCount: 0,
          inFlightCount: 0,
        });

        const websocketA = receiptA.candidate.entrypoints[2];
        WebSocket = await loadWebSocket();
        clientA = createWebSocket(WebSocket, stack.routerHttpUrl, websocketA);
        await openPackageServiceSmokeWebSocket(clientA, WebSocket, lifecycle.signal);

        const receiptB = await lifecycle.wait((ioSignal) => authorCandidate({
          candidate: 'B',
          checkout,
          environment,
          isolatedEnv,
          stack,
          signal: ioSignal,
          runCommand,
        }));
        const candidates = validatePackageServiceGenerationFixturePair(receiptA, receiptB);
        const activationB = await activateCandidate({
          activate,
          lifecycle,
          stack,
          environment,
          receipt: receiptB,
          expectedGeneration: 1,
        });
        await waitForState({
          dependencies,
          lifecycle,
          stack,
          environment,
          generation: activationB.activeAssembly.generation,
          assemblyIdentity: receiptB.candidate.assembly.assemblyIdentity,
          connectionPinCount: 1,
          inFlightCount: 0,
        });

        await sendAndExpect(clientA, EXPECTED_MARKERS.A, lifecycle.signal);
        await sendAndExpect(clientA, EXPECTED_MARKERS.A, lifecycle.signal);

        const websocketB = receiptB.candidate.entrypoints[2];
        clientB = createWebSocket(WebSocket, stack.routerHttpUrl, websocketB);
        await openPackageServiceSmokeWebSocket(clientB, WebSocket, lifecycle.signal);
        await waitForState({
          dependencies,
          lifecycle,
          stack,
          environment,
          generation: activationB.activeAssembly.generation,
          assemblyIdentity: receiptB.candidate.assembly.assemblyIdentity,
          connectionPinCount: 2,
          inFlightCount: 0,
        });
        await sendAndExpect(clientB, EXPECTED_MARKERS.B, lifecycle.signal);

        const unary = receiptB.candidate.entrypoints[1];
        const unaryResponse = await lifecycle.wait((ioSignal) => requestUnary({
          url: `${stack.routerHttpUrl}${unary.path}`,
          host: unary.host,
          signal: ioSignal,
        }));
        validatePackageServiceGenerationUnaryResponse(
          unaryResponse,
          EXPECTED_MARKERS.B,
        );

        await closePackageServiceSmokeWebSocket(
          clientB,
          WebSocket,
          lifecycle.signal,
        );
        clientB = undefined;
        await waitForState({
          dependencies,
          lifecycle,
          stack,
          environment,
          generation: activationB.activeAssembly.generation,
          assemblyIdentity: receiptB.candidate.assembly.assemblyIdentity,
          connectionPinCount: 1,
          inFlightCount: 0,
        });

        await closePackageServiceSmokeWebSocket(
          clientA,
          WebSocket,
          lifecycle.signal,
        );
        clientA = undefined;
        await waitForState({
          dependencies,
          lifecycle,
          stack,
          environment,
          generation: activationB.activeAssembly.generation,
          assemblyIdentity: receiptB.candidate.assembly.assemblyIdentity,
          connectionPinCount: 0,
          inFlightCount: 0,
        });

        result = {
          status: 'PASS',
          probe: 'r05-generation-lifecycle',
          replicas: 1,
          generations: {
            A: 1,
            B: 2,
          },
          candidates,
          markers: EXPECTED_MARKERS,
          sourceFixtures: FIXTURE_RELATIVE_ROOTS,
          productionPath: [
            'compiler',
            'canonicalStore',
            'assemblyActivation',
            'routerWebSocketGateway',
            'runtimeGenerationPin',
            'unaryGenerationB',
            'generationDrain',
          ],
        };
      } catch (error) {
        primaryError = error;
      } finally {
        for (const client of [clientB, clientA]) {
          if (client === undefined) continue;
          try {
            await closePackageServiceSmokeWebSocket(
              client,
              WebSocket,
              lifecycle.signal,
            );
          } catch (error) {
            primaryError ??= error;
          }
        }
        lifecycle.dispose();
      }
      if (primaryError !== undefined) throw primaryError;
      return result;
    },
  });
}

export function packageServiceGenerationFixtureRoot(checkout, candidate) {
  assert.ok(candidate === 'A' || candidate === 'B', 'candidate must be A or B');
  return resolve(checkout, FIXTURE_RELATIVE_ROOTS[candidate]);
}

async function authorCandidate({
  candidate,
  checkout,
  environment,
  isolatedEnv,
  stack,
  signal,
  runCommand,
}) {
  const fixtureRoot = packageServiceGenerationFixtureRoot(checkout, candidate);
  let outcome;
  try {
    outcome = await runCommand(
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
    throw retainGenerationLifecycleFixtureCargoDiagnostic(error, candidate);
  }
  return readPackageServiceGenerationFixtureReceipt(outcome.stdout, environment);
}

async function activateCandidate({
  activate,
  lifecycle,
  stack,
  environment,
  receipt,
  expectedGeneration,
}) {
  const activation = await lifecycle.wait((ioSignal) => activate({
    activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
    expectedGeneration,
    environment,
    assembly: receipt.candidate.assembly,
    signal: ioSignal,
  }));
  return validatePackageServiceActivationReceipt(activation, {
    environment,
    assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
    expectedGeneration,
  });
}

function waitForState({
  dependencies,
  lifecycle,
  stack,
  environment,
  generation,
  assemblyIdentity,
  connectionPinCount,
  inFlightCount,
}) {
  return lifecycle.wait((ioSignal) => waitForPackageServiceGenerationState({
    healthUrl: `${stack.controlUrl}/__router/health`,
    environment,
    generation,
    assemblyIdentity,
    connectionPinCount,
    inFlightCount,
    signal: ioSignal,
    readHealth: dependencies.readHealth,
    now: dependencies.readinessNow,
    sleep: dependencies.readinessSleep,
    timeoutMs: dependencies.readinessTimeoutMs,
    intervalMs: dependencies.readinessIntervalMs,
  }));
}

function createWebSocket(WebSocket, routerHttpUrl, entrypoint) {
  return new WebSocket(
    `${routerHttpUrl.replace(/^http:/, 'ws:')}${entrypoint.path}`,
    { headers: { Host: entrypoint.host } },
  );
}

async function sendAndExpect(client, expectedMarker, signal) {
  const message = nextPackageServiceSmokeWebSocketMessage(client, signal);
  try {
    client.send('r05-generation-lifecycle-probe');
    assert.equal(await message, expectedMarker);
  } catch (error) {
    message.catch(() => {});
    throw error;
  }
}

async function requestGenerationUnary({ url, host, signal }) {
  const response = await fetch(url, {
    method: 'POST',
    headers: { Host: host },
    signal,
  });
  return {
    status: response.status,
    body: await response.text(),
  };
}

function retainGenerationLifecycleFixtureCargoDiagnostic(error, candidate) {
  retainFixtureCargoDiagnostic(error);
  if ((typeof error !== 'object' && typeof error !== 'function') || error === null) {
    return error;
  }
  if (!Object.hasOwn(error, GENERATION_LIFECYCLE_FIXTURE_DIAGNOSTIC_PROPERTY)) {
    Object.defineProperty(error, GENERATION_LIFECYCLE_FIXTURE_DIAGNOSTIC_PROPERTY, {
      value: Object.freeze({
        candidate,
        ...error[FIXTURE_CARGO_DIAGNOSTIC_PROPERTY],
      }),
      enumerable: true,
      writable: false,
      configurable: false,
    });
  }
  return error;
}

export const packageServiceGenerationLifecycleExpectedMarkers = EXPECTED_MARKERS;
export const packageServiceGenerationLifecycleFixtureRelativeRoots =
  FIXTURE_RELATIVE_ROOTS;
