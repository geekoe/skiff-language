import { randomUUID } from 'node:crypto';
import { lstat } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import { captureAttachedCommand } from './command-execution.mjs';

export const defaultAssemblyActivationUrl =
  'http://127.0.0.1:4001/__skiff/activate-assembly';
export const maxExpectedAssemblyGeneration = Number.MAX_SAFE_INTEGER - 1;

const activationEnvironmentPattern = /^[A-Za-z0-9._-]{1,200}$/;
const activationTokenPattern = /^[\x21-\x7e]{1,200}$/;
const runtimeAssemblyIdentityPattern =
  /^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/;
const runtimeConfigSnapshotIdPattern =
  /^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/;

export async function runAuthoringObjectCommand(kind, rawArgs, {
  skiffRoot,
  fetchImpl = fetch,
  stdout = console.log,
} = {}) {
  const action = rawArgs[0];
  if (action === '-h' || action === '--help') {
    stdout(objectUsage(kind));
    return null;
  }
  if (action !== 'build' && action !== 'publish' && !(kind === 'assembly' && action === 'activate')) {
    throw new Error(`unknown ${kind} command ${action || '(missing)'}\n${objectUsage(kind)}`);
  }
  const parsed = parseObjectArgs(kind, action, rawArgs.slice(1));
  const compilerAction = action === 'activate' ? 'build' : action;
  const receipt = await runCompilerAuthoring({
    skiffRoot,
    kind,
    action: compilerAction,
    root: parsed.root,
    rootDeployments: parsed.rootDeployments,
    artifactRoot: parsed.artifactRoot,
    environment: parsed.environment,
  });

  let result = receipt;
  if (action === 'activate') {
    const assembly = receipt?.runtimeAssemblyReceipt?.assembly;
    const environment = receipt?.runtimeAssemblyReceipt?.environment;
    if (!isPlainObject(assembly) || typeof environment !== 'string') {
      throw new Error('compiler assembly build did not return a typed RuntimeAssembly receipt');
    }
    const { request, response } = await requestAssemblyActivation({
      fetchImpl,
      activationUrl: parsed.activationUrl,
      activationId: parsed.activationId,
      expectedGeneration: parsed.expectedGeneration,
      environment,
      assembly,
      configSnapshot: parsed.configSnapshot,
    });
    result = {
      ...receipt,
      assemblyActivationReceipt: {
        request,
        response,
      },
    };
  }

  stdout(parsed.json ? JSON.stringify(result, null, 2) : renderAuthoringResult(result));
  return result;
}

export function renderAuthoringResult(result) {
  const receipt = result?.serviceApiReceipt;
  const functions = receipt?.projection?.functions;
  if (!Array.isArray(functions)) {
    return JSON.stringify(result);
  }
  const available = functions.filter(
    (entry) => entry?.status === 'available' && typeof entry.serviceOperationId === 'string',
  );
  const packageAvailable = functions.filter(
    (entry) => entry?.status === 'available' && entry.serviceOperationId === undefined,
  );
  const unavailable = functions.filter((entry) => entry?.status === 'unavailable');
  if (available.length + packageAvailable.length + unavailable.length !== functions.length) {
    throw new Error('compiler returned an invalid service API projection status');
  }
  const owner = receipt.serviceId ?? '<package only>';
  const lines = [
    `Service API for ${owner}`,
    `Available: ${available.length}`,
    `Package-only: ${packageAvailable.length + unavailable.length}`,
  ];
  for (const entry of functions) {
    if (entry.status === 'available' && typeof entry.serviceOperationId === 'string') {
      lines.push(`  available ${entry.publicPath}`);
      continue;
    }
    if (entry.status === 'available') {
      lines.push(`  package-only ${entry.publicPath}`);
      continue;
    }
    lines.push(`  package-only ${entry.publicPath}`);
    for (const reason of entry.reasons ?? []) {
      lines.push(`    - ${JSON.stringify(reason)}`);
    }
  }
  return lines.join('\n');
}

