import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

const FIXTURE_SCHEMA_VERSION = 'skiff-package-service-smoke-fixture-v4';
const BOOTSTRAP_SCHEMA_VERSION = 'skiff-package-service-bootstrap-v2';
const PACKAGE_POINTER_SCHEMA_VERSION = 'skiff-package-artifact-pointer-v1';

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
const CONFIG_SNAPSHOT_IDENTITY =
  /^skiff-runtime-config-snapshot-v1:[a-f0-9]{32}$/;
const TEST_CASE_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305';
const SMOKE_PROBE_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705';

export function readPackageServiceFixtureReceipt(
  stdout,
  expectedProfile,
  {
    packageId = TEST_SERVICE_PACKAGE_ID,
    packageVersion = TEST_SERVICE_PACKAGE_VERSION,
  } = {},
) {
  const receipt = parseJson(stdout, 'ecosystem smoke fixture');
  exactObject(receipt, ['bootstrap', 'candidate', 'profile', 'schemaVersion'], 'fixture receipt');
  assert.equal(receipt.schemaVersion, FIXTURE_SCHEMA_VERSION);
  assert.equal(receipt.profile, expectedProfile);
  assert.equal(receipt.bootstrap, null, 'candidate fixture must not reinitialize the profile');

  const candidate = exactObject(
    receipt.candidate,
    [
      'assembly',
      'configSnapshot',
      'contracts',
      'deployments',
      'entrypoints',
      'testService',
      'testServiceRecordPath',
    ],
    'fixture candidate',
  );
  runtimeAssemblyRef(candidate.assembly, 'fixture candidate assembly');
  runtimeConfigSnapshotRef(
    candidate.configSnapshot,
    'fixture candidate config snapshot',
  );
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
  assertTestCaseServiceId(contract.serviceId, packageId, 0);
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

export function validatePackageServiceBootstrapReceipt(receipt, expectedProfile) {
  exactObject(receipt, ['bootstrap', 'profile', 'schemaVersion'], 'bootstrap receipt');
  assert.equal(receipt.schemaVersion, BOOTSTRAP_SCHEMA_VERSION);
  assert.equal(receipt.profile, expectedProfile);
  const bootstrap = exactObject(
    receipt.bootstrap,
    ['assembly', 'configSnapshot', 'generation', 'std'],
    'bootstrap payload',
  );
  runtimeAssemblyRef(bootstrap.assembly, 'bootstrap assembly');
  runtimeConfigSnapshotRef(
    bootstrap.configSnapshot,
    'bootstrap config snapshot',
  );
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

function runtimeConfigSnapshotRef(value, label) {
  exactObject(value, ['snapshotId'], label);
  assert.match(value.snapshotId ?? '', CONFIG_SNAPSHOT_IDENTITY);
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

function assertTestCaseServiceId(value, packageId, caseIndex) {
  const packageDigest =
    createHash('sha256').update(packageId).digest('hex').slice(0, 16);
  assert.match(
    value,
    new RegExp(
      `^test\\.skiff/p-${packageDigest}/e-[a-f0-9]{16}/case-${caseIndex}$`,
    ),
    'test service identity must isolate one package and one test execution',
  );
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
