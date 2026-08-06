// `skiff deploy / verify / rollback` core + CLI (M5 pipeline).
//
// deploy publishes the service package root through the canonical authoring
// transaction (`package publish`), which also writes the release pointer
// `(profile, serviceId, version) -> buildId` in the same transaction, records
// the pointer value observed before the deploy in a rollback record at
// `pointers/rollback/<profile>/<service>/<version>.json` (schema
// `skiff-deploy-rollback-v1`), then waits for `/__router/health` to project
// the new buildId in `activeAssembly.buildIds` (fail-closed on timeout;
// `--skip-verify` skips the wait). verify checks that the release pointer is
// set and resolvable (exact deployment record exists, buildId valid) and that
// the router health projection is reachable. rollback points the release
// pointer back to the buildId recorded by the last deploy, or to an explicit
// `--to <build-id>`, as a compare-and-swap (`--expected`) against the current
// pointer.

import { randomUUID } from 'node:crypto';
import {
  lstat,
  mkdir,
  open,
  readFile,
  rename,
  rm,
} from 'node:fs/promises';
import { dirname, join, resolve, sep } from 'node:path';

import { runCompilerAuthoring } from './package-service-authoring.mjs';
import {
  encodeServiceSegment,
  runReleaseCommand,
  validateBuildId,
  validateVersionSegment,
} from './release-command.mjs';

const DEPLOY_ROLLBACK_SCHEMA_VERSION = 'skiff-deploy-rollback-v1';
const DEFAULT_VERIFY_TIMEOUT_MS = 30_000;
const HEALTH_POLL_INTERVAL_MS = 500;
const DEFAULT_CONTROL_URL = 'http://127.0.0.1:4001';
const profileSegmentPattern = /^[A-Za-z0-9._-]{1,200}$/;

export const deployCommandUsage = `usage:
  skiff deploy <service> <version> --root <dir> --artifact-root <dir> --profile <name> [--control-url <url>] [--skip-verify] [--verify-timeout-ms <ms>] [--json]
  skiff verify <service> <version> --artifact-root <dir> --profile <name> [--control-url <url>] [--json]
  skiff rollback <service> <version> [--to <build-id>] --artifact-root <dir> --profile <name> [--json]

deploy publishes the package root through package publish, which writes the
release pointer (profile, serviceId, version) -> buildId in the same
transaction, records the pointer observed before the deploy in a rollback
record at pointers/rollback/<profile>/<service>/<version>.json (schema
skiff-deploy-rollback-v1), then waits for /__router/health to project the new
buildId in activeAssembly.buildIds; the wait fails closed on timeout.
verify checks that the release pointer is set and its exact deployment record
exists, and that the router health projection is reachable (fail closed when
the pointer, record, or health is missing).
rollback points the release pointer back to the buildId recorded by the last
deploy, or to --to <build-id> explicitly; the pointer write is a
compare-and-swap against the current pointer.`;

export async function runDeployCommand(rawArgs, {
  skiffRoot,
  stdout = console.log,
  runCompiler,
  publish,
  fetchImpl = fetch,
  delay = sleep,
} = {}) {
  const action = rawArgs[0];
  const helpRequested = ['-h', '--help'].includes(action)
    || (
      action === 'deploy'
      || action === 'verify'
      || action === 'rollback'
    ) && ['-h', '--help'].includes(rawArgs[1]);
  if (helpRequested) {
    stdout(deployCommandUsage);
    return null;
  }
  const parsed = parseDeployArgs(rawArgs);
  const options = { skiffRoot, stdout, runCompiler, publish, fetchImpl, delay };
  switch (parsed.action) {
    case 'deploy':
      return runDeployAction(parsed, options);
    case 'verify':
      return runVerifyAction(parsed, options);
    case 'rollback':
      return runRollbackAction(parsed, options);
    default:
      throw new Error(`unknown deploy command ${parsed.action}\n${deployCommandUsage}`);
  }
}

