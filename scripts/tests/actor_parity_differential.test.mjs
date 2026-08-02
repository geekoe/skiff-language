import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  projectActorParityFrameEvents,
} from '../lib/router-differential/actor_parity_driver.mjs';
import {
  ACTOR_PARITY_BASELINE,
} from '../lib/router-differential/actor_parity_constants.mjs';
import {
  assertActorParityScenarioRunnable,
  loadActorParityInventory,
} from '../lib/router-differential/actor_parity_scenarios.mjs';
import {
  canonicalJsonBytes,
  synthesizeActorRoutingProjection,
} from '../lib/router-differential/actor_parity_projection.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('checked-in actor parity inventory is complete and consistent', async () => {
  const inventory = await loadActorParityInventory({ skiffRoot: repoRoot });
  assert.equal(inventory.schemaVersion, 'skiff-router-differential-inventory-v1');
  assert.equal(inventory.baseline, ACTOR_PARITY_BASELINE);
  assert.ok(inventory.scenarios.length >= 1);
  const ids = new Set(inventory.scenarios.map((scenario) => scenario.id));
  assert.equal(ids.size, inventory.scenarios.length);

  const runnable = inventory.scenarios.filter((scenario) => scenario.status === 'runnable');
  assert.deepEqual(
    runnable.map((scenario) => scenario.id),
    ['actor_parity_full_chain'],
  );
  const scenario = runnable[0];
  assert.equal(scenario.lane, 'actor');
  assert.ok(scenario.description.includes('TS and Rust'));
  assert.deepEqual(scenario.normalizations, []);
  assert.ok(Array.isArray(scenario.knownDifferences));
  assert.ok(scenario.knownDifferences.length >= 2);
  assert.ok(
    scenario.knownDifferences.every((difference) => difference.accepted === true),
  );
  assert.ok(
    scenario.knownDifferences.some(
      (difference) => difference.id === 'flaky-retained-entry-failure-stage',
    ),
  );
  assert.ok(
    scenario.knownDifferences.some(
      (difference) => difference.id === 'rejected-activation-error-vocabulary',
    ),
  );
  assert.ok(
    (scenario.nonBlockingFollowUps ?? []).some(
      (followUp) => followUp.id === 'async-frame-interleaving-order',
    ),
  );
  assert.ok(scenario.compare.equal.some((entry) => entry.path === 'http.steps'));
  assert.ok(scenario.compare.equal.some((entry) =>
    entry.path === 'frameEvents.actor-parity-replica-1'));
  assert.doesNotThrow(() => assertActorParityScenarioRunnable(scenario));
});

test('actor parity frame projection drops handshake frames and tokenizes ephemeral ids per key', () => {
  const events = projectActorParityFrameEvents([
    {
      replica: 'actor-parity-replica-1',
      records: [
        { direction: 'ToRouter', type: 'router.bootstrap', header: { type: 'router.bootstrap' }, payloadSha256: 'a' },
        {
          direction: 'ToRouter',
          type: 'actor.getOrCreate.request',
          header: {
            type: 'actor.getOrCreate.request',
            schemaVersion: 'skiff-runtime-frame-v3',
            rpcId: 'rpc-ts-1',
            serviceId: 'test.skiff/router-rust-actor-live',
            actorAbiIdentity: 'skiff-actor-abi-v1:sha256:abc',
          },
          payloadSha256: 'abc123',
        },
        {
          direction: 'ToRouter',
          type: 'actor.getOrCreate.request',
          header: {
            type: 'actor.getOrCreate.request',
            schemaVersion: 'skiff-runtime-frame-v3',
            rpcId: 'rpc-ts-2',
            serviceId: 'test.skiff/router-rust-actor-live',
            actorAbiIdentity: 'skiff-actor-abi-v1:sha256:abc',
          },
          payloadSha256: 'abc123',
        },
      ],
    },
  ]);
  assert.deepEqual(events['actor-parity-replica-1'].map((event) => event.type), [
    'actor.getOrCreate.request',
    'actor.getOrCreate.request',
  ]);
  const [first, second] = events['actor-parity-replica-1'];
  assert.equal(first.fields.rpcId, '<rpcId-1>');
  assert.equal(second.fields.rpcId, '<rpcId-2>');
  assert.equal(first.fields.serviceId, 'test.skiff/router-rust-actor-live');
  assert.equal(first.fields.schemaVersion, undefined);
  assert.equal(first.payloadSha256, 'abc123');
});

