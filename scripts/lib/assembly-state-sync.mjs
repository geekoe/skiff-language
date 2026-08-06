// `skiff assembly sync-state` core + CLI: realign the persisted router
// activation state document (Mongo) with the router's active epoch as
// reported by /__router/health.
//
// The state document is rewritten exactly like `stack init` writes it
// (schemaVersion skiff-profile-activation-state-v1, revision 0, pending null);
// only the committed generation/assembly/configSnapshot come from the router
// health activeAssembly instead of a freshly authored empty assembly.

import { isAbsolute, resolve } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { readRouterActivationState } from './dev-assembly-activation.mjs';
import { defaultAssemblyActivationUrl } from './package-service-authoring.mjs';
import { createMongoshCommand } from './mongosh-json-command.mjs';
import { assertProfileToken } from './stack-config.mjs';

const ACTIVATION_STATE_SCHEMA_VERSION = 'skiff-profile-activation-state-v1';
const ACTIVATION_STATE_DATABASE = 'skiff-router';
const ACTIVATION_STATE_COLLECTION = 'activation_state';

export const syncStateMongoUrlEnvVar = 'SKIFF_ACTIVATION_STATE_MONGO_URL';

export const assemblyStateSyncUsage = `usage: skiff assembly sync-state --artifact-root <dir> --profile <name> --activation-url <url> --mongo-url <url> [--json]

Replaces the persisted router activation state document (skiff-router.activation_state,
_id: <profile>) with the router's current active epoch read from
<activation-url origin>/__router/health, resetting revision to 0 and pending to null.

--mongo-url may be omitted when ${syncStateMongoUrlEnvVar} is set.`;

export async function runAssemblyStateSyncCommand(rawArgs, {
  mongoUrlEnv = process.env[syncStateMongoUrlEnvVar],
  stdout = console.log,
  fetchImpl = fetch,
  mongosh = captureCheckedCommand,
} = {}) {
  if (rawArgs[0] === '-h' || rawArgs[0] === '--help') {
    console.log(assemblyStateSyncUsage);
    return null;
  }
  const parsed = parseSyncStateArgs(rawArgs);
  const mongoUrl = parsed.mongoUrl ?? mongoUrlEnv;
  if (typeof mongoUrl !== 'string' || mongoUrl.trim().length === 0) {
    throw new Error(
      `skiff assembly sync-state requires --mongo-url or ${syncStateMongoUrlEnvVar}`,
    );
  }
  return syncAssemblyState({
    artifactRoot: parsed.artifactRoot,
    profile: parsed.profile,
    activationUrl: parsed.activationUrl,
    mongoUrl,
    fetchImpl,
    mongosh,
    stdout,
    json: parsed.json,
  });
}

export async function syncAssemblyState({
  artifactRoot,
  profile,
  activationUrl = defaultAssemblyActivationUrl,
  mongoUrl,
  fetchImpl = fetch,
  mongosh = captureCheckedCommand,
  stdout = console.log,
  json = false,
}) {
  if (typeof artifactRoot !== 'string' || !isAbsolute(artifactRoot)) {
    throw new Error('assembly state sync requires an absolute --artifact-root');
  }
  assertProfileToken(profile, 'assembly state sync profile');
  if (typeof mongoUrl !== 'string' || mongoUrl.trim().length === 0) {
    throw new Error('assembly state sync requires a mongo URL');
  }
  const active = await readRouterActivationState({ fetchImpl, activationUrl });
  if (active.profile !== profile) {
    throw new Error(
      `router coordinates profile ${active.profile}, not requested ${profile}`,
    );
  }

  const mongo = createMongoshCommand({ checkedRunner: mongosh });
  const beforeDocument = await readStateDocument({
    mongo,
    mongoUrl,
    profile,
  });
  const before = projectStateDocument(beforeDocument);

  const stateDocument = {
    _id: profile,
    revision: 0,
    state: {
      schemaVersion: ACTIVATION_STATE_SCHEMA_VERSION,
      profile,
      committed: {
        generation: active.generation,
        assembly: active.assembly,
        configSnapshot: active.configSnapshot,
      },
      pending: null,
    },
  };
  const evalScript = [
    `db.getSiblingDB(${JSON.stringify(ACTIVATION_STATE_DATABASE)})`,
    `.getCollection(${JSON.stringify(ACTIVATION_STATE_COLLECTION)})`,
    `.replaceOne({_id: ${JSON.stringify(profile)}}, ${JSON.stringify(stateDocument)}, {upsert: true});`,
  ].join('');
  await mongosh('mongosh', [mongoUrl, '--quiet', '--eval', evalScript], {
    cwd: process.cwd(),
  });

  const after = {
    revision: 0,
    generation: active.generation,
    assemblyIdentity: active.assembly.assemblyIdentity,
    configSnapshotId: active.configSnapshot.snapshotId,
    pending: null,
  };
  const result = {
    profile,
    mongo: `${ACTIVATION_STATE_DATABASE}.${ACTIVATION_STATE_COLLECTION}`,
    before,
    after,
  };
  stdout(json ? JSON.stringify(result, null, 2) : renderStateSync(result));
  return result;
}

