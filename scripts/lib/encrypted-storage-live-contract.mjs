import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export const repoRoot = resolve(scriptDir, '..', '..');
export const ENCRYPTED_STORAGE_TARGET_ENVIRONMENT = 'dev';

const RUNTIME_ASSEMBLY_IDENTITY =
  /^skiff-runtime-assembly-v2:sha256:[0-9a-f]{64}$/;
const REQUIRED_PACKAGE_COORDINATES = new Set([
  'example.com/encrypted-live-default@0.1.0',
  'example.com/encrypted-live-mapped@0.1.0',
  'example.com/encrypted-live-store@1.0.0',
]);
const REQUIRED_SERVICE_IDS = new Set([
  'example.com/encrypted-live-default',
  'example.com/encrypted-live-mapped',
]);

export function encryptedStorageTestRunnerArgs({
  testFile,
  artifactRoot,
  baseAssembly,
  activationUrl,
  ingressUrl,
  environment,
  expectedGeneration,
}) {
  requiredAbsolutePath(testFile, 'encrypted-storage test file');
  requiredAbsolutePath(artifactRoot, 'encrypted-storage artifact root');
  if (!RUNTIME_ASSEMBLY_IDENTITY.test(baseAssembly ?? '')) {
    throw new Error('encrypted-storage base assembly must be canonical');
  }
  requiredActivationUrl(activationUrl);
  requiredIngressUrl(ingressUrl);
  if (environment !== ENCRYPTED_STORAGE_TARGET_ENVIRONMENT) {
    throw new Error(
      `encrypted-storage target environment must be ${ENCRYPTED_STORAGE_TARGET_ENVIRONMENT}`,
    );
  }
  requiredGeneration(expectedGeneration, 'encrypted-storage expected generation');
  return [
    'run',
    '--locked',
    '--quiet',
    '--manifest-path',
    'test-runner/Cargo.toml',
    '--bin',
    'skiff-test-runner',
    '--',
    testFile,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    repoRoot,
    '--base-assembly',
    baseAssembly,
    '--live',
    '--activation-url',
    activationUrl,
    '--ingress-url',
    ingressUrl,
    '--environment',
    environment,
    '--expected-generation',
    String(expectedGeneration),
    '--deny-skips',
    '--require-tests',
  ];
}

export function encryptedStorageBuildArgs({
  fixtureRoot,
  artifactRoot,
}) {
  requiredAbsolutePath(fixtureRoot, 'encrypted-storage fixture root');
  requiredAbsolutePath(artifactRoot, 'encrypted-storage artifact root');
  return [
    'scripts/skiff-dev-sync.mjs',
    '--root',
    join(
      fixtureRoot,
      'package-store',
      'example~com~~encrypted-live-store',
      '1.0.0',
    ),
    '--root',
    join(fixtureRoot, 'default-service'),
    '--root',
    join(fixtureRoot, 'mapped-service'),
    '--artifact-root',
    artifactRoot,
    '--environment',
    ENCRYPTED_STORAGE_TARGET_ENVIRONMENT,
    '--build-only',
    '--json',
  ];
}

export function encryptedStorageProductionAssembly(receipt) {
  if (!isPlainObject(receipt?.runtimeAssemblyReceipt)) {
    throw new Error('runtime assembly receipt is missing');
  }
  const { runtimeAssemblyReceipt } = receipt;
  if (
    runtimeAssemblyReceipt.environment !== ENCRYPTED_STORAGE_TARGET_ENVIRONMENT
  ) {
    throw new Error(
      `runtime assembly receipt environment must be ${ENCRYPTED_STORAGE_TARGET_ENVIRONMENT}`,
    );
  }
  const assembly = runtimeAssemblyReceipt.assembly;
  if (!isPlainObject(assembly) || assembly.assemblyIdentity === undefined) {
    throw new Error('assembly identity is missing');
  }
  if (
    Object.keys(assembly).length !== 1
    || !RUNTIME_ASSEMBLY_IDENTITY.test(assembly.assemblyIdentity)
  ) {
    throw new Error('assembly identity is not canonical');
  }
  const packageCoordinates = exactStringSet(
    receipt.packageArtifactReceipts,
    (entry) => {
      const artifact = entry?.artifact;
      return typeof artifact?.packageId === 'string'
        && typeof artifact?.packageVersion === 'string'
        ? `${artifact.packageId}@${artifact.packageVersion}`
        : undefined;
    },
  );
  if (!setsEqual(packageCoordinates, REQUIRED_PACKAGE_COORDINATES)) {
    throw new Error('required package roots are incomplete');
  }
  const serviceIds = exactStringSet(
    receipt.serviceDeploymentReceipts,
    (entry) => entry?.deployment?.serviceId,
  );
  if (!setsEqual(serviceIds, REQUIRED_SERVICE_IDS)) {
    throw new Error('required service roots are incomplete');
  }
  return Object.freeze({ assemblyIdentity: assembly.assemblyIdentity });
}

export function encryptedStorageIngressRequest({
  ingressUrl,
  path,
  body,
  rotationToken,
}) {
  const ingress = requiredIngressUrl(ingressUrl);
  if (
    typeof path !== 'string'
    || !path.startsWith('/')
    || path.startsWith('//')
    || path.includes('?')
    || path.includes('#')
  ) {
    throw new Error('encrypted-storage ingress path must come from the manifest');
  }
  const url = new URL(path, ingress);
  return {
    url,
    options: {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(rotationToken === undefined
          ? {}
          : { 'x-skiff-rotation-token': rotationToken }),
      },
      body: JSON.stringify(body),
    },
  };
}

