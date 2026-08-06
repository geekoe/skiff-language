// `skiff release set/unset/get` core + CLI: manipulate the release pointer
// table entry `(profile, serviceId, version) -> buildId` in a canonical
// artifact store.
//
// set resolves the exact deployment record that declares the buildId under
// `records/service-deployments/<service~enc>/<version>/<revision>/<hex>.json`
// (uniqueness required), then delegates the atomic pointer write to the
// `skiff-compiler release` action. unset removes the pointer under the same
// lock; get reads it back.

import { lstat, readdir } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';

import { captureAttachedCommand } from './command-execution.mjs';
import { cargoTargetDir } from './cargo-target-dir.mjs';

const DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX = 'skiff-deployment-artifact-v4:sha256:';
const DEPLOYMENT_RECORDS_DIR = 'records/service-deployments';

export const releaseCommandUsage = `usage:
  skiff release set --artifact-root <dir> --profile <name> --service <id> --version <v> --build-id <id> [--expected '<exact ReleasePointer JSON>'] [--json]
  skiff release unset --artifact-root <dir> --profile <name> --service <id> --version <v> [--expected '<exact ReleasePointer JSON>'] [--json]
  skiff release get --artifact-root <dir> --profile <name> --service <id> --version <v> [--json]

set resolves the deployment record declaring the buildId (deployment artifact
identity), verifies it is the only match, and writes the release pointer
atomically. Without --expected the write is an unconditional atomic
overwrite; with --expected it is a compare-and-swap on the current pointer.
unset removes the pointer under the same lock and is idempotent; get prints
the current pointer.`;

export async function runReleaseCommand(rawArgs, {
  skiffRoot,
  stdout = console.log,
  runCompiler = captureAttachedCommand,
} = {}) {
  if (rawArgs[0] === '-h' || rawArgs[0] === '--help') {
    stdout(releaseCommandUsage);
    return null;
  }
  const parsed = parseReleaseArgs(rawArgs);
  let deploymentRefJson;
  if (parsed.action === 'set') {
    const match = await locateDeploymentRecord({
      artifactRoot: parsed.artifactRoot,
      serviceId: parsed.serviceId,
      version: parsed.version,
      buildId: parsed.buildId,
    });
    deploymentRefJson = serviceDeploymentRefJson({
      serviceId: parsed.serviceId,
      version: parsed.version,
      revision: match.revision,
      buildId: parsed.buildId,
    });
  }
  const invocation = releaseCompilerInvocation({ skiffRoot, ...parsed, deploymentRefJson });
  const outcome = await runCompiler(invocation.command, invocation.args, {
    cwd: invocation.cwd,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: cargoTargetDir(skiffRoot),
    },
  });
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    const detail = outcome.stderr.trim() || outcome.stdout.trim()
      || outcome.error?.message || `cargo exited ${outcome.signal ?? outcome.code}`;
    throw new Error(`release ${parsed.action} failed: ${detail}`);
  }
  let receipt;
  try {
    receipt = JSON.parse(outcome.stdout);
  } catch (error) {
    throw new Error(`release ${parsed.action} returned invalid JSON: ${error.message}`);
  }
  stdout(parsed.json ? JSON.stringify(receipt, null, 2) : renderReleaseReceipt(receipt));
  return receipt;
}

export function parseReleaseArgs(rawArgs) {
  const action = rawArgs[0];
  if (action !== 'set' && action !== 'unset' && action !== 'get') {
    throw new Error(`unknown release action ${action ?? '(missing)'}; expected set, unset, or get\n${releaseCommandUsage}`);
  }
  const options = new Map();
  const flags = new Set();
  const optionsWithValues = new Set([
    '--artifact-root',
    '--profile',
    '--service',
    '--version',
    '--build-id',
    '--expected',
  ]);
  for (let index = 1; index < rawArgs.length; index += 1) {
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
    throw new Error(`skiff release ${action} does not accept a positional argument: ${argument}`);
  }
  const artifactRoot = options.get('--artifact-root');
  if (artifactRoot === undefined) {
    throw new Error(`skiff release ${action} requires --artifact-root`);
  }
  const profile = options.get('--profile');
  if (profile === undefined) {
    throw new Error(`skiff release ${action} requires --profile`);
  }
  const serviceId = options.get('--service');
  if (serviceId === undefined) {
    throw new Error(`skiff release ${action} requires --service <id>`);
  }
  const version = options.get('--version');
  if (version === undefined) {
    throw new Error(`skiff release ${action} requires --version <v>`);
  }
  const parsed = {
    action,
    artifactRoot: resolve(artifactRoot),
    profile,
    serviceId,
    version,
    expected: options.get('--expected'),
    json: flags.has('--json'),
  };
  if (action === 'set') {
    const buildId = options.get('--build-id');
    if (buildId === undefined) {
      throw new Error('skiff release set requires --build-id <id>');
    }
    parsed.buildId = buildId;
  }
  return parsed;
}

export function validateBuildId(buildId) {
  if (typeof buildId !== 'string' || !buildId.startsWith(DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX)) {
    throw new Error(`--build-id must be a ${DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:sha256 identity`);
  }
  const hex = buildId.slice(DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX.length);
  if (!/^[0-9a-f]{64}$/.test(hex)) {
    throw new Error('--build-id must end with 64 lowercase hex characters');
  }
  return hex;
}

