import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';

import {
  deployCommandUsage,
  deploymentRecordExists,
  healthUrl,
  normalizeControlUrl,
  parseDeployArgs,
  pollHealthForBuildId,
  readHealth,
  readRollbackRecord,
  renderDeployReceipt,
  renderRollbackReceipt,
  renderVerifyReceipt,
  rollbackRecordRelativePath,
  runDeployCommand,
  validateProfileSegment,
  writeRollbackRecord,
} from '../lib/deploy-command.mjs';

const skiffRoot = '/workspace/skiff';
const identityPrefix = 'skiff-deployment-artifact-v4:sha256:';
const buildIdA = `${identityPrefix}${'a'.repeat(64)}`;
const buildIdB = `${identityPrefix}${'b'.repeat(64)}`;
const serviceId = 'example.echo';
const version = '1.0.0';
const profile = 'dev';
const pointerPath = 'pointers/releases/dev/example~decho/1.0.0.json';
const rollbackRecordPath = 'pointers/rollback/dev/example~decho/1.0.0.json';

function hexOf(buildId) {
  return buildId.slice(identityPrefix.length);
}

function pointerJson(buildId, revision = 'revision-1') {
  return JSON.stringify({
    schemaVersion: 'skiff-release-pointer-v1',
    profile,
    deployment: {
      serviceId,
      contractVersion: version,
      deploymentRevision: revision,
      deploymentArtifactIdentity: buildId,
    },
    recordPath: `records/service-deployments/example~decho/${version}/${revision}/${hexOf(buildId)}.json`,
  });
}

function pointer(buildId, revision = 'revision-1') {
  return JSON.parse(pointerJson(buildId, revision));
}

function publishReceipt(buildId, revision = 'revision-1') {
  return {
    serviceApiReceipt: { serviceId, projection: { functions: [] } },
    serviceDeploymentReceipt: {
      deployment: pointer(buildId, revision).deployment,
      recordPath: `records/service-deployments/example~decho/${version}/${revision}/${hexOf(buildId)}.json`,
    },
    releasePointerReceipt: {
      pointer: pointer(buildId, revision),
      pointerPath,
    },
  };
}

function okOutcome(receipt) {
  return { error: null, signal: null, code: 0, stdout: JSON.stringify(receipt), stderr: '' };
}

function fakeCompiler({ initialPointerJson = null, onGet } = {}) {
  let current = initialPointerJson;
  const setCalls = [];
  const getCalls = [];
  const runCompiler = async (command, args) => {
    const action = args[args.indexOf('release') + 1];
    if (action === 'get') {
      getCalls.push({});
      const snapshot = current;
      onGet?.();
      return okOutcome({
        action: 'get',
        profile,
        serviceId,
        version,
        pointer: snapshot === null ? null : JSON.parse(snapshot),
        pointerPath,
      });
    }
    if (action === 'set') {
      const deployment = JSON.parse(args[args.indexOf('--deployment') + 1]);
      const expectedIndex = args.indexOf('--expected');
      const expected = expectedIndex === -1 ? null : args[expectedIndex + 1];
      setCalls.push({ deployment, expected });
      if (expected !== null && expected !== current) {
        return { error: null, signal: null, code: 1, stdout: '', stderr: 'release pointer CAS mismatch' };
      }
      const candidate = JSON.stringify({
        schemaVersion: 'skiff-release-pointer-v1',
        profile,
        deployment,
        recordPath: `records/service-deployments/example~decho/${version}/${deployment.deploymentRevision}/${hexOf(deployment.deploymentArtifactIdentity)}.json`,
      });
      current = candidate;
      return okOutcome({ action: 'set', pointer: JSON.parse(candidate), pointerPath });
    }
    throw new Error(`unexpected release action ${action}`);
  };
  return {
    runCompiler,
    setCalls,
    getCalls,
    pointer: () => current,
    setPointer: (json) => {
      current = json;
    },
  };
}

function healthFetch({
  ok = true,
  healthProfile = profile,
  buildIds = [buildIdA],
  reachable = true,
  httpStatus = 200,
} = {}) {
  const calls = [];
  const fetchImpl = async (url) => {
    calls.push(url);
    if (!reachable) {
      throw new Error('fetch failed: connection refused');
    }
    if (httpStatus !== 200) {
      return { ok: false, status: httpStatus, json: async () => ({}) };
    }
    return {
      ok: true,
      json: async () => ({
        ok,
        activeAssembly: { profile: healthProfile, releaseCount: buildIds.length, buildIds },
        replicas: [],
        capabilityConnections: [],
      }),
    };
  };
  return { fetchImpl, calls };
}

