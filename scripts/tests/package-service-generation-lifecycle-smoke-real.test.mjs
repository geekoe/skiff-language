import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { commandExecutionError } from '../lib/command-execution-internal.mjs';
import {
  FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES,
} from '../lib/package-service-ecosystem-smoke-diagnostic.mjs';
import {
  GENERATION_LIFECYCLE_FIXTURE_DIAGNOSTIC_PROPERTY,
  packageServiceGenerationFixtureRoot,
  packageServiceGenerationLifecycleExpectedMarkers,
  requestGenerationUnary,
  runPackageServiceGenerationLifecycleSmoke,
} from '../lib/package-service-generation-lifecycle-smoke-real.mjs';
import {
  packageServiceGenerationLifecycleOracleConstants,
  validatePackageServiceGenerationFixturePair,
} from '../lib/package-service-generation-lifecycle-smoke-oracle.mjs';
import {
  validBootstrapReceipt,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';
import { encodeRuntimePayload } from '../lib/runtime-payload-codec.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const environment = 'r05-generation-lifecycle-test';
const sha256 = (value) => createHash('sha256').update(value).digest('hex');

test('generation lifecycle fixtures preserve the contract and distinguish source markers', async () => {
  const fixtures = [
    ['A', packageServiceGenerationLifecycleExpectedMarkers.A],
    ['B', packageServiceGenerationLifecycleExpectedMarkers.B],
  ];
  for (const [candidate, marker] of fixtures) {
    const root = packageServiceGenerationFixtureRoot(checkout, candidate);
    assert.equal(
      root,
      join(
        checkout,
        'test-runner',
        'fixtures',
        `package-service-websocket-generation-${candidate.toLowerCase()}`,
      ),
    );
    const [packageText, apiText, source, testSource] = await Promise.all([
      readFile(join(root, 'package.yml'), 'utf8'),
      readFile(join(root, 'api.yml'), 'utf8'),
      readFile(join(root, 'main.skiff'), 'utf8'),
      readFile(join(root, 'main.test.skiff'), 'utf8'),
    ]);
    assert.match(
      packageText,
      new RegExp(`id: ${packageServiceGenerationLifecycleOracleConstants.packageId}`),
    );
    assert.match(apiText, /websocket: main\.websocket/);
    assert.match(source, /WebSocketIngressEvent<null>/);
    assert.match(source, new RegExp(marker));
    assert.match(
      testSource,
      new RegExp(packageServiceGenerationLifecycleOracleConstants.packageTestName),
    );
  }
});

test('generation lifecycle transcript authors A then B and closes both generation pins', async () => {
  const receipts = lifecycleReceipts(environment);
  const commandCalls = [];
  const activationCalls = [];
  const healthCalls = [];
  const healthSequence = [
    lifecycleHealth(receipts.A, 1, 0, 0),
    lifecycleHealth(receipts.B, 2, 1, 0),
    lifecycleHealth(receipts.B, 2, 2, 0),
    lifecycleHealth(receipts.B, 2, 1, 1),
    lifecycleHealth(receipts.B, 2, 0, 2),
  ];
  GenerationWebSocket.instances.length = 0;
  const unaryServer = await listenUnaryServer({
    status: 200,
    body: encodeRuntimePayload(
      packageServiceGenerationLifecycleExpectedMarkers.B,
      { type: 'string' },
    ),
  });

  let result;
  try {
    result = await runPackageServiceGenerationLifecycleSmoke({
      checkout,
      replicaCount: 1,
      environment,
    }, {
      runtimeOwner: fakeRuntimeOwner(environment, unaryServer.origin),
      runCommand: async (command, args) => {
        commandCalls.push({ command, args });
        const candidate = commandCalls.length === 1 ? 'A' : 'B';
        return { stdout: JSON.stringify(receipts[candidate]), stderr: '' };
      },
      activate: async (input) => {
        activationCalls.push(input);
        const candidate = input.expectedGeneration === 0 ? receipts.A : receipts.B;
        return activationReceipt(
          environment,
          candidate.candidate.assembly.assemblyIdentity,
          input.expectedGeneration,
        );
      },
      readHealth: async (url) => {
        healthCalls.push(url);
        return healthSequence.shift();
      },
      readinessSleep: async () => {},
      loadWebSocket: async () => GenerationWebSocket,
    });
  } finally {
    await unaryServer.close();
  }

  assert.equal(commandCalls.length, 2);
  assert.ok(commandCalls[0].args.includes(
    packageServiceGenerationFixtureRoot(checkout, 'A'),
  ));
  assert.ok(commandCalls[1].args.includes(
    packageServiceGenerationFixtureRoot(checkout, 'B'),
  ));
  assert.deepEqual(
    activationCalls.map(({ signal: _signal, ...input }) => input),
    [
      {
        activationUrl: 'http://127.0.0.1:46001/__skiff/activate-assembly',
        expectedGeneration: 0,
        environment,
        assembly: receipts.A.candidate.assembly,
      },
      {
        activationUrl: 'http://127.0.0.1:46001/__skiff/activate-assembly',
        expectedGeneration: 1,
        environment,
        assembly: receipts.B.candidate.assembly,
      },
    ],
  );
  assert.equal(healthCalls.length, 5);
  assert.equal(GenerationWebSocket.instances.length, 2);
  assert.deepEqual(
    GenerationWebSocket.instances.map((client) => ({
      sent: client.sent,
      closeCalls: client.closeCalls,
    })),
    [
      {
        sent: ['r05-generation-lifecycle-probe', 'r05-generation-lifecycle-probe'],
        closeCalls: 1,
      },
      {
        sent: ['r05-generation-lifecycle-probe'],
        closeCalls: 1,
      },
    ],
  );
  assert.deepEqual(unaryServer.requests, [{
    method: 'POST',
    url: '/probe',
    host: 'ecosystem-smoke.skiff.localhost',
  }]);
  assert.equal(result.status, 'PASS');
  assert.deepEqual(result.generations, { A: 1, B: 2 });
  assert.equal(result.candidates.A.packageBuildId,
    receipts.A.candidate.production.packageBuildId);
  assert.equal(result.candidates.B.packageBuildId,
    receipts.B.candidate.production.packageBuildId);
  assert.deepEqual(result.markers, packageServiceGenerationLifecycleExpectedMarkers);
});

test('generation decode error remains primary when finally cleanup also fails', async () => {
  const receipts = lifecycleReceipts(environment);
  const healthSequence = [
    lifecycleHealth(receipts.A, 1, 0, 0),
    lifecycleHealth(receipts.B, 2, 1, 0),
    lifecycleHealth(receipts.B, 2, 2, 0),
  ];
  let commandCount = 0;
  const cleanupError = new Error('generation cleanup failed');
  GenerationWebSocket.instances.length = 0;
  CleanupFailingWebSocket.cleanupError = cleanupError;
  const unaryServer = await listenUnaryServer({
    status: 200,
    body: Buffer.from(JSON.stringify(packageServiceGenerationLifecycleExpectedMarkers.B)),
  });

  let observed;
  try {
    await assert.rejects(
      runPackageServiceGenerationLifecycleSmoke({
        checkout,
        replicaCount: 1,
        environment,
      }, {
        runtimeOwner: fakeRuntimeOwner(environment, unaryServer.origin),
        runCommand: async () => {
          commandCount += 1;
          return {
            stdout: JSON.stringify(commandCount === 1 ? receipts.A : receipts.B),
            stderr: '',
          };
        },
        activate: async ({ expectedGeneration }) => {
          const receipt = expectedGeneration === 0 ? receipts.A : receipts.B;
          return activationReceipt(
            environment,
            receipt.candidate.assembly.assemblyIdentity,
            expectedGeneration,
          );
        },
        readHealth: async () => healthSequence.shift(),
        readinessSleep: async () => {},
        loadWebSocket: async () => CleanupFailingWebSocket,
      }),
      (error) => {
        observed = error;
        return true;
      },
    );
  } finally {
    await unaryServer.close();
  }

  assert.notEqual(observed, cleanupError);
  assert.match(observed.message, /runtime payload bytes missing SKPV magic/);
  assert.doesNotMatch(observed.message, /cleanup failed/);
  assert.deepEqual(
    GenerationWebSocket.instances.map((client) => client.closeCalls),
    [1, 1],
  );
});

test('generation unary client preserves bounded raw 200 bytes', async () => {
  const encoded = encodeRuntimePayload('raw-success', { type: 'string' });
  const unaryServer = await listenUnaryServer({ status: 200, body: encoded });

  try {
    const response = await requestGenerationUnary({
      url: `${unaryServer.origin}/probe`,
      host: 'ecosystem-smoke.skiff.localhost',
      signal: new AbortController().signal,
    });
    assert.equal(response.status, 200);
    assert.deepEqual(response.body, encoded);
    assert.equal(response.bodyBytes, encoded.byteLength);
    assert.equal(response.bodyTruncated, false);
  } finally {
    await unaryServer.close();
  }
});

test('generation unary client fails closed when a 200 body exceeds 512 bytes', async () => {
  const unaryServer = await listenUnaryServer({
    status: 200,
    body: Buffer.alloc(513, 0x61),
  });

  try {
    await assert.rejects(
      requestGenerationUnary({
        url: `${unaryServer.origin}/probe`,
        host: 'ecosystem-smoke.skiff.localhost',
        signal: new AbortController().signal,
      }),
      /generation B unary response exceeded 512 bytes/,
    );
  } finally {
    await unaryServer.close();
  }
});

test('generation unary client reports bounded redacted 404 wire diagnostics', async () => {
  const secret = 'token=R05_UNARY_SECRET';
  const responseBody = `${secret} ${'/private/fixture/main.skiff '.repeat(40)}`;
  const unaryServer = await listenUnaryServer({ status: 404, body: responseBody });

  try {
    await assert.rejects(
      requestGenerationUnary({
        url: `${unaryServer.origin}/probe`,
        host: 'ecosystem-smoke.skiff.localhost',
        signal: new AbortController().signal,
      }),
      (error) => {
        assert.match(error.message, /method=POST/);
        assert.match(error.message, new RegExp(`url=${unaryServer.origin}/probe`));
        assert.match(error.message, /wireHost=ecosystem-smoke\.skiff\.localhost/);
        assert.match(error.message, /status=404/);
        assert.match(error.message, /responseBodyBytes=/);
        assert.match(error.message, /responseBodyTruncated=true/);
        assert.match(error.message, /<REDACTED_SECRET>/);
        assert.doesNotMatch(error.message, /R05_UNARY_SECRET/);
        assert.doesNotMatch(error.message, /private\/fixture/);
        return true;
      },
    );
  } finally {
    await unaryServer.close();
  }

  assert.deepEqual(unaryServer.requests, [{
    method: 'POST',
    url: '/probe',
    host: 'ecosystem-smoke.skiff.localhost',
  }]);
});

test('generation fixture Cargo failures retain bounded evidence with the A or B label', async (t) => {
  for (const failedCandidate of ['A', 'B']) {
    await t.test(failedCandidate, async () => {
      const receipts = lifecycleReceipts(environment);
      const stderr = [
        `error: token=P5_R05_${failedCandidate}_SECRET at /private/fixture/main.skiff`,
        'warning: candidate failed',
        'note: compiler stopped',
        'help: inspect locally',
      ].join('\n');
      const cargoError = commandExecutionError(
        'cargo',
        { code: 101, signal: null, error: null },
        { stdout: 'partial receipt', stderr },
      );
      let commandCount = 0;
      let observed;

      await assert.rejects(
        runPackageServiceGenerationLifecycleSmoke({
          checkout,
          replicaCount: 1,
          environment,
        }, {
          runtimeOwner: fakeRuntimeOwner(environment),
          runCommand: async () => {
            commandCount += 1;
            if (
              failedCandidate === 'A'
              || (failedCandidate === 'B' && commandCount === 2)
            ) {
              throw cargoError;
            }
            return { stdout: JSON.stringify(receipts.A), stderr: '' };
          },
          activate: async ({ expectedGeneration }) => activationReceipt(
            environment,
            receipts.A.candidate.assembly.assemblyIdentity,
            expectedGeneration,
          ),
          readHealth: async () => lifecycleHealth(receipts.A, 1, 0, 0),
          readinessSleep: async () => {},
          loadWebSocket: async () => GenerationWebSocket,
        }),
        (error) => {
          observed = error;
          return true;
        },
      );

      assert.equal(observed, cargoError);
      const evidence =
        observed[GENERATION_LIFECYCLE_FIXTURE_DIAGNOSTIC_PROPERTY];
      assert.equal(evidence.candidate, failedCandidate);
      assert.equal(evidence.stderrBytes, Buffer.byteLength(stderr));
      assert.equal(evidence.stderrSha256, sha256(stderr));
      assert.equal(evidence.diagnostics.length, FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES);
      assert.equal(evidence.diagnosticOmittedCount, 2);
      assert.equal(JSON.stringify(evidence).includes('P5_R05_'), false);
      assert.equal(JSON.stringify(evidence).includes('/private/fixture'), false);
    });
  }
});

test('generation pair oracle rejects identity collapse and protocol drift', () => {
  const receipts = lifecycleReceipts(environment);
  const pair = validatePackageServiceGenerationFixturePair(receipts.A, receipts.B);
  assert.notEqual(pair.A.packageRecordPath, pair.B.packageRecordPath);
  assert.equal(
    pair.A.operationIdentities.websocket,
    pair.B.operationIdentities.websocket,
  );

  const sameBuild = structuredClone(receipts.B);
  sameBuild.candidate.production.packageBuildId =
    receipts.A.candidate.production.packageBuildId;
  assert.throws(
    () => validatePackageServiceGenerationFixturePair(receipts.A, sameBuild),
    /distinct PackageBuildId/,
  );

  const protocolDrift = structuredClone(receipts.B);
  protocolDrift.candidate.entrypoints[1].contract.serviceProtocolIdentity =
    identity('skiff-service-protocol-v2:sha256', 'f');
  protocolDrift.candidate.entrypoints[2].contract.serviceProtocolIdentity =
    protocolDrift.candidate.entrypoints[1].contract.serviceProtocolIdentity;
  assert.throws(
    () => validatePackageServiceGenerationFixturePair(receipts.A, protocolDrift),
    /preserve service protocol identity/,
  );
});

class GenerationWebSocket extends EventEmitter {
  static CONNECTING = 0;

  static OPEN = 1;

  static CLOSED = 3;

  static instances = [];

  constructor(url, options) {
    super();
    this.url = url;
    this.options = options;
    this.readyState = GenerationWebSocket.CONNECTING;
    this.sent = [];
    this.closeCalls = 0;
    this.marker = GenerationWebSocket.instances.length === 0
      ? packageServiceGenerationLifecycleExpectedMarkers.A
      : packageServiceGenerationLifecycleExpectedMarkers.B;
    GenerationWebSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = GenerationWebSocket.OPEN;
      this.emit('open');
    });
  }

  send(message) {
    this.sent.push(message);
    queueMicrotask(() => this.emit('message', this.marker));
  }

  close() {
    this.closeCalls += 1;
    this.readyState = GenerationWebSocket.CLOSED;
    queueMicrotask(() => this.emit('close'));
  }

  terminate() {
    this.close();
  }
}

