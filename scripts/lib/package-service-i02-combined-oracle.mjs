import assert from 'node:assert/strict';

const ASSEMBLY_IDENTITY =
  /^skiff-runtime-assembly-v2:sha256:[0-9a-f]{64}$/;
const PACKAGE_BUILD_IDENTITY =
  /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/;
const I02_SPAWN_SUBMIT_BUSINESS_RESULT =
  'P5-F45E-SPAWN-SUBMIT-TYPED-RESPONSE:submitted';

export function validateI02SpawnSubmitBusinessResult(result) {
  assert.equal(
    result,
    I02_SPAWN_SUBMIT_BUSINESS_RESULT,
    'I02 unary must continue only after the canonical typed spawn submit receipt',
  );
  return Object.freeze({
    businessResult: result,
    responseStatus: 'submitted',
  });
}

export function i02RuntimeAssemblyRecordPath(assembly) {
  assert.deepEqual(
    Object.keys(assembly ?? {}).sort(),
    ['assemblyIdentity'],
    'I02 requires an exact RuntimeAssembly reference',
  );
  assert.match(assembly.assemblyIdentity, ASSEMBLY_IDENTITY);
  return `records/runtime-assemblies/${identityHash(assembly.assemblyIdentity)}.json`;
}

export function selectI02TransitivePackageRecord({
  assemblyRecord,
  candidateReceipt,
  bootstrapReceipt,
}) {
  assert.equal(
    assemblyRecord?.assemblyIdentity,
    candidateReceipt?.candidate?.assembly?.assemblyIdentity,
    'I02 assembly record must match the authored candidate',
  );
  assert.ok(
    Array.isArray(assemblyRecord?.resolvedPackages),
    'I02 assembly record must expose resolvedPackages',
  );
  assert.ok(
    Array.isArray(assemblyRecord?.activationTemplates),
    'I02 assembly record must expose activationTemplates',
  );
  const transitive = bootstrapReceipt?.bootstrap?.std?.package?.artifact;
  assertPackageRef(transitive, 'I02 bootstrap transitive package');
  const matches = assemblyRecord.resolvedPackages.filter((reference) =>
    packageRefsEqual(reference, transitive));
  assert.equal(
    matches.length,
    1,
    'I02 candidate must contain the exact bootstrap package once in its package closure',
  );
  const directBuilds = new Set(
    assemblyRecord.activationTemplates.map((template) => {
      assert.match(
        template?.implementationPackageBuildId ?? '',
        PACKAGE_BUILD_IDENTITY,
        'I02 activation template must carry an implementationPackageBuildId',
      );
      return template.implementationPackageBuildId;
    }),
  );
  assert.equal(
    directBuilds.has(transitive.packageBuildId),
    false,
    'I02 tamper target must be transitive, not a direct activation implementation',
  );
  return Object.freeze({
    artifact: Object.freeze({ ...transitive }),
    relativePath: packageArtifactRecordPath(transitive),
  });
}

