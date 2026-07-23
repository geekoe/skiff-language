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
  validatePackageServiceBootstrapReceipt,
} from '../lib/package-service-ecosystem-smoke-oracle.mjs';
import {
  packageServiceEcosystemSmokeExpectedMarker,
  packageServiceEcosystemSmokeFixtureCargoArgs,
  packageServiceEcosystemSmokeFixtureRoot,
  runPackageServiceEcosystemSmoke,
} from '../lib/package-service-ecosystem-smoke-real.mjs';
import {
  readyAssemblyHealth,
  smokeFixtureIdentities,
  validActivationReceipt,
  validBootstrapReceipt,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';

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

test('ecosystem smoke waits for exact delayed readiness and creates one WebSocket', async () => {
  const environment = 'f27c-success';
  const commandCalls = [];
  const activationCalls = [];
  const healthCalls = [];
  const healthSequence = [
    readyAssemblyHealth(environment, {
      activeAssembly: { generation: 0 },
      replicas: [],
      capabilityConnections: [],
    }),
    readyAssemblyHealth(environment, {
      pendingActivation: {
        candidateGeneration: 1,
        assembly: { assemblyIdentity: smokeFixtureIdentities.assembly },
      },
    }),
    readyAssemblyHealth(environment, { capabilityConnections: [] }),
    readyAssemblyHealth(environment),
  ];
  FakeWebSocket.instances.length = 0;
  const result = await runPackageServiceEcosystemSmoke({
    checkout,
    replicaCount: 1,
    environment,
  }, {
    runtimeOwner: fakeRuntimeOwner(environment),
    runCommand: async (...args) => {
      commandCalls.push(args);
      return {
        stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
        stderr: '',
      };
    },
    activate: async (input) => {
      activationCalls.push(input);
      return validActivationReceipt(environment);
    },
    readHealth: async (url) => {
      healthCalls.push(url);
      return healthSequence.shift();
    },
    readinessSleep: async () => {},
    loadWebSocket: async () => FakeWebSocket,
  });

  assert.equal(commandCalls.length, 1);
  assert.equal(commandCalls[0][0], 'cargo');
  assert.equal(activationCalls.length, 1);
  assert.deepEqual(activationCalls[0], {
    activationUrl: 'http://127.0.0.1:46001/__skiff/activate-assembly',
    expectedGeneration: 0,
    environment,
    assembly: { assemblyIdentity: smokeFixtureIdentities.assembly },
  });
  assert.deepEqual(healthCalls, Array(4).fill(
    'http://127.0.0.1:46001/__router/health',
  ));
  assert.equal(FakeWebSocket.instances.length, 1);
  assert.equal(FakeWebSocket.instances[0].url, 'ws://127.0.0.1:46000/socket');
  assert.deepEqual(FakeWebSocket.instances[0].options, {
    headers: { Host: 'ecosystem-smoke.skiff.localhost' },
  });
  assert.deepEqual(result, {
    status: 'PASS',
    probe: 'skiff-cutover-production-websocket-component',
    replicas: 1,
    generation: 1,
    assembly: smokeFixtureIdentities.assembly,
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
      host: 'ecosystem-smoke.skiff.localhost',
      path: '/socket',
      operation: smokeFixtureIdentities.websocketOperation,
      marker: packageServiceEcosystemSmokeExpectedMarker,
    },
  });
  assert.equal(Object.hasOwn(result, FIXTURE_CARGO_DIAGNOSTIC_PROPERTY), false);
});