export function parseDeployArgs(rawArgs) {
  const action = rawArgs[0];
  if (action !== 'deploy' && action !== 'verify' && action !== 'rollback') {
    throw new Error(
      `unknown deploy command ${action ?? '(missing)'}; expected deploy, verify, or rollback\n${deployCommandUsage}`,
    );
  }
  const allowedOptions = new Map([
    ['deploy', new Set(['--artifact-root', '--profile', '--root', '--control-url', '--verify-timeout-ms'])],
    ['verify', new Set(['--artifact-root', '--profile', '--control-url'])],
    ['rollback', new Set(['--artifact-root', '--profile', '--to'])],
  ]);
  const allowedFlags = new Map([
    ['deploy', new Set(['--json', '--skip-verify'])],
    ['verify', new Set(['--json'])],
    ['rollback', new Set(['--json'])],
  ]);
  const options = new Map();
  const flags = new Set();
  const positionals = [];
  const optionsWithValues = new Set([
    '--artifact-root',
    '--profile',
    '--root',
    '--control-url',
    '--verify-timeout-ms',
    '--to',
  ]);
  for (let index = 1; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    if (allowedFlags.get(action).has(argument)) {
      if (flags.has(argument)) {
        throw new Error(`${argument} was provided more than once`);
      }
      flags.add(argument);
      continue;
    }
    const equals = argument.indexOf('=');
    const option = equals === -1 ? argument : argument.slice(0, equals);
    if (optionsWithValues.has(option)) {
      if (!allowedOptions.get(action).has(option)) {
        throw new Error(`skiff ${action} does not accept ${option}`);
      }
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
    positionals.push(argument);
  }
  if (positionals.length !== 2) {
    throw new Error(`skiff ${action} requires exactly <service> <version>`);
  }
  const [serviceId, version] = positionals;
  encodeServiceSegment(serviceId);
  validateVersionSegment(version);
  const artifactRoot = options.get('--artifact-root');
  if (artifactRoot === undefined) {
    throw new Error(`skiff ${action} requires --artifact-root`);
  }
  const profile = options.get('--profile');
  if (profile === undefined) {
    throw new Error(`skiff ${action} requires --profile <name>`);
  }
  validateProfileSegment(profile);
  let toBuildId;
  if (options.has('--to')) {
    validateBuildId(options.get('--to'));
    toBuildId = options.get('--to');
  }
  let verifyTimeoutMs = DEFAULT_VERIFY_TIMEOUT_MS;
  if (options.has('--verify-timeout-ms')) {
    const parsedTimeout = Number(options.get('--verify-timeout-ms'));
    if (!Number.isSafeInteger(parsedTimeout) || parsedTimeout <= 0) {
      throw new Error('--verify-timeout-ms must be a positive safe integer');
    }
    verifyTimeoutMs = parsedTimeout;
  }
  let controlUrl;
  if (options.has('--control-url')) {
    controlUrl = normalizeControlUrl(options.get('--control-url'));
  }
  const parsed = {
    action,
    artifactRoot: resolve(artifactRoot),
    profile,
    serviceId,
    version,
    controlUrl,
    verifyTimeoutMs,
    json: flags.has('--json'),
  };
  if (action === 'deploy') {
    const root = options.get('--root');
    if (root === undefined) {
      throw new Error('skiff deploy requires --root <dir>');
    }
    parsed.root = resolve(root);
    parsed.skipVerify = flags.has('--skip-verify');
  }
  if (action === 'rollback') {
    parsed.toBuildId = toBuildId;
  }
  return parsed;
}

export function validateProfileSegment(profile, option = '--profile') {
  if (
    typeof profile !== 'string'
    || profile === '.'
    || profile === '..'
    || !profileSegmentPattern.test(profile)
  ) {
    throw new Error(`${option} must be a canonical ASCII token`);
  }
  return profile;
}

export function normalizeControlUrl(value, option = '--control-url') {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${option} requires an absolute http:// URL`);
  }
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${option} must be an absolute http:// URL: ${value}`);
  }
  if (
    url.protocol !== 'http:'
    || url.username !== ''
    || url.password !== ''
    || url.search !== ''
    || url.hash !== ''
  ) {
    throw new Error(`${option} must be an absolute http:// URL without credentials, query, or fragment`);
  }
  if (url.pathname !== '' && url.pathname !== '/') {
    throw new Error(`${option} must point exactly to the control origin`);
  }
  return url.origin;
}

export function healthUrl(controlUrl) {
  return `${normalizeControlUrl(controlUrl)}/__router/health`;
}

export function rollbackRecordRelativePath({ profile, serviceId, version }) {
  return join(
    'pointers',
    'rollback',
    profile,
    encodeServiceSegment(serviceId),
    `${version}.json`,
  );
}