export async function runEncryptedStorageTestLifecycle({
  activationState,
  runTest,
  observeStorage,
  cleanupStorage,
  restoreProductionAssembly,
  readCommittedGeneration,
}) {
  assertActivationState(activationState);
  for (const [name, operation] of Object.entries({
    runTest,
    observeStorage,
    cleanupStorage,
    restoreProductionAssembly,
  })) {
    if (typeof operation !== 'function') {
      throw new Error(`encrypted-storage lifecycle requires ${name}`);
    }
  }
  const expectedGeneration = activationState.currentGeneration;
  const baseAssembly = activationState.productionAssembly.assemblyIdentity;
  const [testOutcome, observationOutcome] = await Promise.allSettled([
    Promise.resolve().then(() => runTest({ baseAssembly, expectedGeneration })),
    Promise.resolve().then(() => observeStorage()),
  ]);
  const failures = [];
  let testActivationCommitted = false;
  if (testOutcome.status === 'fulfilled') {
    activationState.currentGeneration += 1;
    testActivationCommitted = true;
  } else {
    failures.push(contextualError('test runner failed', testOutcome.reason));
    if (typeof readCommittedGeneration === 'function') {
      try {
        const committedGeneration = await readCommittedGeneration();
        requiredGeneration(
          committedGeneration,
          'encrypted-storage observed committed generation',
        );
        if (committedGeneration === expectedGeneration + 1) {
          activationState.currentGeneration = committedGeneration;
          testActivationCommitted = true;
        } else if (committedGeneration !== expectedGeneration) {
          failures.push(new Error(
            `encrypted-storage test generation is indeterminate: expected ${expectedGeneration} or ${expectedGeneration + 1}, observed ${committedGeneration}`,
          ));
        }
      } catch (error) {
        failures.push(contextualError(
          'failed to determine whether test activation committed',
          error,
        ));
      }
    }
  }

  let storage;
  if (observationOutcome.status === 'fulfilled') {
    try {
      storage = await cleanupStorage(observationOutcome.value);
    } catch (error) {
      failures.push(contextualError('transient storage cleanup failed', error));
    }
  } else {
    failures.push(contextualError(
      'transient storage observation failed',
      observationOutcome.reason,
    ));
  }

  if (testActivationCommitted) {
    try {
      await restoreProductionAssembly({
        assembly: activationState.productionAssembly,
        expectedGeneration: activationState.currentGeneration,
      });
      activationState.currentGeneration += 1;
    } catch (error) {
      failures.push(contextualError('production restore failed', error));
    }
  }
  if (failures.length > 0) {
    throw new AggregateError(
      failures,
      `encrypted-storage live test lifecycle failed: ${failures.map((error) => error.message).join('; ')}`,
    );
  }
  return { storage, currentGeneration: activationState.currentGeneration };
}

function requiredAbsolutePath(value, label) {
  if (typeof value !== 'string' || !isAbsolute(value)) {
    throw new Error(`${label} must be an absolute path`);
  }
}

function requiredUrl(value, label) {
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${label} must be an absolute URL`);
  }
  if (parsed.protocol !== 'http:' || parsed.username || parsed.password) {
    throw new Error(`${label} must be an unauthenticated http URL`);
  }
  return parsed;
}

function requiredActivationUrl(value) {
  const parsed = requiredUrl(value, 'encrypted-storage activation URL');
  if (
    parsed.pathname !== '/__skiff/activate-assembly'
    || parsed.search
    || parsed.hash
  ) {
    throw new Error(
      'encrypted-storage activation URL must point exactly to /__skiff/activate-assembly',
    );
  }
  return parsed;
}

function requiredIngressUrl(value) {
  const parsed = requiredUrl(value, 'encrypted-storage ingress URL');
  if (parsed.pathname !== '/' || parsed.search || parsed.hash) {
    throw new Error('encrypted-storage ingress URL must be an origin');
  }
  return parsed;
}

function requiredGeneration(value, label) {
  if (
    !Number.isSafeInteger(value)
    || Object.is(value, -0)
    || value < 0
    || value > Number.MAX_SAFE_INTEGER - 2
  ) {
    throw new Error(`${label} must be a non-negative safe generation`);
  }
}

function exactStringSet(values, select) {
  if (!Array.isArray(values)) {
    return undefined;
  }
  const result = new Set();
  for (const value of values) {
    const selected = select(value);
    if (
      typeof selected !== 'string'
      || selected.length === 0
      || result.has(selected)
    ) {
      return undefined;
    }
    result.add(selected);
  }
  return result;
}

function setsEqual(left, right) {
  return left instanceof Set
    && left.size === right.size
    && [...left].every((value) => right.has(value));
}

function assertActivationState(state) {
  if (!isPlainObject(state)) {
    throw new Error('encrypted-storage lifecycle requires caller-owned activation state');
  }
  requiredGeneration(
    state.currentGeneration,
    'encrypted-storage current generation',
  );
  const assembly = state.productionAssembly;
  if (
    !isPlainObject(assembly)
    || Object.keys(assembly).length !== 1
    || !RUNTIME_ASSEMBLY_IDENTITY.test(assembly.assemblyIdentity ?? '')
  ) {
    throw new Error(
      'encrypted-storage lifecycle requires a canonical production assembly',
    );
  }
}

function contextualError(label, error) {
  const cause = error instanceof Error ? error : new Error(String(error));
  return new Error(`${label}: ${cause.message}`, { cause });
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