export function parseSyncStateArgs(rawArgs) {
  const options = new Map();
  const flags = new Set();
  const optionsWithValues = new Set([
    '--artifact-root',
    '--profile',
    '--activation-url',
    '--mongo-url',
  ]);
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    if (argument === '--json') {
      if (flags.has(argument)) {
        throw new Error('--json was provided more than once');
      }
      flags.add(argument);
      continue;
    }
    const equals = argument.indexOf('=');
    const option = equals === -1 ? argument : argument.slice(0, equals);
    if (optionsWithValues.has(option)) {
      if (options.has(option)) {
        throw new Error(`${option} was provided more than once`);
      }
      const value = equals === -1 ? rawArgs[index + 1] : argument.slice(equals + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${option} requires a value`);
      }
      options.set(option, value);
      if (equals === -1) {
        index += 1;
      }
      continue;
    }
    if (argument.startsWith('-')) {
      throw new Error(`unknown option ${argument}`);
    }
    throw new Error(
      `skiff assembly sync-state does not accept a positional root; use --artifact-root`,
    );
  }
  const artifactRoot = options.get('--artifact-root');
  if (artifactRoot === undefined) {
    throw new Error('skiff assembly sync-state requires --artifact-root');
  }
  const profile = options.get('--profile');
  if (profile === undefined) {
    throw new Error('skiff assembly sync-state requires --profile');
  }
  let activationUrl = options.get('--activation-url') ?? defaultAssemblyActivationUrl;
  try {
    activationUrl = new URL(activationUrl);
  } catch (error) {
    throw new Error(`--activation-url must be an absolute http(s) URL: ${error.message}`);
  }
  if (
    (activationUrl.protocol !== 'http:' && activationUrl.protocol !== 'https:')
    || activationUrl.username !== ''
    || activationUrl.password !== ''
  ) {
    throw new Error('--activation-url must be an absolute http(s) URL without credentials');
  }
  return {
    artifactRoot: resolve(artifactRoot),
    profile,
    activationUrl: activationUrl.toString().replace(/\/$/, ''),
    mongoUrl: options.get('--mongo-url'),
    json: flags.has('--json'),
  };
}

async function readStateDocument({ mongo, mongoUrl, profile }) {
  const expression = [
    `db.getSiblingDB(${JSON.stringify(ACTIVATION_STATE_DATABASE)})`,
    `.getCollection(${JSON.stringify(ACTIVATION_STATE_COLLECTION)})`,
    `.findOne({_id: ${JSON.stringify(profile)}}, {_id: 1, revision: 1, "state.schemaVersion": 1, "state.committed.generation": 1, "state.committed.assembly.assemblyIdentity": 1, "state.committed.configSnapshot.snapshotId": 1, "state.pending": 1})`,
  ].join('');
  return mongo.json({ url: mongoUrl, expression, cwd: process.cwd() });
}

function projectStateDocument(document) {
  if (document === null) {
    return null;
  }
  const committed = normalizeEjson(document.state?.committed ?? null);
  if (!isPlainObject(committed)) {
    return null;
  }
  const generation = committed.generation;
  if (!Number.isSafeInteger(generation) || generation < 0) {
    return null;
  }
  const assemblyIdentity = committed.assembly?.assemblyIdentity;
  const configSnapshotId = committed.configSnapshot?.snapshotId;
  if (typeof assemblyIdentity !== 'string' || typeof configSnapshotId !== 'string') {
    return null;
  }
  return {
    revision: normalizeEjson(document.revision ?? 0),
    generation,
    assemblyIdentity,
    configSnapshotId,
    pending: normalizeEjson(document.state?.pending ?? null),
  };
}

function normalizeEjson(value) {
  if (Array.isArray(value)) {
    return value.map(normalizeEjson);
  }
  if (isPlainObject(value)) {
    if (value.$numberInt !== undefined || value.$numberLong !== undefined) {
      return Number(value.$numberInt ?? value.$numberLong);
    }
    if (value.$numberDouble !== undefined) {
      return Number(value.$numberDouble);
    }
    return Object.fromEntries(
      Object.entries(value).map(([key, entry]) => [key, normalizeEjson(entry)]),
    );
  }
  return value;
}

function renderStateSync(result) {
  const mongo = `${result.mongo} (profile ${result.profile})`;
  const before = result.before ?? {};
  const lines = [
    `state: ${mongo}`,
    `generation: ${before.generation ?? '(missing)'} -> ${result.after.generation}`,
    `assembly: ${before.assemblyIdentity ?? '(missing)'}`,
    `  -> ${result.after.assemblyIdentity}`,
    `configSnapshot: ${before.configSnapshotId ?? '(missing)'}`,
    `  -> ${result.after.configSnapshotId}`,
    `revision: ${before.revision ?? '(missing)'} -> ${result.after.revision}`,
  ];
  return lines.join('\n');
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