class CleanupFailingWebSocket extends GenerationWebSocket {
  static cleanupError;

  close() {
    this.closeCalls += 1;
    throw CleanupFailingWebSocket.cleanupError;
  }

  terminate() {
    throw CleanupFailingWebSocket.cleanupError;
  }
}

function lifecycleReceipts(targetEnvironment) {
  const A = generationReceipt(targetEnvironment, {
    assembly: 'a',
    productionBuild: '1',
    productionAbi: '2',
    overlayBuild: '3',
    overlayAbi: '4',
    packageTestDeployment: '7',
    smokeDeployment: '8',
  });
  const B = generationReceipt(targetEnvironment, {
    assembly: 'b',
    productionBuild: 'd',
    productionAbi: 'e',
    overlayBuild: 'f',
    overlayAbi: '0',
    packageTestDeployment: '1',
    smokeDeployment: '2',
  });
  return { A, B };
}

function generationReceipt(targetEnvironment, characters) {
  const receipt = validSmokeFixtureReceipt(targetEnvironment);
  receipt.candidate.entrypoints[0].name =
    packageServiceGenerationLifecycleOracleConstants.packageTestName;
  receipt.candidate.assembly.assemblyIdentity =
    identity('skiff-runtime-assembly-v1:sha256', characters.assembly);
  receipt.candidate.production.packageBuildId =
    identity('skiff-package-build-v4:sha256', characters.productionBuild);
  receipt.candidate.production.packageLocalAbiIdentity =
    identity('skiff-package-local-abi-v3:sha256', characters.productionAbi);
  receipt.candidate.overlay.packageBuildId =
    identity('skiff-package-build-v4:sha256', characters.overlayBuild);
  receipt.candidate.overlay.packageLocalAbiIdentity =
    identity('skiff-package-local-abi-v3:sha256', characters.overlayAbi);
  receipt.candidate.overlayRecordPath = [
    'records/package-artifacts/test~dskiff~spackage-service-websocket-smoke',
    '1.0.0',
    characters.overlayBuild.repeat(64),
    'package.json',
  ].join('/');
  const packageTestDeployment = receipt.candidate.entrypoints[0].deployment;
  packageTestDeployment.deploymentRevision =
    `test-${characters.overlayBuild.repeat(64)}`;
  packageTestDeployment.deploymentArtifactIdentity =
    identity('skiff-deployment-artifact-v1:sha256', characters.packageTestDeployment);
  const smokeDeployment = {
    ...receipt.candidate.entrypoints[1].deployment,
    deploymentRevision: `smoke-${characters.productionBuild.repeat(64)}`,
    deploymentArtifactIdentity:
      identity('skiff-deployment-artifact-v1:sha256', characters.smokeDeployment),
  };
  receipt.candidate.entrypoints[1].deployment = smokeDeployment;
  receipt.candidate.entrypoints[2].deployment = structuredClone(smokeDeployment);
  return receipt;
}