async function tempArtifactStore() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-deploy-test-'));
  return { root, cleanup: () => rm(root, { recursive: true, force: true }) };
}

async function writeDeploymentRecord(root, { buildId, revision }) {
  const directory = join(root, 'records', 'service-deployments', 'example~decho', version, revision);
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, `${hexOf(buildId)}.json`), '{}');
}

function rollbackRecordFixture(overrides = {}) {
  return {
    schemaVersion: 'skiff-deploy-rollback-v1',
    profile,
    serviceId,
    version,
    deployedAt: '2026-08-07T00:00:00.000Z',
    buildId: buildIdA,
    deployment: pointer(buildIdA).deployment,
    previousPointer: null,
    ...overrides,
  };
}

test('parseDeployArgs validates per-action options, flags, and positionals', () => {
  const parsed = parseDeployArgs([
    'deploy',
    serviceId,
    version,
    '--root',
    '/x',
    '--artifact-root',
    '/y',
    '--profile',
    'dev',
    '--json',
  ]);
  assert.equal(parsed.action, 'deploy');
  assert.equal(parsed.serviceId, serviceId);
  assert.equal(parsed.version, version);
  assert.equal(parsed.root, '/x');
  assert.equal(parsed.artifactRoot, '/y');
  assert.equal(parsed.profile, profile);
  assert.equal(parsed.json, true);
  assert.equal(parsed.skipVerify, false);
  assert.equal(parsed.controlUrl, undefined);
  assert.equal(parsed.verifyTimeoutMs, 30_000);

  const skipped = parseDeployArgs([
    'deploy',
    serviceId,
    version,
    '--root=/x',
    '--artifact-root=/y',
    '--profile=dev',
    '--skip-verify',
    '--verify-timeout-ms',
    '5000',
    '--control-url',
    'http://127.0.0.1:9999',
  ]);
  assert.equal(skipped.skipVerify, true);
  assert.equal(skipped.verifyTimeoutMs, 5000);
  assert.equal(skipped.controlUrl, 'http://127.0.0.1:9999');

  const verify = parseDeployArgs(['verify', serviceId, version, '--artifact-root', '/y', '--profile', 'dev']);
  assert.equal(verify.action, 'verify');
  assert.equal(verify.root, undefined);
  const rollback = parseDeployArgs(['rollback', serviceId, version, '--artifact-root', '/y', '--profile', 'dev', '--to', buildIdA]);
  assert.equal(rollback.action, 'rollback');
  assert.equal(rollback.toBuildId, buildIdA);
  assert.equal(rollback.controlUrl, undefined);

  assert.throws(() => parseDeployArgs(['swap', '--artifact-root', '/y']), /unknown deploy command swap/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId]), /requires exactly <service> <version>/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--artifact-root', '/y', '--profile', 'dev']), /requires --root/);
  assert.throws(() => parseDeployArgs(['verify', serviceId, version, '--profile', 'dev']), /requires --artifact-root/);
  assert.throws(() => parseDeployArgs(['verify', serviceId, version, '--artifact-root', '/y']), /requires --profile/);
  assert.throws(() => parseDeployArgs(['rollback', serviceId, version, '--artifact-root', '/y', '--profile', 'dev', '--to', 'bad']), /--build-id must be a/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', 'dev', '--to', buildIdA]), /does not accept --to/);
  assert.throws(() => parseDeployArgs(['verify', serviceId, version, '--artifact-root', '/y', '--profile', 'dev', '--skip-verify']), /unknown option --skip-verify/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', 'dev', '--profile', 'prod']), /was provided more than once/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', 'dev', '--verify-timeout-ms', 'abc']), /positive safe integer/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', '..']), /canonical ASCII token/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', 'dev', '--control-url', 'https://127.0.0.1:4001']), /must be an absolute http:/);
  assert.throws(() => parseDeployArgs(['deploy', serviceId, version, '--root', '/x', '--artifact-root', '/y', '--profile', 'dev', '--control-url', 'http://127.0.0.1:4001/health']), /point exactly to the control origin/);
});

