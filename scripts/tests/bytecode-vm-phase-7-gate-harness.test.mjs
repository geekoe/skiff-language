import assert from 'node:assert/strict';
import {
  mkdir,
  mkdtemp,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  PHASE7_IDENTITY_SCHEMA,
  phase7CapabilityLedger,
  phase7SpecCatalogDigest,
  sha256,
} from '../lib/bytecode-vm-phase-7-contract.mjs';
import {
  phase7IdentityRecord,
  readPhase7IdentitySources,
} from '../lib/bytecode-vm-phase-7-identity-probe.mjs';
import {
  PHASE7_CARRIER_ENV,
  PHASE7_WHOLE_SYSTEM_ARTIFACT,
  consumeArtifact,
  produceArtifact,
} from '../lib/bytecode-vm-phase-7-whole-system-harness.mjs';

const REPOSITORY = new URL('../..', import.meta.url).pathname;

test('identity probe reads every schema/ISA fact from the candidate path', async () => {
  const sources = await readPhase7IdentitySources(REPOSITORY);
  assert.equal(sources.frameSchema.BINARY_FRAME_VERSION, 1);
  assert.equal(typeof sources.frameSchema.RUNTIME_FRAME_SCHEMA_VERSION, 'string');
  assert.equal(sources.frameSchema.RUNTIME_FRAME_SCHEMA_VERSION.length > 0, true);
  assert.equal(sources.assemblyIdentity.RUNTIME_ASSEMBLY_IDENTITY_PREFIX.length > 0, true);
  assert.equal(
    sources.artifactIdentity.DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX.startsWith('skiff-deployment-artifact-v'),
    true,
  );
  assert.equal(sources.artifactIdentity.DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX.endsWith(':sha256'), true);
});

test('identity record binds observation schema, ledger and catalog digests deterministically', async () => {
  const record = await phase7IdentityRecord(REPOSITORY);
  const again = await phase7IdentityRecord(REPOSITORY);
  assert.equal(record.schemaVersion, PHASE7_IDENTITY_SCHEMA);
  assert.equal(JSON.stringify(record), JSON.stringify(again));
  assert.equal(/^[a-f0-9]{64}$/.test(record.digest), true);
  assert.equal(
    record.capabilityLedgerDigest,
    sha256(JSON.stringify(phase7CapabilityLedger(REPOSITORY))),
  );
  assert.equal(record.specCatalogDigest, phase7SpecCatalogDigest(REPOSITORY));
  assert.equal(record.observationSchema.version, 'skiff-bytecode-vm-phase-1-observation-v1');
});

test('whole-system producer and consumer steps round-trip the exact artifact identity', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-carrier-'));
  const temp = await realpath(created);
  const carrier = join(temp, 'carrier');
  try {
    const produced = await produceArtifact(carrier);
    assert.equal(produced.artifactId, PHASE7_WHOLE_SYSTEM_ARTIFACT);
    const consumed = await consumeArtifact(carrier);
    assert.equal(consumed.consumed, true);
    assert.equal(consumed.artifactId, PHASE7_WHOLE_SYSTEM_ARTIFACT);
    assert.equal(consumed.digest, produced.digest);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('whole-system consumer rejects a swapped or damaged artifact record', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-carrier-bad-'));
  const temp = await realpath(created);
  const carrier = join(temp, 'carrier');
  try {
    await mkdir(carrier);
    await writeFile(
      join(carrier, `${PHASE7_WHOLE_SYSTEM_ARTIFACT}.json`),
      '{"schemaVersion":"stale","artifactId":"other"}\n',
    );
    await assert.rejects(consumeArtifact(carrier), /does not match the produced identity/);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('whole-system harness requires a canonical carrier root environment', async () => {
  const { runPhase7WholeSystemStep } = await import(
    '../lib/bytecode-vm-phase-7-whole-system-harness.mjs'
  );
  await assert.rejects(
    runPhase7WholeSystemStep('producer', { env: {} }),
    new RegExp(`${PHASE7_CARRIER_ENV} must be a canonical absolute directory`),
  );
});