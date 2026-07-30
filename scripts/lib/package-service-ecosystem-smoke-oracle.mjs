import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';

const FIXTURE_SCHEMA_VERSION = 'skiff-package-service-smoke-fixture-v3';
const BOOTSTRAP_SCHEMA_VERSION = 'skiff-package-service-bootstrap-v1';
const ENVIRONMENT_STATE_SCHEMA_VERSION = 'skiff-environment-activation-state-v1';
const PACKAGE_POINTER_SCHEMA_VERSION = 'skiff-package-artifact-pointer-v1';
const ACTIVATION_REQUEST_SCHEMA_VERSION = 'skiff-assembly-activation-request-v1';

const TEST_SERVICE_PACKAGE_ID = 'test.skiff/package-service-websocket-smoke';
const TEST_SERVICE_PACKAGE_VERSION = '1.0.0';
const STD_PACKAGE_ID = 'skiff.run/std';
const STD_PACKAGE_VERSION = '1.0.0';

const HASH = '[a-f0-9]{64}';
const ASSEMBLY_IDENTITY = new RegExp(`^skiff-runtime-assembly-v3:sha256:${HASH}$`);
const PACKAGE_BUILD_IDENTITY = new RegExp(`^skiff-package-build-v10:sha256:${HASH}$`);
const PACKAGE_ABI_IDENTITY = new RegExp(`^skiff-package-local-abi-v7:sha256:${HASH}$`);
const DEPLOYMENT_IDENTITY = new RegExp(`^skiff-deployment-artifact-v2:sha256:${HASH}$`);
const SERVICE_PROTOCOL_IDENTITY = new RegExp(`^skiff-service-protocol-v5:sha256:${HASH}$`);
const GATEWAY_IDENTITY = new RegExp(`^skiff-gateway-entry-v2:sha256:${HASH}$`);
const TEST_CASE_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305';
const SMOKE_PROBE_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705';

const READINESS_TIMEOUT_MS = 30_000;
const READINESS_INTERVAL_MS = 100;

export function readPackageServiceFixtureReceipt(
  stdout,
  expectedEnvironment,
  {
    packageId = TEST_SERVICE_PACKAGE_ID,
    packageVersion = TEST_SERVICE_PACKAGE_VERSION,
  } = {},
) {
  const receipt = parseJson(stdout, 'ecosystem smoke fixture');
  exactObject(receipt, ['bootstrap', 'candidate', 'environment', 'schemaVersion'], 'fixture receipt');
  assert.equal(receipt.schemaVersion, FIXTURE_SCHEMA_VERSION);
  assert.equal(receipt.environment, expectedEnvironment);
  assert.equal(receipt.bootstrap, null, 'candidate fixture must not reinitialize the environment');

  const candidate = exactObject(
    receipt.candidate,
    [
      'assembly',
      'contracts',
      'deployments',
      'entrypoints',
      'testService',
      'testServiceRecordPath',
    ],
    'fixture candidate',
  );
  runtimeAssemblyRef(candidate.assembly, 'fixture candidate assembly');
  const testService = packageArtifactRef(candidate.testService, 'fixture test service');
  assert.equal(testService.packageId, packageId);
  assert.equal(testService.packageVersion, packageVersion);
  assert.equal(
    candidate.testServiceRecordPath,
    packageRecordPath(testService),
    'testServiceRecordPath must select the exact ordinary test-service PackageArtifact',
  );
  assert.ok(Array.isArray(candidate.contracts), 'fixture candidate contracts must be an array');
  assert.equal(candidate.contracts.length, 1, 'single-case fixture must publish one contract');
  const contract = serviceContractRef(candidate.contracts[0], 'fixture test-service contract');
  assert.ok(Array.isArray(candidate.deployments), 'fixture candidate deployments must be an array');
  assert.equal(candidate.deployments.length, 1, 'single-case fixture must publish one deployment');
  const deployment = serviceDeploymentRef(
    candidate.deployments[0],
    'fixture test-service deployment',
  );
  assert.equal(contract.serviceId, testCaseServiceId(packageId, 0));
  assert.equal(contract.contractVersion, packageVersion);
  assert.equal(deployment.serviceId, contract.serviceId);
  assert.equal(deployment.contractVersion, contract.contractVersion);

  assert.ok(Array.isArray(candidate.entrypoints), 'fixture candidate entrypoints must be an array');
  assert.equal(candidate.entrypoints.length, 2, 'fixture candidate must publish exactly 2 HTTP entrypoints');
  const [testCase, unary] = candidate.entrypoints;
  httpEntrypoint(testCase, {
    key: 'run',
    identity: TEST_CASE_GATEWAY_IDENTITY,
    selector: {
      protocol: 'http',
      method: 'POST',
      path: '/__skiff/test/0',
    },
    deployment,
  });
  httpEntrypoint(unary, {
    key: 'probe',
    identity: SMOKE_PROBE_GATEWAY_IDENTITY,
    selector: {
      protocol: 'http',
      method: 'POST',
      path: '/probe',
    },
    deployment,
  });
  assert.notEqual(
    testCase.gatewayEntryIdentity,
    unary.gatewayEntryIdentity,
    'Null -> Null and Null -> String gateway surfaces must not share an identity',
  );

  return receipt;
}