test('profile, control URL, health URL, and rollback path validation units', () => {
  assert.equal(validateProfileSegment('dev'), 'dev');
  assert.throws(() => validateProfileSegment('..'), /canonical ASCII token/);
  assert.equal(normalizeControlUrl('http://127.0.0.1:4001/'), 'http://127.0.0.1:4001');
  assert.equal(healthUrl('http://127.0.0.1:4001'), 'http://127.0.0.1:4001/__router/health');
  assert.equal(
    rollbackRecordRelativePath({ profile: 'dev', serviceId: 'example.echo', version: '1.0.0' }),
    rollbackRecordPath,
  );
});

test('rollback records round-trip and reject malformed documents', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const relative = await writeRollbackRecord(
      { artifactRoot: root, profile, serviceId, version },
      rollbackRecordFixture(),
    );
    assert.equal(relative, rollbackRecordPath);
    const read = await readRollbackRecord({ artifactRoot: root, profile, serviceId, version });
    assert.equal(read.recordPath, rollbackRecordPath);
    assert.equal(read.buildId, buildIdA);
    assert.equal(read.previousPointer, null);

    await writeFile(join(root, rollbackRecordPath), JSON.stringify(rollbackRecordFixture({ schemaVersion: 'skiff-deploy-rollback-v9' })));
    await assert.rejects(
      readRollbackRecord({ artifactRoot: root, profile, serviceId, version }),
      /schemaVersion must be skiff-deploy-rollback-v1/,
    );
    await writeFile(join(root, rollbackRecordPath), JSON.stringify(rollbackRecordFixture({ serviceId: 'other.service' })));
    await assert.rejects(
      readRollbackRecord({ artifactRoot: root, profile, serviceId, version }),
      /targets dev other\.service 1\.0\.0; requested dev example\.echo 1\.0\.0/,
    );
    await writeFile(join(root, rollbackRecordPath), 'not-json');
    await assert.rejects(
      readRollbackRecord({ artifactRoot: root, profile, serviceId, version }),
      /failed to read rollback record/,
    );
    await assert.rejects(
      readRollbackRecord({ artifactRoot: root, profile, serviceId, version: '2.0.0' }),
      /no rollback record at .+; run skiff deploy first or pass --to <build-id>/,
    );
  } finally {
    await cleanup();
  }
});

test('readHealth, deploymentRecordExists, and pollHealthForBuildId project the M4 health shape', async () => {
  const healthy = healthFetch({ buildIds: [buildIdA, buildIdB] });
  const projection = await readHealth({ controlUrl: 'http://127.0.0.1:4001', fetchImpl: healthy.fetchImpl });
  assert.equal(projection.ok, true);
  assert.equal(projection.profile, profile);
  assert.equal(projection.releaseCount, 2);
  assert.deepEqual(projection.buildIds, [buildIdA, buildIdB]);

  await assert.rejects(
    readHealth({ controlUrl: 'http://127.0.0.1:4001', fetchImpl: healthFetch({ reachable: false }).fetchImpl }),
    /router health unreachable at http:\/\/127\.0\.0\.1:4001\/__router\/health/,
  );
  await assert.rejects(
    readHealth({ controlUrl: 'http://127.0.0.1:4001', fetchImpl: healthFetch({ httpStatus: 503 }).fetchImpl }),
    /router health returned HTTP 503/,
  );
  await assert.rejects(
    readHealth({ controlUrl: 'http://127.0.0.1:4001', fetchImpl: healthFetch({ buildIds: 'nope' }).fetchImpl }),
    /must contain profile and buildIds/,
  );

  const poll = await pollHealthForBuildId({
    controlUrl: 'http://127.0.0.1:4001',
    profile,
    buildId: buildIdA,
    timeoutMs: 5000,
    fetchImpl: healthy.fetchImpl,
    delay: async () => {},
  });
  assert.equal(poll.ok, true);
  assert.equal(poll.attempts, 1);
  assert.ok(poll.elapsedMs >= 0);

  await assert.rejects(
    pollHealthForBuildId({
      controlUrl: 'http://127.0.0.1:4001',
      profile,
      buildId: buildIdB,
      timeoutMs: 10,
      fetchImpl: healthFetch({ buildIds: [buildIdA] }).fetchImpl,
      delay: async () => {},
    }),
    /timed out after 10ms: buildId .+ is not servable at http:\/\/127\.0\.0\.1:4001\/__router\/health/,
  );

  const { root, cleanup } = await tempArtifactStore();
  try {
    assert.equal(await deploymentRecordExists({ artifactRoot: root, recordPath: 'records/x.json' }), false);
    await mkdir(join(root, 'records'), { recursive: true });
    await writeFile(join(root, 'records', 'x.json'), '{}');
    assert.equal(await deploymentRecordExists({ artifactRoot: root, recordPath: 'records/x.json' }), true);
    await assert.rejects(deploymentRecordExists({ artifactRoot: root, recordPath: '' }), /non-empty string/);
    await assert.rejects(deploymentRecordExists({ artifactRoot: root, recordPath: '../escape.json' }), /escapes the artifact root/);
  } finally {
    await cleanup();
  }
});

