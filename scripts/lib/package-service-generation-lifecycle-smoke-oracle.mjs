import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';

import {
  packageServiceAssemblyReadiness,
  readPackageServiceFixtureReceipt,
} from './package-service-ecosystem-smoke-oracle.mjs';

const PACKAGE_ID = 'test.skiff/package-service-websocket-smoke';
const PACKAGE_VERSION = '1.0.0';
const PACKAGE_TEST_NAME = 'generation lifecycle source fixture compiles';
const READINESS_TIMEOUT_MS = 30_000;
const READINESS_INTERVAL_MS = 100;

export function readPackageServiceGenerationFixtureReceipt(
  stdout,
  expectedEnvironment,
) {
  return readPackageServiceFixtureReceipt(stdout, expectedEnvironment, {
    packageId: PACKAGE_ID,
    packageVersion: PACKAGE_VERSION,
    packageTestName: PACKAGE_TEST_NAME,
  });
}

export function validatePackageServiceGenerationFixturePair(receiptA, receiptB) {
  const candidateA = generationCandidate(receiptA, 'A');
  const candidateB = generationCandidate(receiptB, 'B');

  assert.notEqual(
    candidateA.packageBuildId,
    candidateB.packageBuildId,
    'generation fixtures must produce distinct PackageBuildId values',
  );
  assert.notEqual(
    candidateA.deploymentRevision,
    candidateB.deploymentRevision,
    'generation fixtures must produce distinct deployment revisions',
  );
  assert.notEqual(
    candidateA.assemblyIdentity,
    candidateB.assemblyIdentity,
    'generation fixtures must produce distinct assembly identities',
  );
  assert.equal(
    candidateA.serviceProtocolIdentity,
    candidateB.serviceProtocolIdentity,
    'generation fixtures must preserve service protocol identity',
  );
  assert.deepEqual(
    candidateA.operationIdentities,
    candidateB.operationIdentities,
    'generation fixtures must preserve operation identities',
  );
  return { A: candidateA, B: candidateB };
}

export async function waitForPackageServiceGenerationState({
  healthUrl,
  environment,
  generation,
  assemblyIdentity,
  connectionPinCount,
  inFlightCount,
  signal,
  readHealth = readControlHealth,
  now = Date.now,
  sleep = defaultSleep,
  timeoutMs = READINESS_TIMEOUT_MS,
  intervalMs = READINESS_INTERVAL_MS,
}) {
  assert.ok(Number.isSafeInteger(timeoutMs) && timeoutMs >= 0);
  assert.ok(Number.isSafeInteger(intervalMs) && intervalMs >= 0);
  const deadline = now() + timeoutMs;
  let lastReason = 'control health was not observed';
  for (;;) {
    signal?.throwIfAborted();
    try {
      const health = await readHealth(healthUrl, signal);
      const observed = packageServiceGenerationState(health, {
        environment,
        generation,
        assemblyIdentity,
        connectionPinCount,
        inFlightCount,
      });
      if (observed.ready) return observed;
      lastReason = observed.reason;
    } catch (error) {
      signal?.throwIfAborted();
      lastReason = error?.message || String(error);
    }
    if (now() >= deadline) {
      throw new Error(
        `timed out waiting for generation ${generation} lifecycle state: ${lastReason}`,
      );
    }
    await sleep(
      Math.min(intervalMs, Math.max(0, deadline - now())),
      signal,
    );
  }
}

export function packageServiceGenerationState(health, expected) {
  const readiness = packageServiceAssemblyReadiness(health, expected);
  if (!readiness.ready) return readiness;
  const connectionPinCount = sumReplicaCounter(health.replicas, 'connectionPinCount');
  const inFlightCount = sumReplicaCounter(health.replicas, 'inFlightCount');
  if (connectionPinCount !== expected.connectionPinCount) {
    return {
      ready: false,
      reason:
        `connection pin count ${connectionPinCount} does not equal ${expected.connectionPinCount}`,
    };
  }
  if (inFlightCount !== expected.inFlightCount) {
    return {
      ready: false,
      reason: `in-flight count ${inFlightCount} does not equal ${expected.inFlightCount}`,
    };
  }
  return {
    ready: true,
    replicaId: readiness.replicaId,
    connectionPinCount,
    inFlightCount,
  };
}

export function validatePackageServiceGenerationUnaryResponse(
  response,
  expectedMarker,
) {
  assert.equal(response.status, 200, 'generation B unary request must return HTTP 200');
  let value;
  try {
    value = JSON.parse(response.body);
  } catch (error) {
    throw new Error(`generation B unary response returned invalid JSON: ${error.message}`);
  }
  assert.equal(value, expectedMarker);
  return value;
}

function generationCandidate(receipt, label) {
  const [packageTest, unary, websocket] = receipt.candidate.entrypoints;
  assert.equal(packageTest.name, PACKAGE_TEST_NAME);
  return Object.freeze({
    label,
    packageBuildId: receipt.candidate.production.packageBuildId,
    packageRecordPath: packageRecordPath(receipt.candidate.production),
    deploymentRevision: unary.deployment.deploymentRevision,
    deploymentIdentity: unary.deployment.deploymentArtifactIdentity,
    assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
    serviceProtocolIdentity: unary.contract.serviceProtocolIdentity,
    operationIdentities: Object.freeze({
      unary: unary.operation,
      websocket: websocket.operation,
    }),
  });
}

function packageRecordPath(artifact) {
  return [
    'records',
    'package-artifacts',
    coordinateSegment(artifact.packageId),
    artifact.packageVersion,
    identityHash(artifact.packageBuildId),
    'package.json',
  ].join('/');
}

function coordinateSegment(value) {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function identityHash(value) {
  return value.slice(value.lastIndexOf(':') + 1);
}

function sumReplicaCounter(replicas, key) {
  let total = 0;
  for (const replica of replicas) {
    if (!Number.isSafeInteger(replica?.[key]) || replica[key] < 0) {
      return Number.NaN;
    }
    total += replica[key];
  }
  return total;
}

async function readControlHealth(url, signal) {
  const response = await fetch(url, { signal });
  if (!response.ok) {
    throw new Error(`control health returned HTTP ${response.status}`);
  }
  return response.json();
}

function defaultSleep(milliseconds, signal) {
  return delay(milliseconds, undefined, { signal });
}

export const packageServiceGenerationLifecycleOracleConstants = Object.freeze({
  packageId: PACKAGE_ID,
  packageVersion: PACKAGE_VERSION,
  packageTestName: PACKAGE_TEST_NAME,
});