export function validatePackageServiceBootstrapReceipt(receipt, expectedEnvironment) {
  exactObject(receipt, ['bootstrap', 'environment', 'schemaVersion'], 'bootstrap receipt');
  assert.equal(receipt.schemaVersion, BOOTSTRAP_SCHEMA_VERSION);
  assert.equal(receipt.environment, expectedEnvironment);
  const bootstrap = exactObject(
    receipt.bootstrap,
    ['assembly', 'generation', 'std'],
    'bootstrap payload',
  );
  runtimeAssemblyRef(bootstrap.assembly, 'bootstrap assembly');
  assert.equal(bootstrap.generation, 0, 'bootstrap must install generation 0');

  const std = exactObject(bootstrap.std, ['package', 'pointer', 'pointerPath'], 'bootstrap std');
  const packageReceipt = exactObject(
    std.package,
    ['artifact', 'fileIrRecordPaths', 'recordPath', 'resourceRecordPaths'],
    'bootstrap std package receipt',
  );
  const artifact = packageArtifactRef(packageReceipt.artifact, 'bootstrap std artifact');
  assert.equal(artifact.packageId, STD_PACKAGE_ID);
  assert.equal(artifact.packageVersion, STD_PACKAGE_VERSION);
  assert.equal(packageReceipt.recordPath, packageRecordPath(artifact));
  stringArray(packageReceipt.fileIrRecordPaths, 'bootstrap std fileIrRecordPaths', {
    nonempty: true,
  });
  stringArray(packageReceipt.resourceRecordPaths, 'bootstrap std resourceRecordPaths');
  const packageRecordRoot = packageReceipt.recordPath.slice(
    0,
    -'package.json'.length,
  );
  assert.ok(
    packageReceipt.fileIrRecordPaths.every((path) =>
      new RegExp(`^${escapeRegExp(packageRecordRoot)}file-ir/${HASH}\\.json$`).test(path)),
    'bootstrap std File IR records must stay under the exact package record root',
  );
  assert.ok(
    packageReceipt.resourceRecordPaths.every((path) =>
      new RegExp(`^${escapeRegExp(packageRecordRoot)}resources/${HASH}\\.blob$`).test(path)),
    'bootstrap std resource records must stay under the exact package record root',
  );

  const pointer = exactObject(
    std.pointer,
    ['artifact', 'recordPath', 'schemaVersion'],
    'bootstrap std pointer',
  );
  assert.equal(pointer.schemaVersion, PACKAGE_POINTER_SCHEMA_VERSION);
  assert.deepEqual(pointer.artifact, artifact);
  assert.equal(pointer.recordPath, packageReceipt.recordPath);
  assert.equal(
    std.pointerPath,
    `pointers/package-artifacts/${coordinateSegment(STD_PACKAGE_ID)}/${STD_PACKAGE_VERSION}.json`,
  );
  return receipt;
}

