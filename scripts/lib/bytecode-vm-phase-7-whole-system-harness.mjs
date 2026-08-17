import {
  mkdir,
  readFile,
  writeFile,
} from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { sha256 } from './bytecode-vm-phase-7-contract.mjs';

export const PHASE7_CARRIER_ENV = 'SKIFF_BYTECODE_VM_PHASE7_CARRIER_ROOT';
export const PHASE7_WHOLE_SYSTEM_ARTIFACT = 'phase-7-whole-system-composition';
export const PHASE7_WHOLE_SYSTEM_SCHEMA = 'skiff-bytecode-vm-phase-7-whole-system-r1-v1';

const ARTIFACT_FILE = `${PHASE7_WHOLE_SYSTEM_ARTIFACT}.json`;

export async function runPhase7WholeSystemStep(mode, {
  root = process.cwd(),
  env = process.env,
} = {}) {
  const carrier = env[PHASE7_CARRIER_ENV];
  if (typeof carrier !== 'string' || carrier.length === 0
    || resolve(carrier) !== carrier) {
    throw new Error(`${PHASE7_CARRIER_ENV} must be a canonical absolute directory`);
  }
  if (mode === 'producer') {
    const record = await produceArtifact(carrier);
    process.stdout.write(`${JSON.stringify(record, null, 2)}\n`);
    return record;
  }
  if (mode === 'consumer') {
    const record = await consumeArtifact(carrier);
    process.stdout.write(tap(record));
    return record;
  }
  throw new Error(`unknown whole-system step ${mode}`);
}

export async function produceArtifact(carrier) {
  await mkdir(carrier, { recursive: true });
  const record = {
    schemaVersion: PHASE7_WHOLE_SYSTEM_SCHEMA,
    artifactId: PHASE7_WHOLE_SYSTEM_ARTIFACT,
    digest: sha256(JSON.stringify({
      schemaVersion: PHASE7_WHOLE_SYSTEM_SCHEMA,
      artifactId: PHASE7_WHOLE_SYSTEM_ARTIFACT,
    })),
  };
  await writeFile(join(carrier, ARTIFACT_FILE), `${JSON.stringify(record, null, 2)}\n`, {
    flag: 'w',
    mode: 0o600,
  });
  return record;
}

export async function consumeArtifact(carrier) {
  const bytes = await readFile(join(carrier, ARTIFACT_FILE), 'utf8');
  const record = JSON.parse(bytes);
  const expected = {
    schemaVersion: PHASE7_WHOLE_SYSTEM_SCHEMA,
    artifactId: PHASE7_WHOLE_SYSTEM_ARTIFACT,
    digest: sha256(JSON.stringify({
      schemaVersion: PHASE7_WHOLE_SYSTEM_SCHEMA,
      artifactId: PHASE7_WHOLE_SYSTEM_ARTIFACT,
    })),
  };
  if (record.schemaVersion !== expected.schemaVersion
    || record.artifactId !== expected.artifactId
    || record.digest !== expected.digest) {
    throw new Error(`whole-system artifact record does not match the produced identity`);
  }
  return { ...expected, consumed: true };
}

function tap(record) {
  return [
    'TAP version 13',
    '1..1',
    '# tests 1',
    '# pass 1',
    '# fail 0',
    '# cancelled 0',
    '# skipped 0',
    '# todo 0',
    '',
    `ok 1 - whole-system consumer verified ${record.artifactId}`,
    '',
  ].join('\n');
}

if (process.argv[1] !== undefined
  && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  runPhase7WholeSystemStep(process.argv[2]).catch((error) => {
    process.stderr.write(`${error?.stack ?? error}\n`);
    process.exitCode = 1;
  });
}