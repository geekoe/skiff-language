import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';

import { readPackageServiceFixtureReceipt } from '../lib/package-service-ecosystem-smoke-oracle.mjs';
import {
  smokeFixtureIdentities,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';

const ENVIRONMENT = 'p5-f387-http-fixture';

test('router compiler fixture uses split HTTP authoring without an empty business config profile', async () => {
  const root = join('compiler', 'tests', 'fixtures', 'router-websocket-fixture');
  const [service, http] = await Promise.all([
    readFile(join(root, 'service.yml'), 'utf8'),
    readFile(join(root, 'http.yml'), 'utf8'),
  ]);
  assert.equal(service, 'id: example.com/websocket_fixture\n');
  assert.equal(
    http,
    `ping:
  method: GET
  path: /ping
  kind: typedJson
  handler: main.__skiffHttpPing
  adapterArgs:
    - param: body
      source: { kind: http.body }
`,
  );
  await assert.rejects(
    access(join(root, 'config.dev.yml')),
    { code: 'ENOENT' },
    'router fixture must not carry an empty platform-policy profile',
  );
});

test('owned WebSocket source fixtures use private current connect authoring', async () => {
  for (const [name, serviceId, api, functionNames] of [
    [
      'package-service-websocket-smoke',
      'test.skiff/package-service-websocket-smoke',
      'marker: main.marker\n',
      ['marker', '__skiffHttpProbe', 'websocketConnect'],
    ],
    [
      'package-service-websocket-generation-a',
      'test.skiff/package-service-websocket-smoke',
      'marker: main.marker\n',
      ['marker', '__skiffHttpProbe', 'websocketConnect'],
    ],
    [
      'package-service-websocket-generation-b',
      'test.skiff/package-service-websocket-smoke',
      'marker: main.marker\n',
      ['marker', '__skiffHttpProbe', 'websocketConnect'],
    ],
    [
      'package-service-i02-spawn-submit',
      'test.skiff/package-service-i02-spawn-submit',
      'marker: main.submitSpawnReceipt\n',
      [
        'submittedReceiptSource',
        'nestedSubmittedReceipt',
        'firstSubmittedReceipt',
        'acceptSubmittedReceipt',
        'submitSpawnReceipt',
        '__skiffHttpProbe',
        'websocketConnect',
      ],
    ],
  ]) {
    const root = join('test-runner', 'fixtures', name);
    const [actualApi, service, http, websocket, source] = await Promise.all([
      readFile(join(root, 'api.yml'), 'utf8'),
      readFile(join(root, 'service.yml'), 'utf8'),
      readFile(join(root, 'http.yml'), 'utf8'),
      readFile(join(root, 'websocket.yml'), 'utf8'),
      readFile(join(root, 'main.skiff'), 'utf8'),
    ]);
    assert.equal(actualApi, api, `${name} API`);
    assert.equal(
      service,
      `id: ${serviceId}
kind: test
`,
      `${name} service authoring`,
    );
    assert.equal(
      http,
      `probe:
  method: POST
  path: /probe
  kind: typedJson
  handler: main.__skiffHttpProbe
  adapterArgs:
    - param: body
      source: { kind: http.body }
`,
      `${name} HTTP authoring`,
    );
    await assert.rejects(
      access(join(root, 'config.skiff-test.yml')),
      { code: 'ENOENT' },
      `${name} must not carry an empty legacy test profile`,
    );
    assert.equal(
      websocket,
      `path: /socket
connect:
  handler: main.websocketConnect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }
`,
      `${name} WebSocket authoring`,
    );
    assert.match(
      source,
      /function websocketConnect\(\s+request: std\.websocket\.WebSocketConnectRequest,\s+connectionId: string\s+\) -> std\.websocket\.WebSocketConnectResult \{\s+return \{\s+tag: "accept",\s+businessIdentity: connectionId,\s+connectionPolicy: null\s+\}\s+\}/,
      `${name} connect callable`,
    );
    assert.deepEqual(
      [...source.matchAll(/^function ([A-Za-z0-9_]+)\(/gm)]
        .map((match) => match[1]),
      functionNames,
      `${name} source callable inventory`,
    );
  }
});

test('v4 receipt accepts exactly the test-service and probe HTTP gateways', () => {
  const receipt = validSmokeFixtureReceipt(ENVIRONMENT);
  const decoded = readPackageServiceFixtureReceipt(
    JSON.stringify(receipt),
    ENVIRONMENT,
  );

  assert.equal(decoded.schemaVersion, 'skiff-package-service-smoke-fixture-v4');
  assert.equal(decoded.candidate.entrypoints.length, 2);
  assert.deepEqual(
    decoded.candidate.entrypoints.map((entrypoint) => ({
      key: entrypoint.gatewayEntryKey,
      identity: entrypoint.gatewayEntryIdentity,
      selector: entrypoint.selector,
    })),
    [
      {
        key: 'run',
        identity: smokeFixtureIdentities.packageTestGateway,
        selector: {
          protocol: 'http',
          method: 'POST',
          path: '/__skiff/test/0',
        },
      },
      {
        key: 'probe',
        identity: smokeFixtureIdentities.smokeProbeGateway,
        selector: {
          protocol: 'http',
          method: 'POST',
          path: '/probe',
        },
      },
    ],
  );
});

test('retired receipt schemas and entrypoint fields fail closed without dual-read', () => {
  const cases = [];
  const v1 = validSmokeFixtureReceipt(ENVIRONMENT);
  v1.schemaVersion = 'skiff-package-service-smoke-fixture-v1';
  cases.push(['v1 schema', v1]);
  const v2 = validSmokeFixtureReceipt(ENVIRONMENT);
  v2.schemaVersion = 'skiff-package-service-smoke-fixture-v2';
  cases.push(['v2 schema', v2]);
  const v3 = validSmokeFixtureReceipt(ENVIRONMENT);
  v3.schemaVersion = 'skiff-package-service-smoke-fixture-v3';
  delete v3.candidate.configSnapshot;
  cases.push(['v3 schema', v3]);

  const contract = validSmokeFixtureReceipt(ENVIRONMENT);
  contract.candidate.entrypoints[0].contract = {
    serviceId: 'retired',
    contractVersion: '1.0.0',
    serviceProtocolIdentity:
      `skiff-service-protocol-v5:sha256:${'a'.repeat(64)}`,
  };
  cases.push(['contract field', contract]);

  const operation = validSmokeFixtureReceipt(ENVIRONMENT);
  operation.candidate.entrypoints[1].operation =
    `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;
  cases.push(['operation field', operation]);

  for (const [name, receipt] of cases) {
    assert.throws(
      () => readPackageServiceFixtureReceipt(JSON.stringify(receipt), ENVIRONMENT),
      undefined,
      `${name} was accepted`,
    );
  }
});

test('a third WebSocket candidate is not an HTTP fixture entrypoint', () => {
  const receipt = validSmokeFixtureReceipt(ENVIRONMENT);
  receipt.candidate.entrypoints.push({
    deployment: receipt.candidate.entrypoints[1].deployment,
    gatewayEntryKey: 'websocket',
    gatewayEntryIdentity: smokeFixtureIdentities.smokeProbeGateway,
    mode: 'unary',
    selector: {
      protocol: 'webSocket',
      host: 'ecosystem-smoke.skiff.localhost',
      method: null,
      path: '/socket',
    },
  });

  assert.throws(() =>
    readPackageServiceFixtureReceipt(JSON.stringify(receipt), ENVIRONMENT));
});

test('gateway identities, keys, modes and selectors are exact', () => {
  const mutations = [
    ['shared response identity', (receipt) => {
      receipt.candidate.entrypoints[1].gatewayEntryIdentity =
        smokeFixtureIdentities.packageTestGateway;
    }],
    ['wrong key', (receipt) => {
      receipt.candidate.entrypoints[0].gatewayEntryKey = 'legacy-run';
    }],
    ['wrong mode', (receipt) => {
      receipt.candidate.entrypoints[1].mode = 'serverStream';
    }],
    ['normalized host', (receipt) => {
      receipt.candidate.entrypoints[1].selector.host =
        'ECOSYSTEM-SMOKE.SKIFF.LOCALHOST';
    }],
    ['wrong protocol', (receipt) => {
      receipt.candidate.entrypoints[0].selector.protocol = 'webSocket';
    }],
  ];

  for (const [name, mutate] of mutations) {
    const receipt = validSmokeFixtureReceipt(ENVIRONMENT);
    mutate(receipt);
    assert.throws(
      () => readPackageServiceFixtureReceipt(JSON.stringify(receipt), ENVIRONMENT),
      undefined,
      `${name} was accepted`,
    );
  }
});