// Mirrors the artifact-identity coordinate_segment codec for the
// service/version directory layout of deployment records.
export function encodeServiceSegment(serviceId) {
  if (
    typeof serviceId !== 'string'
    || serviceId.length === 0
    || serviceId.length > 200
    || serviceId !== serviceId.trim()
    || serviceId.includes('~')
    || serviceId.includes('//')
    || serviceId.startsWith('/')
    || serviceId.endsWith('/')
    || /[^a-z0-9_\-./]/.test(serviceId)
  ) {
    throw new Error(`invalid service id ${JSON.stringify(serviceId)}; expected a canonical coordinate`);
  }
  return serviceId.replace(/\./g, '~d').replace(/\//g, '~s');
}

export function validateVersionSegment(version) {
  if (
    typeof version !== 'string'
    || version.length === 0
    || version.length > 200
    || version !== version.trim()
    || version === '.'
    || version === '..'
    || /[^a-zA-Z0-9_\-.]/.test(version)
  ) {
    throw new Error(`invalid version ${JSON.stringify(version)}`);
  }
  return version;
}

// Finds the exact deployment record file `{buildId hex}.json` under the
// version directory; multiple matching revisions are rejected.
export async function locateDeploymentRecord({ artifactRoot, serviceId, version, buildId }) {
  const hex = validateBuildId(buildId);
  const serviceSegment = encodeServiceSegment(serviceId);
  const versionSegment = validateVersionSegment(version);
  const directory = join(artifactRoot, DEPLOYMENT_RECORDS_DIR, serviceSegment, versionSegment);
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`no deployment records for ${serviceId}@${version}: ${directory}`);
    }
    throw error;
  }
  const fileName = `${hex}.json`;
  const matches = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) {
      continue;
    }
    const recordPath = join(directory, entry.name, fileName);
    let isRegularFile = false;
    try {
      isRegularFile = (await lstat(recordPath)).isFile();
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
    if (isRegularFile) {
      matches.push({ revision: entry.name, recordPath });
    }
  }
  if (matches.length === 0) {
    throw new Error(`no deployment record for buildId ${buildId} under ${serviceId}@${version}`);
  }
  if (matches.length > 1) {
    throw new Error(
      `ambiguous deployment record for buildId ${buildId}: revisions ${matches.map((match) => match.revision).join(', ')}; expected exactly one`,
    );
  }
  return matches[0];
}

export function serviceDeploymentRefJson({ serviceId, version, revision, buildId }) {
  validateBuildId(buildId);
  return JSON.stringify({
    serviceId,
    contractVersion: version,
    deploymentRevision: revision,
    deploymentArtifactIdentity: buildId,
  });
}

export function releaseCompilerInvocation({
  skiffRoot,
  action,
  artifactRoot,
  profile,
  serviceId,
  version,
  deploymentRefJson,
  expected,
  json,
}) {
  if (!isAbsolute(skiffRoot) || !isAbsolute(artifactRoot)) {
    throw new Error('release requires absolute skiffRoot and artifactRoot');
  }
  const args = [
    'run',
    '--quiet',
    '--manifest-path',
    resolve(skiffRoot, 'compiler', 'Cargo.toml'),
    '--bin',
    'skiff-compiler',
    '--',
    'release',
    action,
    '--artifact-root',
    artifactRoot,
    '--profile',
    profile,
  ];
  if (action === 'set') {
    if (typeof deploymentRefJson !== 'string' || deploymentRefJson.length === 0) {
      throw new Error('release set requires an exact ServiceDeploymentRef JSON');
    }
    args.push('--deployment', deploymentRefJson);
  } else {
    args.push('--service', serviceId, '--version', version);
  }
  if (expected !== undefined) {
    args.push('--expected', expected);
  }
  if (json) {
    args.push('--json');
  }
  return { command: 'cargo', cwd: skiffRoot, args };
}

export function renderReleaseReceipt(receipt) {
  const pointer = receipt?.pointer ?? receipt?.removedPointer ?? null;
  const key = receipt?.profile && receipt?.serviceId && receipt?.version
    ? `${receipt.profile} ${receipt.serviceId} ${receipt.version}`
    : pointer
      ? `${pointer.profile} ${pointer.deployment.serviceId} ${pointer.deployment.contractVersion}`
      : '(unknown)';
  const buildId = pointer?.deployment?.deploymentArtifactIdentity ?? '(none)';
  const pointerPath = receipt?.pointerPath ?? '(unknown)';
  if (receipt?.action === 'set') {
    return `release pointer set for ${key}\n  -> ${buildId}\npointer: ${pointerPath}`;
  }
  if (receipt?.action === 'unset') {
    return `release pointer unset for ${key}\n  removed: ${buildId}\npointer: ${pointerPath}`;
  }
  if (receipt?.action === 'get') {
    return `release pointer for ${key}\n  -> ${buildId}\npointer: ${pointerPath}`;
  }
  return JSON.stringify(receipt);
}
