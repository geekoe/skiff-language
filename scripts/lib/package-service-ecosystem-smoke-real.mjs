import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { loadRouterWebSocket } from './loop-risk-stress-node.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import { retainFixtureCargoDiagnostic } from './package-service-ecosystem-smoke-diagnostic.mjs';

const FIXTURE_RELATIVE_ROOT = join(
  'test-runner',
  'fixtures',
  'package-service-websocket-smoke',
);
const EXPECTED_MARKER = 'P5-F23D-REAL-COMPONENT-MARKER';
const FIXTURE_SCHEMA_VERSION = 'skiff-package-service-smoke-fixture-v1';

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
  const loadWebSocket = dependencies.loadWebSocket ?? (() =>
    loadRouterWebSocket(new URL('../run-package-service-ecosystem-smoke.mjs', import.meta.url)));

  return runtimeOwner({
    skiffRoot: checkout,
    environment,
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
      const receipt = readFixtureReceipt(fixtureOutcome.stdout, environment);
      const activation = await activate({
        activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
        expectedGeneration: 0,
        environment,
        assembly: receipt.candidate.assembly,
      });
      assert.equal(activation.response?.ok, true, 'production Router must commit the fixture');
      assert.equal(
        activation.response?.activeAssembly?.assemblyIdentity,
        receipt.candidate.assembly.assemblyIdentity,
        'production Router must expose the exact compiled assembly',
      );
      const websocket = requiredEntrypoint(receipt, 'websocket');
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
        generation: activation.response.activeAssembly.generation,
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

function readFixtureReceipt(stdout, expectedEnvironment) {
  let receipt;
  try {
    receipt = JSON.parse(stdout);
  } catch (error) {
    throw new Error(`ecosystem smoke fixture returned invalid JSON: ${error.message}`);
  }
  assert.equal(receipt.schemaVersion, FIXTURE_SCHEMA_VERSION);
  assert.equal(receipt.environment, expectedEnvironment);
  assert.match(
    receipt.candidate?.assembly?.assemblyIdentity ?? '',
    /^skiff-runtime-assembly-v1:sha256:[a-f0-9]{64}$/,
  );
  requiredEntrypoint(receipt, 'websocket');
  return receipt;
}

function requiredEntrypoint(receipt, kind) {
  const entrypoint = receipt.candidate?.entrypoints?.find((entry) => entry.kind === kind);
  assert.ok(entrypoint, `ecosystem smoke fixture must publish a ${kind} entrypoint`);
  assert.equal(typeof entrypoint.host, 'string');
  assert.equal(typeof entrypoint.path, 'string');
  assert.match(
    entrypoint.operation ?? '',
    /^skiff-contract-operation-v1:sha256:[a-f0-9]{64}$/,
  );
  return entrypoint;
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