function rollbackRecordAbsolutePath({ artifactRoot, profile, serviceId, version }) {
  return join(artifactRoot, rollbackRecordRelativePath({ profile, serviceId, version }));
}

export async function writeRollbackRecord({ artifactRoot, profile, serviceId, version }, record) {
  const path = rollbackRecordAbsolutePath({ artifactRoot, profile, serviceId, version });
  const parent = dirname(path);
  await mkdir(parent, { recursive: true });
  const temporaryPath = `${path}.tmp-${process.pid}-${randomUUID()}`;
  let temporary;
  try {
    temporary = await open(temporaryPath, 'wx', 0o600);
    await temporary.writeFile(`${JSON.stringify(record, null, 2)}\n`, 'utf8');
    await temporary.sync();
    await temporary.close();
    temporary = undefined;
    await rename(temporaryPath, path);
  } catch (error) {
    await temporary?.close().catch(() => {});
    await rm(temporaryPath, { force: true }).catch(() => {});
    throw new Error(`failed to write rollback record ${path}: ${formatMessage(error)}`, {
      cause: error,
    });
  }
  return rollbackRecordRelativePath({ profile, serviceId, version });
}

export async function readRollbackRecord({ artifactRoot, profile, serviceId, version }) {
  const path = rollbackRecordAbsolutePath({ artifactRoot, profile, serviceId, version });
  let document;
  try {
    document = JSON.parse(await readFile(path, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(
        `no rollback record at ${path}; run skiff deploy first or pass --to <build-id>`,
      );
    }
    throw new Error(`failed to read rollback record ${path}: ${formatMessage(error)}`, {
      cause: error,
    });
  }
  const label = `rollback record ${path}`;
  if (!isPlainObject(document)) {
    throw new Error(`${label} must contain a JSON object`);
  }
  if (document.schemaVersion !== DEPLOY_ROLLBACK_SCHEMA_VERSION) {
    throw new Error(`${label} schemaVersion must be ${DEPLOY_ROLLBACK_SCHEMA_VERSION}`);
  }
  if (
    document.profile !== profile
    || document.serviceId !== serviceId
    || document.version !== version
  ) {
    throw new Error(
      `${label} targets ${document.profile} ${document.serviceId} ${document.version}; requested ${profile} ${serviceId} ${version}`,
    );
  }
  validateBuildId(document.buildId);
  if (document.previousPointer !== null) {
    assertRollbackPointer(document.previousPointer, label);
  }
  return { ...document, recordPath: rollbackRecordRelativePath({ profile, serviceId, version }) };
}

export async function deploymentRecordExists({ artifactRoot, recordPath }) {
  if (typeof recordPath !== 'string' || recordPath.length === 0) {
    throw new Error('deployment record path must be a non-empty string');
  }
  const resolved = resolve(artifactRoot, recordPath);
  const root = resolve(artifactRoot);
  if (resolved !== root && !resolved.startsWith(`${root}${sep}`)) {
    throw new Error(`deployment record path escapes the artifact root: ${recordPath}`);
  }
  try {
    return (await lstat(resolved)).isFile();
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw new Error(`failed to inspect deployment record ${recordPath}: ${formatMessage(error)}`, {
      cause: error,
    });
  }
}

export async function readHealth({ controlUrl, fetchImpl }) {
  const url = healthUrl(controlUrl);
  let response;
  try {
    response = await fetchImpl(url);
  } catch (error) {
    throw new Error(`router health unreachable at ${url}: ${formatMessage(error)}`);
  }
  if (!response.ok) {
    throw new Error(`router health returned HTTP ${response.status} at ${url}`);
  }
  let body;
  try {
    body = await response.json();
  } catch (error) {
    throw new Error(`router health returned invalid JSON at ${url}: ${formatMessage(error)}`);
  }
  const active = body?.activeAssembly;
  if (!isPlainObject(active)) {
    throw new Error(`router health at ${url} has no activeAssembly projection`);
  }
  if (typeof active.profile !== 'string' || !Array.isArray(active.buildIds)) {
    throw new Error(
      `router health activeAssembly at ${url} must contain profile and buildIds`,
    );
  }
  return {
    ok: body?.ok === true,
    profile: active.profile,
    releaseCount: typeof active.releaseCount === 'number' ? active.releaseCount : null,
    buildIds: active.buildIds,
    loadedBuildIds: Array.isArray(active.loadedBuildIds) ? active.loadedBuildIds : [],
  };
}

export async function pollHealthForBuildId({
  controlUrl,
  profile,
  buildId,
  timeoutMs,
  fetchImpl,
  delay,
}) {
  const startedAt = Date.now();
  let attempts = 0;
  let lastFailure = null;
  while (Date.now() - startedAt < timeoutMs) {
    attempts += 1;
    try {
      const health = await readHealth({ controlUrl, fetchImpl });
      if (health.ok && health.profile === profile && health.buildIds.includes(buildId)) {
        return { ok: true, attempts, elapsedMs: Date.now() - startedAt };
      }
    } catch (error) {
      lastFailure = formatMessage(error);
    }
    await delay(HEALTH_POLL_INTERVAL_MS);
  }
  throw new Error(
    `deploy verification timed out after ${timeoutMs}ms: buildId ${buildId} is not servable at ${healthUrl(controlUrl)} (${attempts} attempts${lastFailure ? `; last failure: ${lastFailure}` : ''})`,
  );
}

export function renderDeployReceipt(result) {
  const key = `${result.profile} ${result.serviceId} ${result.version}`;
  const lines = [
    `deployed ${key}`,
    `  -> ${result.buildId}`,
    `  from: ${result.previousBuildId ?? '(none)'}`,
    `pointer: ${result.pointerPath}`,
    `rollback record: ${result.rollbackRecordPath}`,
  ];
  const verification = result.verify;
  if (verification?.skipped === true) {
    lines.push('verify: skipped');
  } else if (verification?.ok === true) {
    lines.push(`verify: ok (${verification.attempts} attempts, ${verification.elapsedMs}ms)`);
  } else {
    lines.push(`verify: ${JSON.stringify(verification ?? null)}`);
  }
  return lines.join('\n');
}

export function renderVerifyReceipt(result) {
  const key = `${result.profile} ${result.serviceId} ${result.version}`;
  const health = result.health ?? {};
  return [
    `verified ${key}`,
    `  -> ${result.buildId}`,
    `  record: ${result.recordPath}`,
    `  health: reachable, profile ${health.profile ?? '?'}, releaseCount ${health.releaseCount ?? '?'}, status ${health.status ?? '?'}`,
  ].join('\n');
}

export function renderRollbackReceipt(result) {
  const key = `${result.profile} ${result.serviceId} ${result.version}`;
  const lines = [
    `rolled back ${key}`,
    `  from: ${result.fromBuildId}`,
    `  to: ${result.toBuildId}`,
    `  source: ${result.source}`,
  ];
  if (result.rollbackRecordPath !== null) {
    lines.push(`rollback record: ${result.rollbackRecordPath}`);
  }
  lines.push(`pointer: ${result.pointerPath}`);
  return lines.join('\n');
}

async function runDeployAction(parsed, { skiffRoot, stdout, runCompiler, publish, fetchImpl, delay }) {
  const previousReceipt = await readReleasePointer(parsed, { skiffRoot, runCompiler });
  const previousPointer = previousReceipt?.pointer ?? null;
  const previousBuildId = previousPointer?.deployment?.deploymentArtifactIdentity ?? null;

  const publishReceipt = await (publish ?? defaultPublish)({
    skiffRoot,
    root: parsed.root,
    artifactRoot: parsed.artifactRoot,
    profile: parsed.profile,
  });
  const { deployment, releasePointer, pointerPath } = parseDeployReceipt(publishReceipt, parsed);
  const buildId = deployment.deploymentArtifactIdentity;

  const rollbackRecordPath = await writeRollbackRecord(
    {
      artifactRoot: parsed.artifactRoot,
      profile: parsed.profile,
      serviceId: parsed.serviceId,
      version: parsed.version,
    },
    {
      schemaVersion: DEPLOY_ROLLBACK_SCHEMA_VERSION,
      profile: parsed.profile,
      serviceId: parsed.serviceId,
      version: parsed.version,
      deployedAt: new Date().toISOString(),
      buildId,
      deployment,
      previousPointer,
    },
  );

  let verification;
  if (parsed.skipVerify) {
    verification = { skipped: true };
  } else {
    verification = await pollHealthForBuildId({
      controlUrl: resolveControlUrl(parsed),
      profile: parsed.profile,
      buildId,
      timeoutMs: parsed.verifyTimeoutMs,
      fetchImpl,
      delay,
    });
  }

  const result = {
    action: 'deploy',
    profile: parsed.profile,
    serviceId: parsed.serviceId,
    version: parsed.version,
    buildId,
    deployment,
    releasePointer,
    pointerPath,
    previousBuildId,
    rollbackRecordPath,
    verify: verification,
  };
  stdout(parsed.json ? JSON.stringify(result, null, 2) : renderDeployReceipt(result));
  return result;
}

async function runVerifyAction(parsed, { skiffRoot, stdout, runCompiler, fetchImpl }) {
  const receipt = await readReleasePointer(parsed, { skiffRoot, runCompiler });
  const pointer = receipt?.pointer ?? null;
  if (pointer === null) {
    throw new Error(`release pointer for ${parsed.serviceId}@${parsed.version} is not set`);
  }
  if (pointer.profile !== parsed.profile) {
    throw new Error(
      `release pointer profile ${JSON.stringify(pointer.profile)} does not match ${JSON.stringify(parsed.profile)}`,
    );
  }
  const deployment = pointer.deployment;
  if (
    !isPlainObject(deployment)
    || deployment.serviceId !== parsed.serviceId
    || deployment.contractVersion !== parsed.version
  ) {
    throw new Error('release pointer deployment does not match the requested service and version');
  }
  const buildId = deployment.deploymentArtifactIdentity;
  validateBuildId(buildId);
  const recordPath = pointer.recordPath;
  if (typeof recordPath !== 'string' || recordPath.length === 0) {
    throw new Error('release pointer has no deployment record path');
  }
  const recordExists = await deploymentRecordExists({
    artifactRoot: parsed.artifactRoot,
    recordPath,
  });
  if (!recordExists) {
    throw new Error(`deployment record missing for ${buildId}: ${recordPath}`);
  }

  const controlUrl = resolveControlUrl(parsed);
  const health = await readHealth({ controlUrl, fetchImpl });
  if (health.profile !== parsed.profile) {
    throw new Error(
      `router health activeAssembly profile ${JSON.stringify(health.profile)} does not match ${JSON.stringify(parsed.profile)}`,
    );
  }
  const loaded = health.buildIds.includes(buildId);
  const result = {
    action: 'verify',
    profile: parsed.profile,
    serviceId: parsed.serviceId,
    version: parsed.version,
    buildId,
    pointer,
    pointerPath: receipt?.pointerPath ?? null,
    recordPath,
    health: {
      reachable: true,
      ok: health.ok,
      profile: health.profile,
      releaseCount: health.releaseCount,
      buildIds: health.buildIds,
      loadedBuildIds: health.loadedBuildIds,
      loaded,
      status: loaded ? 'loaded' : 'resolvable',
    },
  };
  stdout(parsed.json ? JSON.stringify(result, null, 2) : renderVerifyReceipt(result));
  return result;
}

async function runRollbackAction(parsed, { skiffRoot, stdout, runCompiler }) {
  const currentReceipt = await readReleasePointer(parsed, { skiffRoot, runCompiler });
  const currentPointer = currentReceipt?.pointer ?? null;
  if (currentPointer === null) {
    throw new Error(
      `no current release pointer for ${parsed.serviceId}@${parsed.version}; nothing to roll back`,
    );
  }
  const fromBuildId = currentPointer.deployment?.deploymentArtifactIdentity ?? null;
  validateBuildId(fromBuildId);

  let targetBuildId;
  let source;
  let rollbackRecordPath = null;
  if (parsed.toBuildId !== undefined) {
    targetBuildId = parsed.toBuildId;
    source = 'explicit';
  } else {
    const record = await readRollbackRecord({
      artifactRoot: parsed.artifactRoot,
      profile: parsed.profile,
      serviceId: parsed.serviceId,
      version: parsed.version,
    });
    rollbackRecordPath = record.recordPath;
    if (record.previousPointer === null) {
      throw new Error(
        `no previous buildId recorded for ${parsed.serviceId}@${parsed.version}; use --to <build-id>`,
      );
    }
    const previous = record.previousPointer;
    const previousDeployment = previous.deployment;
    if (
      previous.profile !== parsed.profile
      || previousDeployment.serviceId !== parsed.serviceId
      || previousDeployment.contractVersion !== parsed.version
    ) {
      throw new Error('rollback record deployment does not match the requested service and version');
    }
    targetBuildId = previousDeployment.deploymentArtifactIdentity;
    validateBuildId(targetBuildId);
    source = 'rollback-record';
  }

  const setReceipt = await runReleaseCommand([
    'set',
    '--artifact-root',
    parsed.artifactRoot,
    '--profile',
    parsed.profile,
    '--service',
    parsed.serviceId,
    '--version',
    parsed.version,
    '--build-id',
    targetBuildId,
    '--expected',
    JSON.stringify(currentPointer),
    '--json',
  ], { skiffRoot, stdout: () => {}, runCompiler });

  const result = {
    action: 'rollback',
    profile: parsed.profile,
    serviceId: parsed.serviceId,
    version: parsed.version,
    fromBuildId,
    toBuildId: targetBuildId,
    source,
    rollbackRecordPath,
    pointer: setReceipt?.pointer ?? null,
    pointerPath: setReceipt?.pointerPath ?? null,
  };
  stdout(parsed.json ? JSON.stringify(result, null, 2) : renderRollbackReceipt(result));
  return result;
}

const defaultPublish = ({ skiffRoot, root, artifactRoot, profile }) =>
  runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'publish',
    root,
    artifactRoot,
    profile,
  });

