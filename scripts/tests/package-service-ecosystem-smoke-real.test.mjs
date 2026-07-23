import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import {
  access,
  mkdtemp,
  readFile,
  rm,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { inspect } from 'node:util';

import { commandExecutionError } from '../lib/command-execution-internal.mjs';
import {
  FIXTURE_CARGO_DIAGNOSTIC_PROPERTY,
} from '../lib/package-service-ecosystem-smoke-diagnostic.mjs';
import {
  packageServiceEcosystemSmokeExpectedMarker,
  packageServiceEcosystemSmokeFixtureCargoArgs,
  packageServiceEcosystemSmokeFixtureRoot,
  runPackageServiceEcosystemSmoke,
} from '../lib/package-service-ecosystem-smoke-real.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('real ecosystem smoke uses the checked-in normal-source WebSocket fixture', async () => {
  const fixtureRoot = packageServiceEcosystemSmokeFixtureRoot(checkout);
  assert.equal(
    fixtureRoot,
    join(checkout, 'test-runner', 'fixtures', 'package-service-websocket-smoke')
  );
  const source = await readFile(join(fixtureRoot, 'main.skiff'), 'utf8');
  assert.match(source, /WebSocketIngressEvent<null>/);
  assert.match(source, /sendTextToConnection\(event\.receiveEvent\.connection\.id, marker\(\)\)/);
  assert.match(source, new RegExp(packageServiceEcosystemSmokeExpectedMarker));

  assert.deepEqual(
    packageServiceEcosystemSmokeFixtureCargoArgs({
      checkout,
      fixtureRoot,
      artifactRoot: '/tmp/f23d-artifacts',
      environment: 'f23d-test'
    }),
    [
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
      '/tmp/f23d-artifacts',
      '--platform-source-root',
      checkout,
      '--environment',
      'f23d-test'
    ]
  );
});

test('fixture Cargo failure evidence remains visible after isolated cleanup', async () => {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-f26a-smoke-'));
  const secret = 'P5_F26A_CLEANUP_SECRET_SENTINEL';
  const stderr =
    `error: password=${secret} at ${join(tempRoot, secret, 'Cargo.toml')}`;
  const originalCause = new Error('fixture process cause');
  const cargoError = fixtureCargoError({ code: 101, stdout: '', stderr });
  Object.defineProperty(cargoError, 'cause', { value: originalCause });
  let commandCount = 0;
  let cleanupCount = 0;
  let activationCount = 0;
  let observed;

  const runtimeOwner = async ({ runTest }) => {
    let testError;
    try {
      await runTest(
        { SKIFF_TEST_ENVIRONMENT: 'f26a-test' },
        new AbortController().signal,
        {
          artifactRoot: join(tempRoot, 'artifacts'),
          controlUrl: 'http://127.0.0.1:46001',
          routerHttpUrl: 'http://127.0.0.1:46000',
        },
      );
    } catch (error) {
      testError = error;
    } finally {
      cleanupCount += 1;
      await rm(tempRoot, { recursive: true, force: true });
    }
    if (testError !== undefined) throw testError;
    throw new Error('fixture Cargo failure double unexpectedly passed');
  };

  await assert.rejects(
    runPackageServiceEcosystemSmoke({
      checkout,
      replicaCount: 1,
      environment: 'f26a-test',
    }, {
      runtimeOwner,
      runCommand: async () => {
        commandCount += 1;
        throw cargoError;
      },
      activate: async () => {
        activationCount += 1;
      },
      loadWebSocket: async () => {
        throw new Error('WebSocket must not load after fixture Cargo failure');
      },
    }),
    (error) => {
      observed = error;
      return true;
    },
  );

  assert.equal(observed, cargoError);
  assert.equal(observed.cause, originalCause);
  assert.equal(observed.message, 'cargo exited with 101');
  assert.equal(observed.command, 'cargo');
  assert.equal(observed.code, 101);
  assert.equal(observed.signal, null);
  assert.equal(commandCount, 1);
  assert.equal(cleanupCount, 1);
  assert.equal(activationCount, 0);
  await assert.rejects(access(tempRoot), { code: 'ENOENT' });
  const evidence = observed[FIXTURE_CARGO_DIAGNOSTIC_PROPERTY];
  assert.equal(evidence.stderrBytes, Buffer.byteLength(stderr));
  assert.equal(evidence.stderrSha256, sha256(stderr));
  assert.match(evidence.diagnostics[0].sanitizedExcerpt, /<PATH>/);
  assert.match(evidence.diagnostics[0].sanitizedExcerpt, /<REDACTED_SECRET>/);
  assert.equal(Object.keys(observed).includes('stderr'), false);
  for (const rendered of [JSON.stringify(observed), inspect(observed)]) {
    assert.equal(rendered.includes(tempRoot), false);
    assert.equal(rendered.includes(secret), false);
    assert.equal(rendered.includes(stderr), false);
  }
});

