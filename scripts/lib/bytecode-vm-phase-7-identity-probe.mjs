import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import {
  PHASE7_IDENTITY_SCHEMA,
  phase7CapabilityLedger,
  phase7SpecCatalogDigest,
  sha256,
} from './bytecode-vm-phase-7-contract.mjs';
import { phase1ObservationSchemaIdentity } from './bytecode-vm-phase-1-observation-schema.mjs';

const SOURCES = Object.freeze({
  frameSchema: {
    path: 'runtime/transport/src/protocol/frame.rs',
    constants: Object.freeze({
      BINARY_FRAME_VERSION: 'u8',
      RUNTIME_FRAME_SCHEMA_VERSION: '&str',
      RESPONSE_ERROR_FRAME_SCHEMA_VERSION: '&str',
    }),
  },
  artifactIdentity: {
    path: 'artifact-identity/src/constants.rs',
    constants: Object.freeze({
      FILE_IR_IDENTITY_PREFIX: '&str',
      BYTECODE_IDENTITY_SCHEMA_MARKER: '&str',
      BYTECODE_IDENTITY_PREFIX: '&str',
      ACTOR_ABI_IDENTITY_PREFIX: '&str',
      ACTOR_IMPLEMENTATION_IDENTITY_PREFIX: '&str',
      PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX: '&str',
      SERVICE_PROTOCOL_IDENTITY_PREFIX: '&str',
      DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER: '&str',
      DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX: '&str',
      ASSEMBLY_IDENTITY_SCHEMA_MARKER: '&str',
    }),
  },
  assemblyIdentity: {
    path: 'artifact-model/src/activation_lexical.rs',
    constants: Object.freeze({
      RUNTIME_ASSEMBLY_IDENTITY_PREFIX: '&str',
    }),
  },
});

export async function readPhase7IdentitySources(root) {
  const record = {};
  for (const [group, { path, constants }] of Object.entries(SOURCES)) {
    const source = await readFile(join(root, path), 'utf8');
    const values = {};
    for (const [name, type] of Object.entries(constants)) {
      const value = parseConstant(source, name, type);
      if (value === null) {
        throw new Error(`Phase 7 identity source ${path} is missing constant ${name}`);
      }
      values[name] = value;
    }
    record[group] = Object.freeze(values);
  }
  return Object.freeze(record);
}

export async function phase7IdentityRecord(root) {
  const sources = await readPhase7IdentitySources(root);
  const observationSchema = phase1ObservationSchemaIdentity();
  const capabilityLedgerDigest = sha256(JSON.stringify(phase7CapabilityLedger(root)));
  const specCatalogDigest = phase7SpecCatalogDigest(root);
  const content = {
    schemaVersion: PHASE7_IDENTITY_SCHEMA,
    ...sources,
    observationSchema,
    capabilityLedgerDigest,
    specCatalogDigest,
  };
  return Object.freeze({
    ...content,
    digest: sha256(JSON.stringify(content)),
  });
}

export async function runPhase7IdentityProbe(root = process.cwd()) {
  const record = await phase7IdentityRecord(root);
  process.stdout.write(`${JSON.stringify(record, null, 2)}\n`);
  return record;
}

function parseConstant(source, name, type) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(
    `pub const ${escaped}\\s*:\\s*${type}\\s*=\\s*("(?:[^"\\\\]|\\\\.)*"|\\d+)`,
    's',
  );
  const match = source.match(pattern);
  if (!match) return null;
  const literal = match[1];
  if (literal.startsWith('"')) {
    return JSON.parse(literal);
  }
  return Number(literal);
}

if (process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runPhase7IdentityProbe().catch((error) => {
    process.stderr.write(`${error?.stack ?? error}\n`);
    process.exitCode = 1;
  });
}