function parseDeployReceipt(publishReceipt, parsed) {
  const serviceId = publishReceipt?.serviceApiReceipt?.serviceId;
  if (serviceId !== parsed.serviceId) {
    throw new Error(
      `package publish produced service ${JSON.stringify(serviceId ?? null)}, expected ${JSON.stringify(parsed.serviceId)}`,
    );
  }
  const deployment = publishReceipt?.serviceDeploymentReceipt?.deployment;
  if (
    !isPlainObject(deployment)
    || deployment.serviceId !== parsed.serviceId
    || deployment.contractVersion !== parsed.version
  ) {
    throw new Error(
      `package publish deployment does not match ${parsed.serviceId}@${parsed.version}`,
    );
  }
  const releasePointer = publishReceipt?.releasePointerReceipt?.pointer;
  const pointerPath = publishReceipt?.releasePointerReceipt?.pointerPath;
  if (
    !isPlainObject(releasePointer)
    || typeof pointerPath !== 'string'
    || pointerPath.length === 0
  ) {
    throw new Error(
      `package publish for ${parsed.serviceId}@${parsed.version} did not return a release pointer receipt`,
    );
  }
  const buildId = releasePointer.deployment?.deploymentArtifactIdentity;
  validateBuildId(buildId);
  if (releasePointer.profile !== parsed.profile) {
    throw new Error(
      `package publish release pointer profile ${JSON.stringify(releasePointer.profile)} does not match ${JSON.stringify(parsed.profile)}`,
    );
  }
  if (buildId !== deployment.deploymentArtifactIdentity) {
    throw new Error('package publish release pointer buildId does not match its deployment receipt');
  }
  return { deployment, releasePointer, pointerPath };
}