test('fixture receipt mutations fail before activation, health, or WebSocket creation', async (t) => {
  const environment = 'f27c-fixture-mutation';
  const cases = [
    {
      name: 'unknown top-level field',
      mutate: (receipt) => { receipt.extra = true; },
      error: /fixture receipt must have exact keys/,
    },
    {
      name: 'missing production ref',
      mutate: (receipt) => { delete receipt.candidate.production; },
      error: /fixture candidate must have exact keys/,
    },
    {
      name: 'overlay record points at another build',
      mutate: (receipt) => { receipt.candidate.overlayRecordPath += '.wrong'; },
      error: /overlayRecordPath must select the exact overlay/,
    },
    {
      name: 'contract ref is incomplete',
      mutate: (receipt) => {
        receipt.candidate.entrypoints[2].contract = {
          ...receipt.candidate.entrypoints[2].contract,
        };
        delete receipt.candidate.entrypoints[2].contract.serviceProtocolIdentity;
      },
      error: /websocket contract must have exact keys/,
    },
    {
      name: 'deployment does not bind its contract',
      mutate: (receipt) => {
        receipt.candidate.entrypoints[1].deployment.serviceId = 'wrong/service';
      },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'entrypoint set is incomplete',
      mutate: (receipt) => { receipt.candidate.entrypoints.pop(); },
      error: /exactly 3 entrypoints/,
    },
    {
      name: 'WebSocket selector is not fixed',
      mutate: (receipt) => { receipt.candidate.entrypoints[2].path = '/wrong'; },
      error: /Expected values to be strictly equal/,
    },
  ];

  for (const scenario of cases) {
    await t.test(scenario.name, async () => {
      const receipt = validSmokeFixtureReceipt(environment);
      scenario.mutate(receipt);
      const calls = { activation: 0, health: 0, websocket: 0 };
      FakeWebSocket.instances.length = 0;
      await assert.rejects(
        runPackageServiceEcosystemSmoke({
          checkout,
          replicaCount: 1,
          environment,
        }, {
          runtimeOwner: fakeRuntimeOwner(environment),
          runCommand: async () => ({ stdout: JSON.stringify(receipt), stderr: '' }),
          activate: async () => {
            calls.activation += 1;
            return validActivationReceipt(environment);
          },
          readHealth: async () => {
            calls.health += 1;
            return readyAssemblyHealth(environment);
          },
          loadWebSocket: async () => {
            calls.websocket += 1;
            return FakeWebSocket;
          },
        }),
        scenario.error,
      );
      assert.deepEqual(calls, { activation: 0, health: 0, websocket: 0 });
      assert.equal(FakeWebSocket.instances.length, 0);
    });
  }
});

test('bootstrap oracle accepts receipt-owned canonical std build identities', () => {
  const environment = 'f28a-bootstrap-identity';
  for (const character of ['1', 'f']) {
    const receipt = validBootstrapReceipt(environment, {
      packageBuildId: `skiff-package-build-v4:sha256:${character.repeat(64)}`,
    });
    assert.strictEqual(
      validatePackageServiceBootstrapReceipt(receipt, environment),
      receipt,
    );
  }
});