function activationReceipt(
  targetEnvironment,
  assemblyIdentity,
  expectedGeneration,
) {
  const generation = expectedGeneration + 1;
  return {
    request: {
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment: targetEnvironment,
      activationId: `r05-generation-${generation}`,
      expectedGeneration,
      assembly: { assemblyIdentity },
    },
    response: {
      ok: true,
      committed: {
        generation,
        assembly: { assemblyIdentity },
      },
      activeAssembly: {
        environment: targetEnvironment,
        generation,
        assemblyIdentity,
      },
      replicas: [],
    },
  };
}

function lifecycleHealth(
  receipt,
  generation,
  connectionPinCount,
  connectionReleaseAckCount,
) {
  const replicaId = 'runtime-r05';
  return {
    ok: true,
    activeAssembly: {
      environment,
      generation,
      assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
      ingressCount: 3,
    },
    pendingActivation: null,
    capabilityConnections: [{
      runtimeId: replicaId,
      connected: true,
      registeredAt: '2026-07-23T00:00:00.000Z',
      capabilities: { runtimeProgram: true },
    }],
    replicas: [{
      replicaId,
      environment,
      generation,
      assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
      state: 'healthy',
      connected: true,
      inFlightCount: 0,
      connectionPinCount,
      connectionReleaseAckCount,
      registeredAt: '2026-07-23T00:00:00.000Z',
    }],
  };
}