export function captureI02CommittedState(
  health,
  { environment, generation, assemblyIdentity, replicaId },
) {
  assert.equal(health?.ok, true, 'I02 control health must return ok:true');
  assert.equal(
    health.pendingActivation,
    null,
    'I02 control health must have no pending activation',
  );
  assert.deepEqual(
    {
      environment: health?.activeAssembly?.environment,
      generation: health?.activeAssembly?.generation,
      assemblyIdentity: health?.activeAssembly?.assemblyIdentity,
    },
    { environment, generation, assemblyIdentity },
    'I02 committed tuple must remain exact',
  );
  const replicas = (health.replicas ?? []).filter(
    (candidate) => candidate?.replicaId === replicaId,
  );
  assert.equal(replicas.length, 1, 'I02 must observe its exact runtime replica once');
  const [replica] = replicas;
  assert.equal(replica.environment, environment);
  assert.equal(replica.generation, generation);
  assert.equal(replica.assemblyIdentity, assemblyIdentity);
  assert.equal(replica.state, 'healthy');
  assert.equal(replica.connected, true);
  assert.equal(typeof replica.registeredAt, 'string');
  assert.ok(replica.registeredAt.length > 0);

  const capabilities = (health.capabilityConnections ?? []).filter(
    (candidate) => candidate?.runtimeId === replicaId,
  );
  assert.equal(
    capabilities.length,
    1,
    'I02 must observe its exact runtime capability connection once',
  );
  const [capability] = capabilities;
  assert.equal(capability.connected, true);
  assert.equal(typeof capability.registeredAt, 'string');
  assert.ok(capability.registeredAt.length > 0);
  assertPlainObject(capability.capabilities, 'I02 runtime capabilities');

  return Object.freeze({
    committedTuple: Object.freeze({ environment, generation, assemblyIdentity }),
    replica: Object.freeze({
      replicaId,
      connected: replica.connected,
      registeredAt: replica.registeredAt,
    }),
    capability: Object.freeze({
      runtimeId: capability.runtimeId,
      connected: capability.connected,
      registeredAt: capability.registeredAt,
      capabilities: deepFreeze(structuredClone(capability.capabilities)),
    }),
  });
}

export function assertI02CommittedStateUnchanged(before, after) {
  assert.deepEqual(
    after,
    before,
    'I02 rollback changed the committed tuple, replica, or capability connection',
  );
  return after;
}

export function classifyI02LoadReject(error, {
  activationId,
  expectedGeneration,
  assemblyIdentity,
}) {
  assert.ok(error instanceof Error, 'I02 rejected activation must return an Error');
  assert.match(
    error.message,
    /assembly activation rejected with HTTP 409/,
    'I02 candidate must be rejected by the real activation endpoint',
  );
  assert.match(
    error.message,
    /rejected activation during load/,
    'I02 transitive tamper must produce the typed load reject reason',
  );
  return Object.freeze({
    activationId,
    expectedGeneration,
    candidateGeneration: expectedGeneration + 1,
    assemblyIdentity,
    reason: 'load',
    stagePrepared: false,
    stagedAllocated: false,
  });
}

export function packageArtifactRecordPath(artifact) {
  assertPackageRef(artifact, 'I02 package artifact');
  return [
    'records',
    'package-artifacts',
    coordinateSegment(artifact.packageId),
    artifact.packageVersion,
    identityHash(artifact.packageBuildId),
    'package.json',
  ].join('/');
}

function assertPackageRef(value, label) {
  assertPlainObject(value, label);
  assert.equal(typeof value.packageId, 'string');
  assert.ok(value.packageId.length > 0);
  assert.equal(typeof value.packageVersion, 'string');
  assert.ok(value.packageVersion.length > 0);
  assert.match(value.packageBuildId ?? '', PACKAGE_BUILD_IDENTITY);
  assert.equal(typeof value.packageLocalAbiIdentity, 'string');
  assert.ok(value.packageLocalAbiIdentity.length > 0);
}

function packageRefsEqual(left, right) {
  return left?.packageId === right.packageId
    && left?.packageVersion === right.packageVersion
    && left?.packageBuildId === right.packageBuildId
    && left?.packageLocalAbiIdentity === right.packageLocalAbiIdentity;
}

function coordinateSegment(value) {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function identityHash(value) {
  return value.slice(value.lastIndexOf(':') + 1);
}

function assertPlainObject(value, label) {
  assert.ok(
    value !== null && typeof value === 'object' && !Array.isArray(value),
    `${label} must be an object`,
  );
}

function deepFreeze(value) {
  for (const child of Object.values(value)) {
    if (child !== null && typeof child === 'object') deepFreeze(child);
  }
  return Object.freeze(value);
}

export const packageServiceI02SpawnSubmitBusinessResult =
  I02_SPAWN_SUBMIT_BUSINESS_RESULT;