export function validatePackageServiceActivationReceipt(
  activation,
  { environment, assemblyIdentity, expectedGeneration = 0 },
) {
  exactObject(activation, ['request', 'response'], 'assembly activation receipt');
  const request = exactObject(
    activation.request,
    ['activationId', 'assembly', 'environment', 'expectedGeneration', 'schemaVersion'],
    'assembly activation request receipt',
  );
  assert.equal(request.schemaVersion, ACTIVATION_REQUEST_SCHEMA_VERSION);
  assert.equal(request.environment, environment);
  assert.equal(request.expectedGeneration, expectedGeneration);
  assert.equal(typeof request.activationId, 'string');
  assert.ok(request.activationId.length > 0);
  assert.equal(runtimeAssemblyRef(request.assembly, 'activation request assembly').assemblyIdentity,
    assemblyIdentity);

  const response = exactObject(
    activation.response,
    ['activeAssembly', 'committed', 'ok', 'replicas'],
    'assembly activation response',
  );
  assert.equal(response.ok, true);
  assert.ok(Array.isArray(response.replicas), 'activation response replicas must be an array');
  const committed = exactObject(
    response.committed,
    ['assembly', 'generation'],
    'activation committed tuple',
  );
  const committedGeneration = expectedGeneration + 1;
  assert.equal(committed.generation, committedGeneration);
  assert.equal(
    runtimeAssemblyRef(committed.assembly, 'activation committed assembly').assemblyIdentity,
    assemblyIdentity,
  );
  const active = exactObject(
    response.activeAssembly,
    ['assemblyIdentity', 'environment', 'generation'],
    'activation active tuple',
  );
  assert.deepEqual(active, {
    environment,
    generation: committedGeneration,
    assemblyIdentity,
  });
  return response;
}

export async function waitForPackageServiceAssemblyReady({
  healthUrl,
  environment,
  assemblyIdentity,
  generation = 1,
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
      const health = await withReadinessDeadline(
        (attemptSignal) => readHealth(healthUrl, attemptSignal),
        Math.max(0, deadline - now()),
        signal,
      );
      const readiness = packageServiceAssemblyReadiness(
        health,
        { environment, generation, assemblyIdentity },
      );
      if (readiness.ready) return readiness;
      lastReason = readiness.reason;
    } catch (error) {
      signal?.throwIfAborted();
      if (error instanceof ReadinessDeadlineError) {
        throw readinessTimeout(generation, lastReason);
      }
      lastReason = error?.message || String(error);
    }
    if (now() >= deadline) {
      throw readinessTimeout(generation, lastReason);
    }
    await withReadinessDeadline(
      (attemptSignal) =>
        sleep(Math.min(intervalMs, Math.max(0, deadline - now())), attemptSignal),
      Math.max(0, deadline - now()),
      signal,
    );
  }
}

export function packageServiceAssemblyReadiness(
  health,
  { environment, generation, assemblyIdentity },
) {
  if (!isPlainObject(health) || health.ok !== true) {
    return notReady('control health did not return ok:true');
  }
  const active = health.activeAssembly;
  if (!isPlainObject(active)
    || active.environment !== environment
    || active.generation !== generation
    || active.assemblyIdentity !== assemblyIdentity) {
    return notReady('active assembly tuple does not match the committed candidate');
  }
  if (health.pendingActivation !== null) {
    return notReady('an assembly activation is still pending');
  }
  if (!Array.isArray(health.replicas)) {
    return notReady('control health replicas is not an array');
  }
  const replicas = health.replicas.filter((candidate) =>
    isPlainObject(candidate)
    && typeof candidate.replicaId === 'string'
    && candidate.replicaId.length > 0
    && candidate.environment === environment
    && candidate.generation === generation
    && candidate.assemblyIdentity === assemblyIdentity
    && candidate.state === 'healthy'
    && candidate.connected === true);
  if (replicas.length === 0) {
    return notReady('no healthy connected replica matches the committed assembly tuple');
  }
  if (!Array.isArray(health.capabilityConnections)) {
    return notReady('control health capabilityConnections is not an array');
  }
  const replica = replicas.find((candidate) =>
    health.capabilityConnections.some((connection) =>
      isPlainObject(connection)
      && connection.runtimeId === candidate.replicaId
      && connection.connected === true
      && isPlainObject(connection.capabilities)));
  if (replica === undefined) {
    return notReady('no matching replica has its own connected capability');
  }
  return { ready: true, replicaId: replica.replicaId };
}

function httpEntrypoint(value, expected) {
  exactObject(
    value,
    ['deployment', 'gatewayEntryIdentity', 'gatewayEntryKey', 'mode', 'selector'],
    `${expected.key} HTTP entrypoint`,
  );
  assert.equal(value.gatewayEntryKey, expected.key);
  assert.match(value.gatewayEntryIdentity ?? '', GATEWAY_IDENTITY);
  assert.equal(value.gatewayEntryIdentity, expected.identity);
  assert.equal(value.mode, 'unary');
  exactObject(value.selector, ['method', 'path', 'protocol'], `${expected.key} selector`);
  assert.deepEqual(value.selector, expected.selector);
  const deployment = serviceDeploymentRef(value.deployment, `${expected.key} deployment`);
  assert.deepEqual(deployment, expected.deployment);
  return value;
}