export async function requestAssemblyActivation({
  fetchImpl = fetch,
  activationUrl = defaultAssemblyActivationUrl,
  activationId = `skiff-${randomUUID()}`,
  expectedGeneration,
  environment,
  assembly,
  configSnapshot,
  signal,
}) {
  if (
    typeof expectedGeneration !== 'number'
    || Object.is(expectedGeneration, -0)
    || !Number.isSafeInteger(expectedGeneration)
    || expectedGeneration < 0
    || expectedGeneration > maxExpectedAssemblyGeneration
  ) {
    throw new Error(
      `assembly activation expectedGeneration must be between 0 and ${maxExpectedAssemblyGeneration}`,
    );
  }
  if (
    typeof environment !== 'string'
    || environment === '.'
    || environment === '..'
    || !activationEnvironmentPattern.test(environment)
  ) {
    throw new Error('assembly activation environment must be a canonical ASCII environment token');
  }
  if (typeof activationId !== 'string' || !activationTokenPattern.test(activationId)) {
    throw new Error('assembly activation activationId must be an ASCII visible token between 1 and 200 bytes');
  }
  if (
    !isPlainObject(assembly)
    || Object.keys(assembly).length !== 1
    || typeof assembly.assemblyIdentity !== 'string'
    || !runtimeAssemblyIdentityPattern.test(assembly.assemblyIdentity)
  ) {
    throw new Error('assembly activation requires an exact RuntimeAssembly reference');
  }
  if (
    !isPlainObject(configSnapshot)
    || Object.keys(configSnapshot).length !== 1
    || typeof configSnapshot.snapshotId !== 'string'
    || !runtimeConfigSnapshotIdPattern.test(configSnapshot.snapshotId)
  ) {
    throw new Error('assembly activation requires an exact RuntimeConfigSnapshot reference');
  }
  const request = {
    schemaVersion: 'skiff-assembly-activation-request-v2',
    environment,
    activationId,
    expectedGeneration,
    assembly,
    configSnapshot,
  };
  const response = await postActivation(fetchImpl, activationUrl, request, signal);
  return { request, response };
}

export async function runCompilerAuthoring({
  skiffRoot,
  kind,
  action,
  root,
  rootDeployments,
  artifactRoot,
  environment,
}) {
  if (kind !== 'package' && kind !== 'assembly') {
    throw new Error(`unsupported authoring object ${kind}; ServiceContract and ServiceDeployment are generated by package ${action}`);
  }
  const invocation = compilerAuthoringInvocation({
    skiffRoot,
    kind,
    action,
    root,
    rootDeployments,
    artifactRoot,
    environment,
  });
  const outcome = await captureAttachedCommand(invocation.command, invocation.args, {
    cwd: invocation.cwd,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: cargoTargetDir(skiffRoot),
    },
  });
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    const detail = outcome.stderr.trim() || outcome.stdout.trim()
      || outcome.error?.message || `cargo exited ${outcome.signal ?? outcome.code}`;
    throw new Error(`${kind} ${action} failed: ${detail}`);
  }
  try {
    return JSON.parse(outcome.stdout);
  } catch (error) {
    throw new Error(`${kind} ${action} returned invalid JSON: ${error.message}`);
  }
}

export async function runConfigSnapshotAuthoring({
  skiffRoot,
  artifactRoot,
  environment,
  profile,
  assemblyRecord,
  sources,
}) {
  if (!isAbsolute(skiffRoot) || !isAbsolute(artifactRoot)) {
    throw new Error('config snapshot authoring requires absolute skiffRoot and artifactRoot');
  }
  if (
    typeof profile !== 'string'
    || profile === '.'
    || profile === '..'
    || !activationEnvironmentPattern.test(profile)
  ) {
    throw new Error('config snapshot authoring requires an explicit canonical profile');
  }
  if (
    typeof environment !== 'string'
    || environment === '.'
    || environment === '..'
    || !activationEnvironmentPattern.test(environment)
  ) {
    throw new Error('config snapshot authoring requires an explicit canonical target environment');
  }
  if (typeof assemblyRecord !== 'string' || assemblyRecord.length === 0) {
    throw new Error('config snapshot authoring requires the RuntimeAssembly record path');
  }
  if (!Array.isArray(sources) || sources.length === 0) {
    throw new Error('config snapshot authoring requires at least one service config source');
  }
  const args = [
    'run',
    '--quiet',
    '--manifest-path',
    resolve(skiffRoot, 'config-snapshot-tooling', 'Cargo.toml'),
    '--',
    '--artifact-root',
    artifactRoot,
    '--assembly-record',
    assemblyRecord,
    '--environment',
    environment,
    '--profile',
    profile,
  ];
  for (const source of sources) {
    if (
      !isPlainObject(source)
      || !isAbsolute(source.root)
      || !isPlainObject(source.deployment)
    ) {
      throw new Error('config snapshot source requires an absolute root and exact deployment');
    }
    args.push('--source', JSON.stringify(source));
  }
  await verifySecretConfigSources(profile, sources);
  const outcome = await captureAttachedCommand('cargo', args, {
    cwd: skiffRoot,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: cargoTargetDir(skiffRoot),
    },
  });
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    const detail = outcome.stderr.trim() || outcome.stdout.trim()
      || outcome.error?.message || `cargo exited ${outcome.signal ?? outcome.code}`;
    throw new Error(`config snapshot production failed: ${detail}`);
  }
  let result;
  try {
    result = JSON.parse(outcome.stdout);
  } catch (error) {
    throw new Error(`config snapshot production returned invalid JSON: ${error.message}`);
  }
  const reference = result?.runtimeConfigSnapshotReceipt?.snapshot;
  if (
    !isPlainObject(reference)
    || Object.keys(reference).length !== 1
    || typeof reference.snapshotId !== 'string'
    || !runtimeConfigSnapshotIdPattern.test(reference.snapshotId)
  ) {
    throw new Error('config snapshot production did not return an exact snapshot reference');
  }
  return result;
}

