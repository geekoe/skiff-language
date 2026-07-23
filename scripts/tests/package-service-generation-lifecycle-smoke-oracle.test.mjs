import assert from 'node:assert/strict';
import test from 'node:test';

import {
  packageServiceGenerationState,
  validatePackageServiceGenerationUnaryResponse,
} from '../lib/package-service-generation-lifecycle-smoke-oracle.mjs';
import { encodeRuntimePayload } from '../lib/runtime-payload-codec.mjs';

const environment = 'r05-generation-oracle';
const assemblyIdentity =
  `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const expectedMarker = 'P5-R05-GENERATION-B-MARKER';
const stringSchema = Object.freeze({ type: 'string' });
const expectedState = Object.freeze({
  environment,
  generation: 2,
  assemblyIdentity,
  connectionPinCount: 1,
  inFlightCount: 0,
  connectionReleaseAckCount: 1,
});

test('generation unary oracle decodes canonical raw RuntimePayload string bytes', () => {
  const body = encodeRuntimePayload(expectedMarker, stringSchema);
  assert.equal(
    validatePackageServiceGenerationUnaryResponse({ status: 200, body }, expectedMarker),
    expectedMarker,
  );
});

test('generation unary oracle rejects JSON 200 as missing RuntimePayload magic', () => {
  assert.throws(
    () => validatePackageServiceGenerationUnaryResponse({
      status: 200,
      body: Buffer.from(JSON.stringify(expectedMarker)),
    }, expectedMarker),
    /runtime payload bytes missing SKPV magic/,
  );
});

test('generation unary oracle rejects a truncated raw RuntimePayload body', () => {
  const body = encodeRuntimePayload(expectedMarker, stringSchema);
  assert.throws(
    () => validatePackageServiceGenerationUnaryResponse({
      status: 200,
      body: body.subarray(0, body.byteLength - 1),
    }, expectedMarker),
    /runtime payload ended early/,
  );
});

test('generation unary oracle rejects a decoded marker mismatch', () => {
  assert.throws(
    () => validatePackageServiceGenerationUnaryResponse({
      status: 200,
      body: encodeRuntimePayload('wrong-generation', stringSchema),
    }, expectedMarker),
    /Expected values to be strictly equal/,
  );
});

test('generation state oracle requires the exact ACK, pin, in-flight, and pending tail', async (t) => {
  const canonical = lifecycleHealth();
  assert.deepEqual(packageServiceGenerationState(canonical, expectedState), {
    ready: true,
    replicaId: 'runtime-r05',
    connectionPinCount: 1,
    inFlightCount: 0,
    connectionReleaseAckCount: 1,
  });

  const cases = [
    ['ACK', (health) => {
      health.replicas[0].connectionReleaseAckCount = 0;
    }, /release ACK count 0 does not equal 1/],
    ['pin', (health) => {
      health.replicas[0].connectionPinCount = 2;
    }, /connection pin count 2 does not equal 1/],
    ['in-flight', (health) => {
      health.replicas[0].inFlightCount = 1;
    }, /in-flight count 1 does not equal 0/],
    ['pending', (health) => {
      health.pendingActivation = { generation: 3 };
    }, /activation is still pending/],
  ];

  for (const [name, mutate, reason] of cases) {
    await t.test(name, () => {
      const health = structuredClone(canonical);
      mutate(health);
      const observed = packageServiceGenerationState(health, expectedState);
      assert.equal(observed.ready, false);
      assert.match(observed.reason, reason);
    });
  }
});

function lifecycleHealth() {
  return {
    ok: true,
    activeAssembly: {
      environment,
      generation: 2,
      assemblyIdentity,
    },
    pendingActivation: null,
    capabilityConnections: [{
      runtimeId: 'runtime-r05',
      connected: true,
      capabilities: { runtimeProgram: true },
    }],
    replicas: [{
      replicaId: 'runtime-r05',
      environment,
      generation: 2,
      assemblyIdentity,
      state: 'healthy',
      connected: true,
      connectionPinCount: 1,
      inFlightCount: 0,
      connectionReleaseAckCount: 1,
    }],
  };
}