function packageArtifactRef(value, label) {
  exactObject(
    value,
    ['packageBuildId', 'packageId', 'packageLocalAbiIdentity', 'packageVersion'],
    label,
  );
  assert.equal(typeof value.packageId, 'string');
  assert.equal(typeof value.packageVersion, 'string');
  assert.match(value.packageBuildId ?? '', PACKAGE_BUILD_IDENTITY);
  assert.match(value.packageLocalAbiIdentity ?? '', PACKAGE_ABI_IDENTITY);
  return value;
}

function serviceDeploymentRef(value, label) {
  exactObject(
    value,
    ['contractVersion', 'deploymentArtifactIdentity', 'deploymentRevision', 'serviceId'],
    label,
  );
  assert.equal(typeof value.serviceId, 'string');
  assert.equal(typeof value.contractVersion, 'string');
  assert.equal(typeof value.deploymentRevision, 'string');
  assert.match(value.deploymentArtifactIdentity ?? '', DEPLOYMENT_IDENTITY);
  return value;
}

function serviceContractRef(value, label) {
  exactObject(
    value,
    ['contractVersion', 'serviceId', 'serviceProtocolIdentity'],
    label,
  );
  assert.equal(typeof value.serviceId, 'string');
  assert.equal(typeof value.contractVersion, 'string');
  assert.match(value.serviceProtocolIdentity ?? '', SERVICE_PROTOCOL_IDENTITY);
  return value;
}

function runtimeAssemblyRef(value, label) {
  exactObject(value, ['assemblyIdentity'], label);
  assert.match(value.assemblyIdentity ?? '', ASSEMBLY_IDENTITY);
  return value;
}

function exactObject(value, keys, label) {
  assert.ok(isPlainObject(value), `${label} must be an object`);
  assert.deepEqual(Object.keys(value).sort(), [...keys].sort(), `${label} must have exact keys`);
  return value;
}

function stringArray(value, label, { nonempty = false } = {}) {
  assert.ok(Array.isArray(value), `${label} must be an array`);
  if (nonempty) assert.ok(value.length > 0, `${label} must not be empty`);
  assert.ok(value.every((item) => typeof item === 'string' && item.length > 0));
}

function packageRecordPath(artifact) {
  return [
    'records',
    'package-artifacts',
    coordinateSegment(artifact.packageId),
    artifact.packageVersion,
    identityHash(artifact.packageBuildId, PACKAGE_BUILD_IDENTITY),
    'package.json',
  ].join('/');
}

function coordinateSegment(value) {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function testCaseServiceId(packageId, caseIndex) {
  const digest = createHash('sha256').update(packageId).digest('hex').slice(0, 32);
  return `test.skiff/p-${digest}/case-${caseIndex}`;
}

function identityHash(value, pattern) {
  assert.match(value ?? '', pattern);
  return value.slice(value.lastIndexOf(':') + 1);
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function parseJson(value, label) {
  try {
    return JSON.parse(value);
  } catch (error) {
    throw new Error(`${label} returned invalid JSON: ${error.message}`);
  }
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function notReady(reason) {
  return { ready: false, reason };
}

class ReadinessDeadlineError extends Error {
  constructor() {
    super('assembly readiness deadline expired');
    this.name = 'ReadinessDeadlineError';
  }
}

async function withReadinessDeadline(operation, remainingMs, signal) {
  signal?.throwIfAborted();
  const controller = new AbortController();
  let onParentAbort;
  let timeout;
  const aborted = new Promise((_resolve, reject) => {
    const abort = (reason) => {
      if (controller.signal.aborted) return;
      controller.abort(reason);
      reject(reason);
    };
    onParentAbort = () => abort(signal.reason);
    signal?.addEventListener('abort', onParentAbort, { once: true });
    timeout = setTimeout(
      () => abort(new ReadinessDeadlineError()),
      remainingMs,
    );
  });
  try {
    return await Promise.race([
      Promise.resolve().then(() => operation(controller.signal)),
      aborted,
    ]);
  } finally {
    clearTimeout(timeout);
    if (onParentAbort !== undefined) {
      signal?.removeEventListener('abort', onParentAbort);
    }
  }
}

function readinessTimeout(generation, reason) {
  return new Error(
    `timed out waiting for generation ${generation} assembly readiness: ${reason}`,
  );
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

export const packageServiceEcosystemSmokeOracleConstants = Object.freeze({
  readinessTimeoutMs: READINESS_TIMEOUT_MS,
  readinessIntervalMs: READINESS_INTERVAL_MS,
});
