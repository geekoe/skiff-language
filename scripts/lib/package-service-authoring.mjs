import { randomUUID } from 'node:crypto';
import { isAbsolute, resolve } from 'node:path';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import { captureAttachedCommand } from './command-execution.mjs';

export const defaultAssemblyActivationUrl =
  'http://127.0.0.1:4001/__skiff/activate-assembly';
export const maxExpectedAssemblyGeneration = Number.MAX_SAFE_INTEGER - 1;

const activationEnvironmentPattern = /^[A-Za-z0-9._-]{1,200}$/;
const activationTokenPattern = /^[\x21-\x7e]{1,200}$/;
const runtimeAssemblyIdentityPattern =
  /^skiff-runtime-assembly-v1:sha256:[0-9a-f]{64}$/;

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
  const request = {
    schemaVersion: 'skiff-assembly-activation-request-v1',
    environment,
    activationId,
    expectedGeneration,
    assembly,
  };
  const response = await postActivation(fetchImpl, activationUrl, request, signal);
  return { request, response };
}

export async function runCompilerAuthoring({
  skiffRoot,
  kind,
  action,
  root,
  artifactRoot,
  environment = 'dev',
}) {
  if (kind !== 'package' && kind !== 'assembly') {
    throw new Error(`unsupported authoring object ${kind}; ServiceContract and ServiceDeployment are generated by package ${action}`);
  }
  const invocation = compilerAuthoringInvocation({
    skiffRoot,
    kind,
    action,
    root,
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

export function compilerAuthoringInvocation({
  skiffRoot,
  kind,
  action,
  root,
  artifactRoot,
  environment = 'dev',
}) {
  if (!isAbsolute(skiffRoot)) {
    throw new Error('compiler authoring requires an absolute skiffRoot');
  }
  return {
    command: 'cargo',
    cwd: skiffRoot,
    args: [
      'run',
      '--quiet',
      '--manifest-path',
      resolve(skiffRoot, 'compiler', 'Cargo.toml'),
      '--bin',
      'skiff-compiler',
      '--',
      kind,
      action,
      root,
      '--artifact-root',
      artifactRoot,
      '--environment',
      environment,
      '--platform-source-root',
      skiffRoot,
      '--json',
    ],
  };
}

export function parseObjectArgs(kind, action, rawArgs) {
  const options = new Map();
  const flags = new Set();
  let root;
  const optionsWithValues = new Set(['--artifact-root', '--environment']);
  if (action === 'activate') {
    optionsWithValues.add('--activation-url');
    optionsWithValues.add('--activation-id');
    optionsWithValues.add('--expected-generation');
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
    if (root !== undefined) {
      throw new Error(`unexpected argument ${argument}`);
    }
    root = resolve(argument);
  }
  if (root === undefined) {
    throw new Error(`skiff ${kind} ${action} requires a root`);
  }
  const artifactRoot = options.get('--artifact-root');
  if (artifactRoot === undefined) {
    throw new Error(`skiff ${kind} ${action} requires --artifact-root`);
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
  }
  return {
    root,
    artifactRoot: resolve(artifactRoot),
    environment: options.get('--environment') ?? 'dev',
    activationUrl: options.get('--activation-url') ?? defaultAssemblyActivationUrl,
    activationId: options.get('--activation-id'),
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
    `usage: ${base}`,
    '       skiff assembly activate <root> --artifact-root <dir> --expected-generation <n> [--activation-url <url>] [--activation-id <id>] [--json]',
  ].join('\n');
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