async function verifySecretConfigSources(profile, sources) {
  const filename = `config.${profile}.secret.yml`;
  for (const source of sources) {
    const path = join(source.root, filename);
    let metadata;
    try {
      metadata = await lstat(path);
    } catch (error) {
      if (error?.code === 'ENOENT') {
        continue;
      }
      throw new Error(`failed to inspect secret config ${path}`, { cause: error });
    }
    if (metadata.isSymbolicLink() || !metadata.isFile()) {
      throw new Error(`secret config ${path} must be a regular file, not a symlink`);
    }
    if (process.platform !== 'win32' && (metadata.mode & 0o7777) !== 0o600) {
      throw new Error(
        `secret config ${path} permissions must be 0600; run \`chmod 600 <path>\` before retrying`,
      );
    }
  }
}

export function compilerAuthoringInvocation({
  skiffRoot,
  kind,
  action,
  root,
  rootDeployments,
  artifactRoot,
  environment,
}) {
  if (!isAbsolute(skiffRoot)) {
    throw new Error('compiler authoring requires an absolute skiffRoot');
  }
  const args = [
    'run',
    '--quiet',
    '--manifest-path',
    resolve(skiffRoot, 'compiler', 'Cargo.toml'),
    '--bin',
    'skiff-compiler',
    '--',
    kind,
    action,
  ];
  if (kind === 'package') {
    const packageEnvironment = environment ?? 'dev';
    args.push(
      root,
      '--artifact-root',
      artifactRoot,
      '--environment',
      packageEnvironment,
      '--platform-source-root',
      skiffRoot,
      '--json',
    );
  } else {
    if (typeof environment !== 'string' || environment.length === 0) {
      throw new Error('assembly authoring requires an explicit environment');
    }
    const references = normalizeRootDeployments(rootDeployments);
    args.push('--artifact-root', artifactRoot, '--environment', environment);
    for (const reference of references) {
      args.push('--root-deployment', JSON.stringify(reference));
    }
    args.push('--json');
  }
  return { command: 'cargo', cwd: skiffRoot, args };
}

