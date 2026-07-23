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
      const activation = await activate({
        activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
        expectedGeneration: 0,
        environment,
        assembly: receipt.candidate.assembly,
      });
      const activationResponse = validatePackageServiceActivationReceipt(activation, {
        environment,
        assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
      });
      await waitForPackageServiceAssemblyReady({
        healthUrl: `${stack.controlUrl}/__router/health`,
        environment,
        generation: activationResponse.activeAssembly.generation,
        assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
        signal,
        readHealth,
        now: dependencies.readinessNow,
        sleep: dependencies.readinessSleep,
        timeoutMs: dependencies.readinessTimeoutMs,
        intervalMs: dependencies.readinessIntervalMs,
      });
      const websocket = receipt.candidate.entrypoints[2];
      const WebSocket = await loadWebSocket();
      const client = new WebSocket(
        `${stack.routerHttpUrl.replace(/^http:/, 'ws:')}${websocket.path}`,
        { headers: { Host: websocket.host } },
      );
      try {
        await opened(client, WebSocket);
        const marker = nextMessage(client);
        client.send('production-component-probe');
        assert.equal(await marker, EXPECTED_MARKER);
      } finally {
        await closeWebSocket(client, WebSocket);
      }

      return {
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

async function opened(client, WebSocket) {
  if (client.readyState === WebSocket.OPEN) return;
  await new Promise((resolvePromise, reject) => {
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      client.off('open', onOpen);
      client.off('error', onError);
      client.off('unexpected-response', onUnexpectedResponse);
      operation();
    };
    const onOpen = () => finish(resolvePromise);
    const onError = (error) => finish(() => reject(error));
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
  });
}

async function closeWebSocket(client, WebSocket) {
  if (client.readyState === WebSocket.CLOSED) return;
  const closed = new Promise((resolvePromise) => client.once('close', resolvePromise));
  const ignoreExpectedCleanupError = () => {};
  client.once('error', ignoreExpectedCleanupError);
  if (client.readyState === WebSocket.CONNECTING) client.terminate();
  else client.close();
  await closed;
  client.off('error', ignoreExpectedCleanupError);
}

function nextMessage(client) {
  return new Promise((resolvePromise, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('timed out waiting for the production WebSocket marker')),
      10_000,
    );
    client.once('message', (data) => {
      clearTimeout(timeout);
      resolvePromise(String(data));
    });
  });
}

export const packageServiceEcosystemSmokeExpectedMarker = EXPECTED_MARKER;