test('bootstrap receipt mutations fail before fixture Cargo or WebSocket creation', async (t) => {
  const environment = 'f27c-bootstrap-mutation';
  const cases = [
    {
      name: 'unknown top-level field',
      mutate: (receipt) => { receipt.extra = true; },
      error: /bootstrap receipt must have exact keys/,
    },
    {
      name: 'wrong std package coordinate',
      mutate: (receipt) => {
        receipt.bootstrap.std.package.artifact.packageId = 'example.com/std';
      },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'wrong std package version',
      mutate: (receipt) => {
        receipt.bootstrap.std.package.artifact.packageVersion = '2.0.0';
      },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'invalid std build identity framing',
      mutate: (receipt) => {
        receipt.bootstrap.std.package.artifact.packageBuildId =
          `skiff-package-build-v4:sha256:${'A'.repeat(64)}`;
      },
      error: /did not match/,
    },
    {
      name: 'published artifact does not match pointer artifact',
      mutate: (receipt) => {
        receipt.bootstrap.std.pointer.artifact.packageBuildId =
          `skiff-package-build-v4:sha256:${'f'.repeat(64)}`;
      },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'package record does not match published artifact identity',
      mutate: (receipt) => { receipt.bootstrap.std.package.recordPath += '.wrong'; },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'pointer does not bind the published std record',
      mutate: (receipt) => { receipt.bootstrap.std.pointer.recordPath += '.wrong'; },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'pointer path is absent',
      mutate: (receipt) => { delete receipt.bootstrap.std.pointerPath; },
      error: /bootstrap std must have exact keys/,
    },
    {
      name: 'std receipt contains an untyped extra field',
      mutate: (receipt) => { receipt.bootstrap.std.extra = true; },
      error: /bootstrap std must have exact keys/,
    },
    {
      name: 'File IR record escapes the exact std record root',
      mutate: (receipt) => {
        receipt.bootstrap.std.package.fileIrRecordPaths[0] =
          `records/package-artifacts/other/1.0.0/${'e'.repeat(64)}.json`;
      },
      error: /File IR records must stay under the exact package record root/,
    },
  ];

  for (const scenario of cases) {
    await t.test(scenario.name, async () => {
      const bootstrap = validBootstrapReceipt(environment);
      scenario.mutate(bootstrap);
      const calls = { cargo: 0, websocket: 0 };
      FakeWebSocket.instances.length = 0;
      await assert.rejects(
        runPackageServiceEcosystemSmoke({
          checkout,
          replicaCount: 1,
          environment,
        }, {
          runtimeOwner: fakeRuntimeOwner(environment, bootstrap),
          runCommand: async () => {
            calls.cargo += 1;
            return {
              stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
              stderr: '',
            };
          },
          loadWebSocket: async () => {
            calls.websocket += 1;
            return FakeWebSocket;
          },
        }),
        scenario.error,
      );
      assert.deepEqual(calls, { cargo: 0, websocket: 0 });
      assert.equal(FakeWebSocket.instances.length, 0);
    });
  }
});

test('activation receipt mutations fail before health or WebSocket creation', async (t) => {
  const environment = 'f27c-activation-mutation';
  const cases = [
    {
      name: 'generation zero remains active',
      mutate: (activation) => { activation.response.activeAssembly.generation = 0; },
      error: /Expected values to be strictly deep-equal/,
    },
    {
      name: 'committed assembly is wrong',
      mutate: (activation) => {
        activation.response.committed.assembly.assemblyIdentity =
          `skiff-runtime-assembly-v1:sha256:${'f'.repeat(64)}`;
      },
      error: /Expected values to be strictly equal/,
    },
    {
      name: 'active environment is wrong',
      mutate: (activation) => {
        activation.response.activeAssembly.environment = 'wrong-environment';
      },
      error: /Expected values to be strictly deep-equal/,
    },
    {
      name: 'response is missing committed tuple',
      mutate: (activation) => { delete activation.response.committed; },
      error: /activation response must have exact keys/,
    },
    {
      name: 'response contains an untyped extra field',
      mutate: (activation) => { activation.response.status = 'accepted'; },
      error: /activation response must have exact keys/,
    },
  ];

  for (const scenario of cases) {
    await t.test(scenario.name, async () => {
      const activation = validActivationReceipt(environment);
      scenario.mutate(activation);
      const calls = { health: 0, websocket: 0 };
      FakeWebSocket.instances.length = 0;
      await assert.rejects(
        runPackageServiceEcosystemSmoke({
          checkout,
          replicaCount: 1,
          environment,
        }, {
          runtimeOwner: fakeRuntimeOwner(environment),
          runCommand: async () => ({
            stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
            stderr: '',
          }),
          activate: async () => activation,
          readHealth: async () => {
            calls.health += 1;
            return readyAssemblyHealth(environment);
          },
          loadWebSocket: async () => {
            calls.websocket += 1;
            return FakeWebSocket;
          },
        }),
        scenario.error,
      );
      assert.deepEqual(calls, { health: 0, websocket: 0 });
      assert.equal(FakeWebSocket.instances.length, 0);
    });
  }
});