export function parseObjectArgs(kind, action, rawArgs) {
  const options = new Map();
  const flags = new Set();
  const rootDeployments = [];
  let root;
  const optionsWithValues = new Set(['--artifact-root', '--environment']);
  if (kind === 'assembly') {
    optionsWithValues.add('--root-deployment');
  }
  if (action === 'activate') {
    optionsWithValues.add('--activation-url');
    optionsWithValues.add('--activation-id');
    optionsWithValues.add('--expected-generation');
    optionsWithValues.add('--config-snapshot');
  }
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
      if (option !== '--root-deployment' && options.has(option)) {
        throw new Error(`${option} was provided more than once`);
      }
      const value = equals === -1 ? rawArgs[index + 1] : argument.slice(equals + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${option} requires a value`);
      }
      if (option === '--root-deployment') {
        rootDeployments.push(parseRootDeployment(value));
      } else if (option === '--config-snapshot') {
        options.set(option, parseConfigSnapshotRef(value));
      } else {
        options.set(option, value);
      }
      if (equals === -1) {
        index += 1;
      }
      continue;
    }
    if (argument.startsWith('-')) {
      throw new Error(`unknown option ${argument}`);
    }
    if (kind === 'assembly') {
      throw new Error(
        `skiff assembly ${action} does not accept a positional root; use --root-deployment`,
      );
    }
    if (root !== undefined) {
      throw new Error(`unexpected argument ${argument}`);
    }
    root = resolve(argument);
  }
  if (kind === 'package' && root === undefined) {
    throw new Error(`skiff ${kind} ${action} requires a root`);
  }
  if (kind === 'assembly' && rootDeployments.length === 0) {
    throw new Error(`skiff assembly ${action} requires at least one --root-deployment`);
  }
  const normalizedRootDeployments = kind === 'assembly'
    ? normalizeRootDeployments(rootDeployments)
    : undefined;
  const artifactRoot = options.get('--artifact-root');
  if (artifactRoot === undefined) {
    throw new Error(`skiff ${kind} ${action} requires --artifact-root`);
  }
  if (kind === 'assembly' && options.get('--environment') === undefined) {
    throw new Error(`skiff assembly ${action} requires --environment`);
  }
  let expectedGeneration;
  if (action === 'activate') {
    const rawGeneration = options.get('--expected-generation');
    if (rawGeneration === undefined || !/^(?:0|[1-9][0-9]*)$/.test(rawGeneration)) {
      throw new Error('skiff assembly activate requires a non-negative integer --expected-generation');
    }
    expectedGeneration = Number(rawGeneration);
    if (
      !Number.isSafeInteger(expectedGeneration)
      || expectedGeneration > maxExpectedAssemblyGeneration
    ) {
      throw new Error(`--expected-generation must not exceed ${maxExpectedAssemblyGeneration}`);
    }
    const activationId = options.get('--activation-id');
    if (activationId !== undefined && !activationTokenPattern.test(activationId)) {
      throw new Error('--activation-id must be an ASCII visible token between 1 and 200 bytes');
    }
    if (options.get('--config-snapshot') === undefined) {
      throw new Error('skiff assembly activate requires --config-snapshot');
    }
  }
  return {
    root,
    rootDeployments: normalizedRootDeployments,
    artifactRoot: resolve(artifactRoot),
    environment: options.get('--environment') ?? 'dev',
    activationUrl: options.get('--activation-url') ?? defaultAssemblyActivationUrl,
    activationId: options.get('--activation-id'),
    configSnapshot: options.get('--config-snapshot'),
    expectedGeneration,
    json: flags.has('--json'),
  };
}

export function objectUsage(kind) {
  const base = `skiff ${kind} <build|publish> <root> --artifact-root <dir> [--environment <name>] [--json]`;
  if (kind !== 'assembly') {
    return `usage: ${base}`;
  }
  return [
    "usage: skiff assembly <build|publish> --artifact-root <dir> --environment <name> --root-deployment '<exact ServiceDeploymentRef JSON>'... [--json]",
    "       skiff assembly activate --artifact-root <dir> --environment <name> --root-deployment '<exact ServiceDeploymentRef JSON>'... --config-snapshot '<exact RuntimeConfigSnapshotRef JSON>' --expected-generation <n> [--activation-url <url>] [--activation-id <id>] [--json]",
  ].join('\n');
}

function parseRootDeployment(source) {
  let value;
  try {
    value = JSON.parse(source);
  } catch (error) {
    throw new Error(`--root-deployment requires exact ServiceDeploymentRef JSON: ${error.message}`);
  }
  return normalizeRootDeployment(value, '--root-deployment');
}

function parseConfigSnapshotRef(source) {
  let value;
  try {
    value = JSON.parse(source);
  } catch (error) {
    throw new Error(`--config-snapshot requires exact RuntimeConfigSnapshotRef JSON: ${error.message}`);
  }
  if (
    !isPlainObject(value)
    || Object.keys(value).length !== 1
    || typeof value.snapshotId !== 'string'
    || !runtimeConfigSnapshotIdPattern.test(value.snapshotId)
  ) {
    throw new Error('--config-snapshot must be an exact RuntimeConfigSnapshotRef object');
  }
  return { snapshotId: value.snapshotId };
}

function normalizeRootDeployments(values) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new Error('assembly authoring requires at least one exact root deployment');
  }
  const normalized = values.map((value, index) => (
    normalizeRootDeployment(value, `root deployment ${index}`)
  ));
  const seen = new Set();
  for (const reference of normalized) {
    const key = JSON.stringify(reference);
    if (seen.has(key)) {
      throw new Error(
        `assembly authoring root deployment set contains duplicate exact reference ${key}`,
      );
    }
    seen.add(key);
  }
  return normalized;
}

function normalizeRootDeployment(value, label) {
  const fields = [
    'contractVersion',
    'deploymentArtifactIdentity',
    'deploymentRevision',
    'serviceId',
  ];
  if (!isPlainObject(value)) {
    throw new Error(`${label} must be an exact ServiceDeploymentRef object`);
  }
  const actual = Object.keys(value).sort();
  if (
    actual.length !== fields.length
    || actual.some((field, index) => field !== fields[index])
  ) {
    throw new Error(`${label} fields must be exactly ${fields.join(', ')}`);
  }
  for (const field of fields) {
    if (
      typeof value[field] !== 'string'
      || value[field].length === 0
      || value[field].trim() !== value[field]
    ) {
      throw new Error(`${label}.${field} must be a non-empty trimmed string`);
    }
  }
  return Object.fromEntries(fields.map((field) => [field, value[field]]));
}

async function postActivation(fetchImpl, url, request, signal) {
  let response;
  try {
    response = await fetchImpl(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
      signal,
    });
  } catch (error) {
    if (signal?.aborted) throw signal.reason;
    throw new Error(`assembly activation request failed for ${url}: ${error.message}`);
  }
  const text = await response.text();
  let body = null;
  if (text.trim().length > 0) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  if (!response.ok) {
    const detail = typeof body === 'string' ? body : JSON.stringify(body);
    throw new Error(`assembly activation rejected with HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
  }
  return body;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