test('deploy publishes, records the pre-deploy pointer, and verifies the new buildId', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    const fake = fakeCompiler({ initialPointerJson: null });
    const health = healthFetch({ buildIds: [buildIdA] });
    let published = 0;
    const publish = async ({ skiffRoot: rootArg, root: rootOption, artifactRoot, profile: profileArg }) => {
      assert.equal(rootArg, skiffRoot);
      assert.equal(rootOption, root);
      assert.equal(artifactRoot, root);
      assert.equal(profileArg, profile);
      published += 1;
      fake.setPointer(pointerJson(buildIdA, 'revision-1'));
      return publishReceipt(buildIdA, 'revision-1');
    };
    const output = [];
    const result = await runDeployCommand([
      'deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--json',
    ], {
      skiffRoot,
      stdout: (line) => output.push(line),
      runCompiler: fake.runCompiler,
      publish,
      fetchImpl: health.fetchImpl,
      delay: async () => {},
    });
    assert.equal(result.action, 'deploy');
    assert.equal(result.buildId, buildIdA);
    assert.equal(result.previousBuildId, null);
    assert.equal(result.pointerPath, pointerPath);
    assert.equal(result.rollbackRecordPath, rollbackRecordPath);
    assert.equal(result.releasePointer.deployment.deploymentArtifactIdentity, buildIdA);
    assert.equal(result.verify.ok, true);
    assert.equal(result.verify.attempts, 1);
    assert.equal(published, 1);
    assert.equal(health.calls.length, 1);
    assert.equal(health.calls[0], 'http://127.0.0.1:4001/__router/health');
    assert.equal(JSON.parse(output[0]).buildId, buildIdA);

    const record = JSON.parse(await readFile(join(root, rollbackRecordPath), 'utf8'));
    assert.equal(record.schemaVersion, 'skiff-deploy-rollback-v1');
    assert.equal(record.profile, profile);
    assert.equal(record.serviceId, serviceId);
    assert.equal(record.version, version);
    assert.equal(record.buildId, buildIdA);
    assert.equal(record.previousPointer, null);
    assert.match(record.deployedAt, /^\d{4}-\d{2}-\d{2}T/);
    assert.deepEqual(record.deployment, pointer(buildIdA).deployment);
  } finally {
    await cleanup();
  }
});

test('deploy is idempotent and keeps the pre-deploy pointer of the latest deploy', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    const fake = fakeCompiler({ initialPointerJson: null });
    const publish = async () => {
      fake.setPointer(pointerJson(buildIdA, 'revision-1'));
      return publishReceipt(buildIdA, 'revision-1');
    };
    const options = {
      skiffRoot,
      runCompiler: fake.runCompiler,
      publish,
      fetchImpl: healthFetch({ buildIds: [buildIdA] }).fetchImpl,
      delay: async () => {},
    };
    const first = await runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], options);
    assert.equal(first.previousBuildId, null);
    const second = await runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], options);
    assert.equal(second.buildId, buildIdA);
    assert.equal(second.previousBuildId, buildIdA);
    const record = JSON.parse(await readFile(join(root, rollbackRecordPath), 'utf8'));
    assert.equal(record.buildId, buildIdA);
    assert.equal(record.previousPointer.deployment.deploymentArtifactIdentity, buildIdA);
  } finally {
    await cleanup();
  }
});

test('deploy --skip-verify skips the health wait', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const fake = fakeCompiler({ initialPointerJson: null });
    const health = healthFetch({ buildIds: [buildIdA] });
    const result = await runDeployCommand([
      'deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify',
    ], {
      skiffRoot,
      runCompiler: fake.runCompiler,
      publish: async () => publishReceipt(buildIdA),
      fetchImpl: health.fetchImpl,
    });
    assert.deepEqual(result.verify, { skipped: true });
    assert.equal(health.calls.length, 0);
  } finally {
    await cleanup();
  }
});