test('actor parity request.start projection keeps semantic routing fields only', () => {
  const events = projectActorParityFrameEvents([
    {
      replica: 'actor-parity-replica-1',
      records: [
        {
          direction: 'ToRuntime',
          type: 'request.start',
          header: {
            type: 'request.start',
            schemaVersion: 'skiff-runtime-frame-v3',
            requestId: 'request-1',
            mode: 'unary',
            caller: { kind: 'gateway' },
            routing: {
              kind: 'runtimeAssembly',
              assemblyIdentity: 'assembly-1',
              assemblyGeneration: 1,
              deployment: { serviceId: 'svc', contractVersion: '1.0.0' },
              gatewayEntryIdentity: 'gateway-1',
              ingress: { protocol: 'http', method: 'POST', path: '/probe' },
            },
            deadline: { timeoutMs: 30000, expiresAt: '2099-01-01T00:00:00.000Z' },
            trace: { traceId: 'trace-1', spanId: 'span-1' },
            httpRequest: {
              method: 'POST',
              path: '/probe',
              url: 'http://127.0.0.1:45000/probe',
            },
          },
          payloadSha256: 'sha',
        },
      ],
    },
  ]);
  const [event] = events['actor-parity-replica-1'];
  assert.deepEqual(event.fields, {
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: 'assembly-1',
      assemblyGeneration: 1,
      deployment: { serviceId: 'svc', contractVersion: '1.0.0' },
      gatewayEntryIdentity: 'gateway-1',
      ingress: { protocol: 'http', method: 'POST', path: '/probe' },
    },
    requestId: '<requestId-token>',
    deadline: { timeoutMs: 30000 },
    httpRequest: { method: 'POST', path: '/probe' },
  });
});

test('canonical JSON encoder matches the frozen serde_json output shape', () => {
  const bytes = canonicalJsonBytes({
    z: [1, 2, 3],
    a: { nested: 'x"y\\z\n\u0001', n: 1 },
    flag: true,
    nothing: null,
  });
  assert.equal(
    bytes.toString('utf8'),
    '{"a":{"n":1,"nested":"x\\"y\\\\z\\n\\u0001"},"flag":true,"nothing":null,"z":[1,2,3]}',
  );
});

test('actor parity projection synthesizer writes sorted canonical methods', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-actor-parity-projection-'));
  try {
    const packageId = 'test.skiff/actors';
    const packageBuildId = 'skiff-package-build-v10:sha256:' + 'a'.repeat(64);
    const fileIrIdentity = 'skiff-file-ir-v11:sha256:' + 'b'.repeat(64);
    const assemblyIdentity = 'skiff-runtime-assembly-v3:sha256:' + 'c'.repeat(64);
    const deployment = {
      serviceId: packageId,
      contractVersion: '1.0.0',
      deploymentRevision: 'rev-1',
      deploymentArtifactIdentity: 'skiff-deployment-artifact-v4:sha256:' + 'd'.repeat(64),
    };
    const packageRef = {
      packageId,
      packageVersion: '1.0.0',
      packageBuildId,
      packageLocalAbiIdentity: 'skiff-package-local-abi-v7:sha256:' + 'e'.repeat(64),
    };
    const assembly = {
      assemblyIdentity,
      activationTemplates: [
        { deployment, implementationPackageBuildId: packageBuildId },
      ],
      packageLinkPlan: {
        codeSlots: [{ package: packageRef }],
      },
    };
    const packageValue = {
      packageId,
      packageVersion: '1.0.0',
      packageBuildId,
      packageLocalAbi: {},
      files: [{ fileIrIdentity }],
    };
    const fileValue = {
      fileIrIdentity,
      actorDeclarations: [
        {
          actorAbiIdentity: 'skiff-actor-abi-v1:sha256:' + 'f'.repeat(64),
          actorImplementationIdentity: 'skiff-actor-implementation-v1:sha256:' + '0'.repeat(64),
          methodImplementations: {
            ['skiff-actor-method-v1:sha256:' + '1'.repeat(64)]: 1,
            ['skiff-actor-method-v1:sha256:' + '2'.repeat(64)]: 2,
          },
        },
      ],
    };
    const encoded = packageId.replaceAll('.', '~d').replaceAll('/', '~s');
    await mkdir(
      join(
        root,
        'records',
        'package-artifacts',
        encoded,
        '1.0.0',
        'a'.repeat(64),
        'file-ir',
      ),
      { recursive: true },
    );
    await mkdir(join(root, 'records', 'runtime-assemblies'), { recursive: true });
    await writeFile(
      join(root, 'records', 'runtime-assemblies', 'c'.repeat(64) + '.json'),
      JSON.stringify(assembly),
    );
    await writeFile(
      join(
        root,
        'records',
        'package-artifacts',
        encoded,
        '1.0.0',
        'a'.repeat(64),
        'package.json',
      ),
      JSON.stringify(packageValue),
    );
    await writeFile(
      join(
        root,
        'records',
        'package-artifacts',
        encoded,
        '1.0.0',
        'a'.repeat(64),
        'file-ir',
        'b'.repeat(64) + '.json',
      ),
      JSON.stringify(fileValue),
    );

    const projection = await synthesizeActorRoutingProjection({
      artifactRoot: root,
      deploymentRecord: { deployment },
    });
    assert.equal(projection.schemaVersion, 'skiff-actor-routing-projection-v1');
    assert.equal(projection.methods.length, 2);
    assert.deepEqual(
      projection.methods.map((method) => method.methodIdentity.slice(-64)),
      ['1'.repeat(64), '2'.repeat(64)],
    );
    assert.deepEqual(projection.methods[0].package, packageRef);
    assert.deepEqual(projection.methods[0].deployment, deployment);

    const record = JSON.parse(
      await (await import('node:fs/promises')).readFile(
        join(root, 'records', 'actor-routing', 'current.json'),
        'utf8',
      ),
    );
    assert.deepEqual(record, projection);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