function assertRollbackPointer(pointer, label) {
  if (!isPlainObject(pointer)) {
    throw new Error(`${label} previousPointer must be a ReleasePointer object or null`);
  }
  const deployment = pointer.deployment;
  if (
    typeof pointer.schemaVersion !== 'string'
    || pointer.schemaVersion.length === 0
    || typeof pointer.profile !== 'string'
    || pointer.profile.length === 0
    || typeof pointer.recordPath !== 'string'
    || pointer.recordPath.length === 0
    || !isPlainObject(deployment)
  ) {
    throw new Error(`${label} previousPointer is not an exact ReleasePointer`);
  }
  for (const field of ['serviceId', 'contractVersion', 'deploymentRevision']) {
    if (typeof deployment[field] !== 'string' || deployment[field].length === 0) {
      throw new Error(`${label} previousPointer.deployment.${field} must be a non-empty string`);
    }
  }
  validateBuildId(deployment.deploymentArtifactIdentity);
}

async function readReleasePointer(parsed, { skiffRoot, runCompiler }) {
  return runReleaseCommand([
    'get',
    '--artifact-root',
    parsed.artifactRoot,
    '--profile',
    parsed.profile,
    '--service',
    parsed.serviceId,
    '--version',
    parsed.version,
    '--json',
  ], { skiffRoot, stdout: () => {}, runCompiler });
}

function resolveControlUrl(parsed) {
  return normalizeControlUrl(
    parsed.controlUrl ?? process.env.SKIFF_DEV_CONTROL_URL ?? DEFAULT_CONTROL_URL,
  );
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatMessage(error) {
  return error?.message ?? String(error);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