test('deploy verify fails closed when the buildId never projects', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const fake = fakeCompiler({ initialPointerJson: null });
    const options = {
      skiffRoot,
      runCompiler: fake.runCompiler,
      publish: async () => publishReceipt(buildIdA),
      delay: async () => {},
    };
    await assert.rejects(
      runDeployCommand([
        'deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--verify-timeout-ms', '50',
      ], { ...options, fetchImpl: healthFetch({ buildIds: [buildIdB] }).fetchImpl }),
      /deploy verification timed out after 50ms: buildId .+ is not servable at http:\/\/127\.0\.0\.1:4001\/__router\/health/,
    );
    await assert.rejects(
      runDeployCommand([
        'deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--verify-timeout-ms', '50',
      ], { ...options, fetchImpl: healthFetch({ reachable: false }).fetchImpl }),
      /timed out after 50ms: buildId .+ \(.+connection refused/,
    );
  } finally {
    await cleanup();
  }
});

test('deploy fails closed on invalid publish receipts and publish failures', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const fake = fakeCompiler({ initialPointerJson: null });
    await assert.rejects(
      runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], {
        skiffRoot,
        runCompiler: fake.runCompiler,
        publish: async () => ({ serviceApiReceipt: { serviceId: 'other.service' } }),
      }),
      /produced service "other\.service", expected "example\.echo"/,
    );
    await assert.rejects(
      runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], {
        skiffRoot,
        runCompiler: fake.runCompiler,
        publish: async () => ({
          serviceApiReceipt: { serviceId },
          serviceDeploymentReceipt: publishReceipt(buildIdA).serviceDeploymentReceipt,
        }),
      }),
      /did not return a release pointer receipt/,
    );
    await assert.rejects(
      runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], {
        skiffRoot,
        runCompiler: fake.runCompiler,
        publish: async () => { throw new Error('publish boom'); },
      }),
      /publish boom/,
    );
  } finally {
    await cleanup();
  }
});

test('verify resolves the pointer, the deployment record, and the health projection', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    const fake = fakeCompiler({ initialPointerJson: pointerJson(buildIdA, 'revision-1') });
    const health = healthFetch({ buildIds: [buildIdA, buildIdB] });
    const output = [];
    const result = await runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile, '--json'], {
      skiffRoot,
      stdout: (line) => output.push(line),
      runCompiler: fake.runCompiler,
      fetchImpl: health.fetchImpl,
    });
    assert.equal(result.action, 'verify');
    assert.equal(result.buildId, buildIdA);
    assert.equal(result.pointerPath, pointerPath);
    assert.equal(result.recordPath, `records/service-deployments/example~decho/1.0.0/revision-1/${hexOf(buildIdA)}.json`);
    assert.equal(result.health.reachable, true);
    assert.equal(result.health.profile, profile);
    assert.equal(result.health.loaded, true);
    assert.equal(result.health.status, 'loaded');
    assert.equal(JSON.parse(output[0]).health.status, 'loaded');

    const resolvable = await runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
      skiffRoot,
      runCompiler: fake.runCompiler,
      fetchImpl: healthFetch({ buildIds: [buildIdB] }).fetchImpl,
    });
    assert.equal(resolvable.health.loaded, false);
    assert.equal(resolvable.health.status, 'resolvable');
  } finally {
    await cleanup();
  }
});

test('verify fails closed on missing pointer, missing record, and unreachable health', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const none = fakeCompiler({ initialPointerJson: null });
    await assert.rejects(
      runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: none.runCompiler,
        fetchImpl: healthFetch().fetchImpl,
      }),
      /release pointer for example\.echo@1\.0\.0 is not set/,
    );

    const recordMissing = fakeCompiler({ initialPointerJson: pointerJson(buildIdA) });
    await assert.rejects(
      runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: recordMissing.runCompiler,
        fetchImpl: healthFetch().fetchImpl,
      }),
      /deployment record missing for .+\/revision-1\/aa+\.json/,
    );

    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    await assert.rejects(
      runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: recordMissing.runCompiler,
        fetchImpl: healthFetch({ reachable: false }).fetchImpl,
      }),
      /router health unreachable at http:\/\/127\.0\.0\.1:4001\/__router\/health/,
    );
    await assert.rejects(
      runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: recordMissing.runCompiler,
        fetchImpl: healthFetch({ healthProfile: 'prod' }).fetchImpl,
      }),
      /router health activeAssembly profile "prod" does not match "dev"/,
    );

    const mismatchedDeployment = pointer(buildIdA);
    mismatchedDeployment.deployment.serviceId = 'other.service';
    const mismatched = fakeCompiler({ initialPointerJson: JSON.stringify(mismatchedDeployment) });
    await assert.rejects(
      runDeployCommand(['verify', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: mismatched.runCompiler,
        fetchImpl: healthFetch().fetchImpl,
      }),
      /release pointer deployment does not match the requested service and version/,
    );
  } finally {
    await cleanup();
  }
});