function fakeRuntimeOwner(
  targetEnvironment,
  routerHttpUrl = 'http://127.0.0.1:46000',
) {
  return async ({ runTest, validateBootstrapReceipt }) => {
    validateBootstrapReceipt(validBootstrapReceipt(targetEnvironment));
    return runTest(
      { SKIFF_TEST_ENVIRONMENT: targetEnvironment },
      new AbortController().signal,
      {
        artifactRoot: '/isolated/artifacts',
        controlUrl: 'http://127.0.0.1:46001',
        routerHttpUrl,
      },
    );
  };
}

async function listenUnaryServer({ status, body }) {
  const requests = [];
  const server = createServer((request, response) => {
    requests.push({
      method: request.method,
      url: request.url,
      host: request.headers.host,
    });
    response.statusCode = status;
    response.end(body);
  });
  await new Promise((resolveListen) => {
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const address = server.address();
  assert.ok(address !== null && typeof address !== 'string');
  return {
    close: () => new Promise((resolveClose, rejectClose) => {
      server.close((error) => {
        if (error !== undefined) rejectClose(error);
        else resolveClose();
      });
    }),
    origin: `http://127.0.0.1:${address.port}`,
    requests,
  };
}

function identity(prefix, character) {
  return `${prefix}:${character.repeat(64)}`;
}