test('successful ecosystem smoke output and fixture command count remain unchanged', async () => {
  const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
  const operation = `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;
  const commandCalls = [];
  const activationCalls = [];
  FakeWebSocket.instances.length = 0;
  const result = await runPackageServiceEcosystemSmoke({
    checkout,
    replicaCount: 1,
    environment: 'f26a-success',
  }, {
    runtimeOwner: async ({ runTest }) => runTest(
      { SKIFF_TEST_ENVIRONMENT: 'f26a-success' },
      new AbortController().signal,
      {
        artifactRoot: '/isolated/artifacts',
        controlUrl: 'http://127.0.0.1:46001',
        routerHttpUrl: 'http://127.0.0.1:46000',
      },
    ),
    runCommand: async (...args) => {
      commandCalls.push(args);
      return {
        stdout: JSON.stringify({
          schemaVersion: 'skiff-package-service-smoke-fixture-v1',
          environment: 'f26a-success',
          candidate: {
            assembly: { assemblyIdentity },
            entrypoints: [{
              kind: 'websocket',
              host: 'fixture.skiff.test',
              path: '/socket',
              operation,
            }],
          },
        }),
        stderr: '',
      };
    },
    activate: async (input) => {
      activationCalls.push(input);
      return {
        response: {
          ok: true,
          activeAssembly: { assemblyIdentity, generation: 0 },
        },
      };
    },
    loadWebSocket: async () => FakeWebSocket,
  });

  assert.equal(commandCalls.length, 1);
  assert.equal(commandCalls[0][0], 'cargo');
  assert.equal(activationCalls.length, 1);
  assert.equal(FakeWebSocket.instances.length, 1);
  assert.deepEqual(result, {
    status: 'PASS',
    probe: 'skiff-cutover-production-websocket-component',
    replicas: 1,
    generation: 0,
    assembly: assemblyIdentity,
    sourceFixture: join(
      'test-runner',
      'fixtures',
      'package-service-websocket-smoke',
    ),
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
      host: 'fixture.skiff.test',
      path: '/socket',
      operation,
      marker: packageServiceEcosystemSmokeExpectedMarker,
    },
  });
  assert.equal(Object.hasOwn(result, FIXTURE_CARGO_DIAGNOSTIC_PROPERTY), false);
});

class FakeWebSocket extends EventEmitter {
  static CONNECTING = 0;

  static OPEN = 1;

  static CLOSED = 3;

  static instances = [];

  constructor(url, options) {
    super();
    this.url = url;
    this.options = options;
    this.readyState = FakeWebSocket.CONNECTING;
    FakeWebSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = FakeWebSocket.OPEN;
      this.emit('open');
    });
  }

  send(message) {
    assert.equal(message, 'production-component-probe');
    queueMicrotask(() => this.emit(
      'message',
      packageServiceEcosystemSmokeExpectedMarker,
    ));
  }

  close() {
    this.readyState = FakeWebSocket.CLOSED;
    queueMicrotask(() => this.emit('close'));
  }

  terminate() {
    this.close();
  }
}

function fixtureCargoError({
  code,
  signal = null,
  stdout,
  stderr,
}) {
  return commandExecutionError(
    'cargo',
    { code, signal, error: null },
    { stdout, stderr },
  );
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