test('rollback returns the pointer to the buildId recorded by the last deploy', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    await writeDeploymentRecord(root, { buildId: buildIdB, revision: 'revision-2' });
    const fake = fakeCompiler({ initialPointerJson: pointerJson(buildIdA, 'revision-1') });
    const publish = async () => {
      fake.setPointer(pointerJson(buildIdB, 'revision-2'));
      return publishReceipt(buildIdB, 'revision-2');
    };
    await runDeployCommand(['deploy', serviceId, version, '--root', root, '--artifact-root', root, '--profile', profile, '--skip-verify'], {
      skiffRoot,
      runCompiler: fake.runCompiler,
      publish,
      fetchImpl: healthFetch().fetchImpl,
    });
    assert.equal(fake.pointer(), pointerJson(buildIdB, 'revision-2'));

    const output = [];
    const result = await runDeployCommand(['rollback', serviceId, version, '--artifact-root', root, '--profile', profile, '--json'], {
      skiffRoot,
      stdout: (line) => output.push(line),
      runCompiler: fake.runCompiler,
    });
    assert.equal(result.action, 'rollback');
    assert.equal(result.fromBuildId, buildIdB);
    assert.equal(result.toBuildId, buildIdA);
    assert.equal(result.source, 'rollback-record');
    assert.equal(result.rollbackRecordPath, rollbackRecordPath);
    assert.equal(result.pointer.deployment.deploymentArtifactIdentity, buildIdA);
    assert.equal(JSON.parse(output[0]).toBuildId, buildIdA);

    assert.equal(fake.setCalls.length, 1);
    assert.equal(fake.setCalls[0].expected, pointerJson(buildIdB, 'revision-2'));
    assert.equal(fake.setCalls[0].deployment.deploymentArtifactIdentity, buildIdA);
    assert.equal(fake.setCalls[0].deployment.deploymentRevision, 'revision-1');
    assert.equal(fake.pointer(), pointerJson(buildIdA, 'revision-1'));
  } finally {
    await cleanup();
  }
});

test('rollback --to points the pointer directly at an explicit buildId', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    const fake = fakeCompiler({ initialPointerJson: pointerJson(buildIdB, 'revision-2') });
    const result = await runDeployCommand(['rollback', serviceId, version, '--to', buildIdA, '--artifact-root', root, '--profile', profile, '--json'], {
      skiffRoot,
      runCompiler: fake.runCompiler,
    });
    assert.equal(result.fromBuildId, buildIdB);
    assert.equal(result.toBuildId, buildIdA);
    assert.equal(result.source, 'explicit');
    assert.equal(result.rollbackRecordPath, null);
    assert.equal(result.pointer.deployment.deploymentArtifactIdentity, buildIdA);
    assert.equal(fake.setCalls.length, 1);
    assert.equal(fake.setCalls[0].expected, pointerJson(buildIdB, 'revision-2'));
    assert.equal(fake.pointer(), pointerJson(buildIdA, 'revision-1'));
  } finally {
    await cleanup();
  }
});