test('readiness failures stay on the control plane and never create a WebSocket', async (t) => {
  const environment = 'f27c-readiness-negative';
  const cases = [
    {
      name: 'generation zero',
      health: readyAssemblyHealth(environment, {
        activeAssembly: { generation: 0 },
      }),
      error: /active assembly tuple does not match/,
    },
    {
      name: 'wrong assembly tuple',
      health: readyAssemblyHealth(environment, {
        activeAssembly: {
          assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'f'.repeat(64)}`,
        },
      }),
      error: /active assembly tuple does not match/,
    },
    {
      name: 'pending activation',
      health: readyAssemblyHealth(environment, {
        pendingActivation: { candidateGeneration: 2 },
      }),
      error: /activation is still pending/,
    },
    {
      name: 'no matching healthy replica',
      health: readyAssemblyHealth(environment, { replicas: [] }),
      error: /no healthy connected replica/,
    },
    {
      name: 'no matching capability connection',
      health: readyAssemblyHealth(environment, { capabilityConnections: [] }),
      error: /no matching replica has its own connected capability/,
    },
  ];

  for (const scenario of cases) {
    await t.test(scenario.name, async () => {
      const calls = { health: 0, websocket: 0 };
      FakeWebSocket.instances.length = 0;
      await assert.rejects(
        runPackageServiceEcosystemSmoke({
          checkout,
          replicaCount: 1,
          environment,
        }, {
          runtimeOwner: fakeRuntimeOwner(environment),
          runCommand: async () => ({
            stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
            stderr: '',
          }),
          activate: async () => validActivationReceipt(environment),
          readHealth: async () => {
            calls.health += 1;
            return scenario.health;
          },
          readinessTimeoutMs: 0,
          loadWebSocket: async () => {
            calls.websocket += 1;
            return FakeWebSocket;
          },
        }),
        scenario.error,
      );
      assert.deepEqual(calls, { health: 1, websocket: 0 });
      assert.equal(FakeWebSocket.instances.length, 0);
    });
  }
});

test('control health timeout is bounded and never retries the business WebSocket', async () => {
  const environment = 'f27c-readiness-timeout';
  let healthCalls = 0;
  let websocketLoads = 0;
  FakeWebSocket.instances.length = 0;
  await assert.rejects(
    runPackageServiceEcosystemSmoke({
      checkout,
      replicaCount: 1,
      environment,
    }, {
      runtimeOwner: fakeRuntimeOwner(environment),
      runCommand: async () => ({
        stdout: JSON.stringify(validSmokeFixtureReceipt(environment)),
        stderr: '',
      }),
      activate: async () => validActivationReceipt(environment),
      readHealth: async (_url, signal) => {
        healthCalls += 1;
        return new Promise((_resolve, reject) => {
          signal.addEventListener('abort', () => reject(signal.reason), { once: true });
        });
      },
      readinessTimeoutMs: 5,
      loadWebSocket: async () => {
        websocketLoads += 1;
        return FakeWebSocket;
      },
    }),
    /timed out waiting for generation 1 assembly readiness: control health was not observed/,
  );
  assert.equal(healthCalls, 1);
  assert.equal(websocketLoads, 0);
  assert.equal(FakeWebSocket.instances.length, 0);
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

function fakeRuntimeOwner(
  environment,
  bootstrap = validBootstrapReceipt(environment),
) {
  return async ({ runTest, validateBootstrapReceipt }) => {
    validateBootstrapReceipt(bootstrap);
    return runTest(
      { SKIFF_TEST_ENVIRONMENT: environment },
      new AbortController().signal,
      {
        artifactRoot: '/isolated/artifacts',
        controlUrl: 'http://127.0.0.1:46001',
        routerHttpUrl: 'http://127.0.0.1:46000',
      },
    );
  };
}