test('rollback fails closed on missing pointer, record, target record, and CAS mismatch', async () => {
  const { root, cleanup } = await tempArtifactStore();
  try {
    const none = fakeCompiler({ initialPointerJson: null });
    await assert.rejects(
      runDeployCommand(['rollback', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: none.runCompiler,
      }),
      /no current release pointer for example\.echo@1\.0\.0; nothing to roll back/,
    );

    const withPointer = fakeCompiler({ initialPointerJson: pointerJson(buildIdA) });
    await assert.rejects(
      runDeployCommand(['rollback', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: withPointer.runCompiler,
      }),
      /no rollback record at .+; run skiff deploy first or pass --to <build-id>/,
    );

    await assert.rejects(
      runDeployCommand(['rollback', serviceId, version, '--to', buildIdA, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: withPointer.runCompiler,
      }),
      /no deployment (records for example\.echo@1\.0\.0|record for buildId .+ under example\.echo@1\.0\.0)/,
    );

    await writeDeploymentRecord(root, { buildId: buildIdA, revision: 'revision-1' });
    await writeRollbackRecord({ artifactRoot: root, profile, serviceId, version }, rollbackRecordFixture());
    await assert.rejects(
      runDeployCommand(['rollback', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: withPointer.runCompiler,
      }),
      /no previous buildId recorded .+; use --to <build-id>/,
    );

    await writeRollbackRecord(
      { artifactRoot: root, profile, serviceId, version },
      rollbackRecordFixture({ previousPointer: pointer(buildIdA) }),
    );
    const racing = fakeCompiler({
      initialPointerJson: pointerJson(buildIdB, 'revision-2'),
      onGet: () => racing.setPointer(pointerJson(buildIdA, 'revision-1')),
    });
    await assert.rejects(
      runDeployCommand(['rollback', serviceId, version, '--artifact-root', root, '--profile', profile], {
        skiffRoot,
        runCompiler: racing.runCompiler,
      }),
      /release set failed: release pointer CAS mismatch/,
    );
  } finally {
    await cleanup();
  }
});

test('renderDeployReceipt, renderVerifyReceipt, and renderRollbackReceipt render structured receipts', () => {
  const deploy = renderDeployReceipt({
    profile,
    serviceId,
    version,
    buildId: buildIdA,
    previousBuildId: null,
    pointerPath,
    rollbackRecordPath,
    verify: { ok: true, attempts: 2, elapsedMs: 13 },
  });
  assert.match(deploy, /^deployed dev example\.echo 1\.0\.0/);
  assert.match(deploy, /-> skiff-deployment-artifact-v4:sha256:aa+/);
  assert.match(deploy, /from: \(none\)/);
  assert.match(deploy, /verify: ok \(2 attempts, 13ms\)/);

  const skipped = renderDeployReceipt({
    profile,
    serviceId,
    version,
    buildId: buildIdA,
    previousBuildId: buildIdB,
    pointerPath,
    rollbackRecordPath,
    verify: { skipped: true },
  });
  assert.match(skipped, /from: skiff-deployment-artifact-v4:sha256:bb+/);
  assert.match(skipped, /verify: skipped/);

  const verify = renderVerifyReceipt({
    profile,
    serviceId,
    version,
    buildId: buildIdA,
    recordPath: `records/service-deployments/example~decho/1.0.0/revision-1/${hexOf(buildIdA)}.json`,
    health: { reachable: true, profile, releaseCount: 2, status: 'loaded' },
  });
  assert.match(verify, /^verified dev example\.echo 1\.0\.0/);
  assert.match(verify, /health: reachable, profile dev, releaseCount 2, status loaded/);

  const rollback = renderRollbackReceipt({
    profile,
    serviceId,
    version,
    fromBuildId: buildIdB,
    toBuildId: buildIdA,
    source: 'rollback-record',
    rollbackRecordPath,
    pointerPath,
  });
  assert.match(rollback, /^rolled back dev example\.echo 1\.0\.0/);
  assert.match(rollback, /source: rollback-record/);
  assert.match(rollback, /rollback record: pointers\/rollback\/dev\/example~decho\/1\.0\.0\.json/);

  const explicit = renderRollbackReceipt({
    profile,
    serviceId,
    version,
    fromBuildId: buildIdB,
    toBuildId: buildIdA,
    source: 'explicit',
    rollbackRecordPath: null,
    pointerPath,
  });
  assert.equal(explicit.includes('rollback record:'), false);
});

test('deployCommandUsage covers the three commands and help short-circuits', async () => {
  assert.match(deployCommandUsage, /skiff deploy <service> <version>/);
  assert.match(deployCommandUsage, /skiff verify <service> <version>/);
  assert.match(deployCommandUsage, /skiff rollback <service> <version>/);
  assert.match(deployCommandUsage, /pointers\/rollback/);
  for (const args of [['-h'], ['--help'], ['deploy', '-h'], ['verify', '--help'], ['rollback', '-h']]) {
    const output = [];
    const result = await runDeployCommand(args, { skiffRoot, stdout: (line) => output.push(line) });
    assert.equal(result, null);
    assert.equal(output.join('\n'), deployCommandUsage);
  }
});
