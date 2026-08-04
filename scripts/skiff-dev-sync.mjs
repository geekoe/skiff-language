#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  defaultAssemblyActivationUrl,
  runConfigSnapshotAuthoring,
  runCompilerAuthoring,
} from './lib/package-service-authoring.mjs';
import { activateDevAssembly } from './lib/dev-assembly-activation.mjs';
export {
  activateDevAssembly,
  readRouterActivationState,
} from './lib/dev-assembly-activation.mjs';
import {
  assertProfile,
  devRegistrySchemaVersion,
  readStoredDevRegistry,
  writeStoredDevRegistry,
} from './lib/dev-registry-store.mjs';
import {
  defaultBuildStatusPath,
  summarizeBuildError,
  writeBuildStatus,
} from './lib/dev-sync-build-status.mjs';
import { parseServiceManifestIdentity } from './lib/service-manifest-identity.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const defaultDevHome = resolve(process.env.SKIFF_DEV_HOME ?? join(skiffRoot, '.stack', 'dev-home'));
const defaultRegistryPath = join(defaultDevHome, 'watch.json');
const defaultArtifactRoot = join(defaultDevHome, 'artifacts');
const ignoredDirectories = new Set(['.git', 'build', 'node_modules', 'target']);
const devBuildStates = new WeakMap();

const usage = `usage: node skiff-dev-sync.mjs [--watch] [--root <package-root>]... [--config <path>] [--artifact-root <dir>] [--profile <name>] [--activation-url <url>] [--activation-id <id>] [--poll-interval-ms <ms>] [--build-only] [--json]`;

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main(process.argv.slice(2));
  } catch (error) {
    console.error(`error: ${formatError(error)}`);
    process.exitCode = 1;
  }
}

export async function main(rawArgs, dependencies = {}) {
  const options = parseDevSyncArgs(rawArgs);
  if (options.help) {
    return null;
  }
  if (options.watch) {
    return runDevWatch(options, dependencies);
  }
  const registry = await readDevRegistry(options.config, {
    allowMissing: options.roots.length > 0,
  });
  const result = await runDevSyncOnce({
    roots: [...registry.roots, ...options.roots],
    profile: options.profile ?? registry.profile,
    artifactRoot: options.artifactRoot,
    activationUrl: options.activationUrl,
    activationId: options.activationId,
    buildOnly: options.buildOnly,
    skiffRoot: dependencies.skiffRoot ?? skiffRoot,
    fetchImpl: dependencies.fetchImpl ?? fetch,
    compilerRunner: dependencies.compilerRunner ?? runCompilerAuthoring,
    configSnapshotRunner: dependencies.configSnapshotRunner ?? runConfigSnapshotAuthoring,
    activationWait: dependencies.activationWait ?? delay,
  });
  printResult(result, options.json);
  return result;
}

export async function runDevWatch(options, dependencies = {}) {
  const wait = dependencies.wait ?? delay;
  const now = dependencies.now ?? Date.now;
  const reportError = dependencies.reportError
    ?? ((error) => console.error(`dev sync rejected: ${formatError(error)}`));
  const buildStatusPath = dependencies.buildStatusPath
    ?? defaultBuildStatusPath(options.config);
  const writeBuildStatusFile = dependencies.writeBuildStatusFile
    ?? writeBuildStatus;
  const syncRunner = dependencies.syncRunner ?? runDevSyncOnce;
  const buildStateFromResult =
    dependencies.buildStateFromResult ?? reusableDevBuildState;
  const emitResult = dependencies.printResult ?? printResult;
  let lastKnownRegistry;
  let lastRegistryError;
  let successful;
  let pending;
  let attempt = 0;
  let retryDelayMs = 1000;
  let retryAt = 0;
  let registryRetryDelayMs = 1000;
  let registryRetryAt = 0;
  console.error(
    `[dev-sync] watch mode: staying alive and retrying build failures with backoff; `
      + `build status: ${buildStatusPath}`,
  );
  const deferUntilFirstValidRegistry = (error) => {
    const signature = formatError(error);
    const shouldReport = signature !== lastRegistryError;
    lastRegistryError = signature;
    registryRetryAt = now() + registryRetryDelayMs;
    registryRetryDelayMs = Math.min(registryRetryDelayMs * 2, 30000);
    if (shouldReport) {
      reportError(
        new Error(`${signature}; waiting for the first valid dev registry`),
      );
    }
  };
  let first = true;
  for (;;) {
    if (!first) {
      await wait(options.pollIntervalMs);
    }
    first = false;
    if (lastKnownRegistry === undefined && now() < registryRetryAt) {
      continue;
    }

    let registry;
    let registryIsCurrent = true;
    try {
      registry = await readDevRegistry(options.config);
    } catch (error) {
      registryIsCurrent = false;
      if (lastKnownRegistry !== undefined) {
        registry = lastKnownRegistry;
        const signature = formatError(error);
        if (signature !== lastRegistryError) {
          reportError(
            new Error(`${signature}; continuing with the last known-good dev registry`),
          );
          lastRegistryError = signature;
        }
      } else {
        deferUntilFirstValidRegistry(error);
        continue;
      }
    }

    let desired;
    try {
      desired = await devWatchObservation({
        registryPath: options.config,
        registry,
        explicitRoots: options.roots,
        profileOverride: options.profile,
      });
    } catch (error) {
      if (lastKnownRegistry === undefined) {
        deferUntilFirstValidRegistry(error);
      } else {
        const signature = formatError(error);
        if (signature !== lastRegistryError) {
          reportError(
            new Error(`${signature}; continuing with the last known-good dev registry`),
          );
          lastRegistryError = signature;
        }
      }
      continue;
    }
    if (registryIsCurrent) {
      lastKnownRegistry = registry;
      lastRegistryError = undefined;
      registryRetryDelayMs = 1000;
      registryRetryAt = 0;
    }

    if (pending?.fingerprint !== desired.fingerprint) {
      if (successful?.fingerprint === desired.fingerprint) {
        pending = undefined;
      } else {
        pending = desired;
        retryDelayMs = 1000;
        retryAt = 0;
      }
    }
    if (pending === undefined || now() < retryAt) {
      continue;
    }

    try {
      const configOnly = successful?.codeFingerprint === pending.codeFingerprint;
      const result = await syncRunner({
        roots: pending.roots,
        profile: pending.profile,
        artifactRoot: options.artifactRoot,
        activationUrl: options.activationUrl,
        activationId: options.activationId,
        buildOnly: options.buildOnly,
        skiffRoot: dependencies.skiffRoot ?? skiffRoot,
        fetchImpl: dependencies.fetchImpl ?? fetch,
        compilerRunner: dependencies.compilerRunner ?? runCompilerAuthoring,
        configSnapshotRunner:
          dependencies.configSnapshotRunner ?? runConfigSnapshotAuthoring,
        activationWait: dependencies.activationWait ?? delay,
        buildState: configOnly ? successful.buildState : undefined,
      });
      successful = {
        ...pending,
        buildState: buildStateFromResult(result),
      };
      pending = undefined;
      attempt = 0;
      retryDelayMs = 1000;
      retryAt = 0;
      await writeBuildStatusFile({
        path: buildStatusPath,
        state: 'ok',
        updatedAt: new Date(now()).toISOString(),
        attempt,
      });
      emitResult(result, options.json);
    } catch (error) {
      reportError(error);
      attempt += 1;
      retryAt = now() + retryDelayMs;
      retryDelayMs = Math.min(retryDelayMs * 2, 30000);
      await writeBuildStatusFile({
        path: buildStatusPath,
        state: 'failed',
        updatedAt: new Date(now()).toISOString(),
        nextRetryAt: new Date(retryAt).toISOString(),
        error: summarizeBuildError(error),
        attempt,
      });
    }
  }
}

async function devWatchObservation({
  registryPath,
  registry,
  explicitRoots,
  profileOverride,
}) {
  const roots = await normalizedRoots([...registry.roots, ...explicitRoots]);
  const profile = profileOverride ?? registry.profile;
  assertProfile(profile);
  const treeFingerprint = await rootsFingerprint(roots);
  const codeTreeFingerprint = await rootsCodeFingerprint(roots);
  return {
    roots,
    profile,
    fingerprint: structuredFingerprint({
      registryPath,
      profile,
      roots,
      treeFingerprint,
    }),
    codeFingerprint: structuredFingerprint({
      registryPath,
      profile,
      roots,
      treeFingerprint: codeTreeFingerprint,
    }),
  };
}

export async function runDevSyncOnce({
  roots,
  profile,
  artifactRoot,
  activationUrl = defaultAssemblyActivationUrl,
  activationId,
  buildOnly = false,
  skiffRoot: compilerRoot = skiffRoot,
  fetchImpl = fetch,
  compilerRunner = runCompilerAuthoring,
  configSnapshotRunner = runConfigSnapshotAuthoring,
  activationWait = delay,
  buildState,
}) {
  assertProfile(profile);
  const classified = await normalizedRoots(roots);
  await mkdir(artifactRoot, { recursive: true });
  const build = buildState ?? await buildDevAssembly({
    classified,
    profile,
    artifactRoot,
    compilerRoot,
    compilerRunner,
  });
  validateReusableBuildState(build, { profile, artifactRoot });
  const {
    serviceContractReceipts,
    packageArtifactReceipts,
    serviceDeploymentReceipts,
    serviceConfigSources,
    assemblyReceipt,
  } = build;
  if (!isPlainObject(assemblyReceipt?.assembly)) {
    throw new Error('assembly build did not return an exact RuntimeAssembly reference');
  }
  if (typeof assemblyReceipt.recordPath !== 'string' || assemblyReceipt.recordPath.length === 0) {
    throw new Error('assembly build did not return a RuntimeAssembly record path');
  }
  const snapshotResult = await configSnapshotRunner({
    skiffRoot: compilerRoot,
    artifactRoot,
    profile,
    assemblyRecord: assemblyReceipt.recordPath,
    sources: serviceConfigSources,
  });
  const configSnapshotReceipt = snapshotResult?.runtimeConfigSnapshotReceipt;
  if (!isPlainObject(configSnapshotReceipt?.snapshot)) {
    throw new Error('config snapshot production did not return an exact snapshot reference');
  }

  const result = {
    serviceContractReceipts,
    packageArtifactReceipts,
    serviceDeploymentReceipts,
    runtimeAssemblyReceipt: assemblyReceipt,
    runtimeConfigSnapshotReceipt: configSnapshotReceipt,
  };
  devBuildStates.set(result, build);
  if (buildOnly) {
    return result;
  }
  const activation = await activateDevAssembly({
    fetchImpl,
    activationUrl,
    activationId,
    profile,
    assembly: assemblyReceipt.assembly,
    configSnapshot: configSnapshotReceipt.snapshot,
    wait: activationWait,
  });
  const activated = {
    ...result,
    assemblyActivationReceipt: activation,
  };
  devBuildStates.set(activated, build);
  return activated;
}

export function reusableDevBuildState(result) {
  const state = devBuildStates.get(result);
  if (state === undefined) {
    throw new Error('dev sync result does not own reusable build state');
  }
  return state;
}

async function buildDevAssembly({
  classified,
  profile,
  artifactRoot,
  compilerRoot,
  compilerRunner,
}) {
  const serviceContractReceipts = [];
  const packageArtifactReceipts = [];
  const serviceDeploymentReceipts = [];
  const serviceConfigSources = [];
  const coordinateOwners = new Map();
  const buildPackage = async (entry) => compilerRunner({
    skiffRoot: compilerRoot,
    kind: 'package',
    action: 'publish',
    root: entry.root,
    artifactRoot,
    profile,
  });
  for (const { entry, receipt } of await buildDependencyOrdered(classified, buildPackage)) {
    rejectDuplicateCoordinate(
      coordinateOwners,
      `package:${coordinate(receipt.packageArtifactReceipt?.artifact, ['packageId', 'packageVersion'])}`,
      entry.root,
    );
    packageArtifactReceipts.push(receipt.packageArtifactReceipt);
    if (receipt.serviceContractReceipt !== undefined) {
      rejectDuplicateCoordinate(
        coordinateOwners,
        `contract:${coordinate(receipt.serviceContractReceipt?.contract, ['serviceId', 'contractVersion'])}`,
        entry.root,
      );
      serviceContractReceipts.push(receipt.serviceContractReceipt);
    }
    if (receipt.serviceDeploymentReceipt !== undefined) {
      serviceDeploymentReceipts.push(receipt.serviceDeploymentReceipt);
      serviceConfigSources.push({
        root: entry.root,
        deployment: receipt.serviceDeploymentReceipt?.deployment,
      });
    }
  }
  const rootDeployments = serviceDeploymentReceipts.map((receipt) => receipt?.deployment);
  if (rootDeployments.some((reference) => !isPlainObject(reference))) {
    throw new Error('deployment publish did not return an exact ServiceDeployment reference');
  }
  const assemblyResult = await compilerRunner({
    skiffRoot: compilerRoot,
    kind: 'assembly',
    action: 'build',
    rootDeployments,
    artifactRoot,
    profile,
  });
  return {
    profile,
    artifactRoot,
    serviceContractReceipts,
    packageArtifactReceipts,
    serviceDeploymentReceipts,
    serviceConfigSources,
    assemblyReceipt: assemblyResult.runtimeAssemblyReceipt,
  };
}

function validateReusableBuildState(state, { profile, artifactRoot }) {
  if (
    !isPlainObject(state)
    || state.profile !== profile
    || state.artifactRoot !== artifactRoot
    || !Array.isArray(state.serviceContractReceipts)
    || !Array.isArray(state.packageArtifactReceipts)
    || !Array.isArray(state.serviceDeploymentReceipts)
    || !Array.isArray(state.serviceConfigSources)
    || !isPlainObject(state.assemblyReceipt)
  ) {
    throw new Error('dev sync reusable build state does not match this profile and artifact root');
  }
}

async function buildDependencyOrdered(entries, build) {
  const completed = [];
  let pending = [...entries];
  while (pending.length > 0) {
    const deferred = [];
    let progressed = false;
    for (const entry of pending) {
      try {
        completed.push({ entry, receipt: await build(entry) });
        progressed = true;
      } catch (error) {
        if (isUnpublishedExactDependency(error)) {
          deferred.push({ entry, error });
          continue;
        }
        throw error;
      }
    }
    if (!progressed) {
      const details = deferred
        .map(({ entry, error }) => `${entry.root}: ${formatError(error)}`)
        .join('\n');
      throw new Error(`dev sync could not close exact package/service dependencies:\n${details}`);
    }
    pending = deferred.map(({ entry }) => entry);
  }
  return completed;
}

function isUnpublishedExactDependency(error) {
  return /has no published (?:PackageArtifact|ServiceContract) pointer/.test(formatError(error));
}

export async function readDevRegistry(path = defaultRegistryPath, options = {}) {
  return readStoredDevRegistry(path, options);
}

export async function writeDevRegistry(path, registry) {
  return writeStoredDevRegistry(path, {
    schemaVersion: devRegistrySchemaVersion,
    profile: registry.profile,
    roots: registry.roots,
  });
}

export async function classifyAuthoringRoot(root) {
  const absolute = resolve(root);
  const rootEntries = await readdir(absolute, { withFileTypes: true });
  const present = [];
  for (const file of [
    'package.yml',
    'service.yml',
    'http.yml',
    'websocket.yml',
    'contract.yml',
    'deployment.yml',
  ]) {
    try {
      const metadata = await stat(join(absolute, file));
      if (metadata.isFile()) {
        present.push(file);
      }
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
  }
  const externalServiceControlFiles = present.filter(
    (file) => file === 'http.yml' || file === 'websocket.yml',
  );
  if (externalServiceControlFiles.length > 0 && !present.includes('service.yml')) {
    throw new Error(
      `${absolute} contains external service control file(s) ${externalServiceControlFiles.join(', ')}; external service control files require service.yml to declare the service role`,
    );
  }
  if (!present.includes('package.yml')) {
    throw new Error(`${absolute} must contain package.yml`);
  }
  const configFiles = rootEntries
    .filter((entry) => entry.isFile() && isServiceConfigFileName(entry.name))
    .map((entry) => entry.name)
    .sort();
  if (configFiles.length > 0 && !present.includes('service.yml')) {
    throw new Error(
      `${absolute} contains service config file(s) ${configFiles.join(', ')}; profile config belongs only to a Package with service.yml`,
    );
  }
  const legacy = present.filter(
    (file) => file === 'contract.yml' || file === 'deployment.yml',
  );
  if (legacy.length > 0) {
    throw new Error(`${absolute} contains retired independent authoring file(s): ${legacy.join(', ')}`);
  }
  if (!present.includes('service.yml')) {
    return { kind: 'package', root: absolute };
  }
  const servicePath = join(absolute, 'service.yml');
  const service = parseServiceManifestIdentity(
    await readFile(servicePath, 'utf8'),
    servicePath,
  );
  return { kind: 'service', root: absolute, serviceId: service.id };
}

export function parseDevSyncArgs(rawArgs) {
  const result = {
    roots: [],
    config: defaultRegistryPath,
    artifactRoot: defaultArtifactRoot,
    activationUrl: defaultAssemblyActivationUrl,
    activationId: undefined,
    profile: undefined,
    pollIntervalMs: 500,
    watch: false,
    buildOnly: false,
    json: false,
  };
  const flags = new Map([
    ['--watch', 'watch'],
    ['--build-only', 'buildOnly'],
    ['--json', 'json'],
  ]);
  const valued = new Map([
    ['--root', 'roots'],
    ['--config', 'config'],
    ['--artifact-root', 'artifactRoot'],
    ['--activation-url', 'activationUrl'],
    ['--activation-id', 'activationId'],
    ['--profile', 'profile'],
    ['--poll-interval-ms', 'pollIntervalMs'],
  ]);
  const seen = new Set();
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    if (argument === '-h' || argument === '--help') {
      console.log(usage);
      return { ...result, help: true };
    }
    if (flags.has(argument)) {
      if (seen.has(argument)) {
        throw new Error(`${argument} was provided more than once`);
      }
      seen.add(argument);
      result[flags.get(argument)] = true;
      continue;
    }
    const equals = argument.indexOf('=');
    const option = equals === -1 ? argument : argument.slice(0, equals);
    if (!valued.has(option)) {
      throw new Error(`unknown option ${argument}\n${usage}`);
    }
    if (option !== '--root' && seen.has(option)) {
      throw new Error(`${option} was provided more than once`);
    }
    seen.add(option);
    const value = equals === -1 ? rawArgs[index + 1] : argument.slice(equals + 1);
    if (!value || value.startsWith('--')) {
      throw new Error(`${option} requires a value`);
    }
    if (equals === -1) {
      index += 1;
    }
    const field = valued.get(option);
    if (field === 'roots') {
      result.roots.push({ root: resolve(value) });
    } else {
      result[field] = value;
    }
  }
  result.config = resolve(result.config);
  result.artifactRoot = resolve(result.artifactRoot);
  result.pollIntervalMs = parsePositiveInteger(result.pollIntervalMs, '--poll-interval-ms');
  return result;
}

async function normalizedRoots(values, label = 'dev registry') {
  const roots = [];
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (!isPlainObject(value)) {
      throw new Error(`${label} root ${index} must be an object`);
    }
    const fields = Object.keys(value).sort();
    if (
      fields.some((field) => !['kind', 'root', 'serviceId'].includes(field))
      || !fields.includes('root')
    ) {
      throw new Error(`${label} root ${index} fields must be root and optional kind/serviceId`);
    }
    const detected = await classifyAuthoringRoot(value.root);
    if (value.kind !== undefined && value.kind !== detected.kind) {
      throw new Error(`${value.root} is ${detected.kind}, not declared kind ${value.kind}`);
    }
    if (value.serviceId !== undefined && value.serviceId !== detected.serviceId) {
      throw new Error(
        `${value.root} declares serviceId ${detected.serviceId}, not stored ${value.serviceId}`,
      );
    }
    roots.push(detected);
  }
  roots.sort((left, right) => left.kind.localeCompare(right.kind) || left.root.localeCompare(right.root));
  for (let index = 1; index < roots.length; index += 1) {
    if (roots[index - 1].root === roots[index].root) {
      throw new Error(`dev registry contains duplicate root ${roots[index].root}`);
    }
  }
  const serviceIds = new Set();
  for (const entry of roots) {
    if (entry.serviceId !== undefined && serviceIds.has(entry.serviceId)) {
      throw new Error(`dev registry contains duplicate serviceId ${entry.serviceId}`);
    }
    if (entry.serviceId !== undefined) {
      serviceIds.add(entry.serviceId);
    }
  }
  return roots;
}

async function rootsFingerprint(roots) {
  const hash = createHash('sha256');
  for (const { kind, root, serviceId } of roots) {
    hash.update(`${kind}\0${root}\0${serviceId ?? ''}\0`);
    await hashTree(root, root, hash);
  }
  return hash.digest('hex');
}

async function rootsCodeFingerprint(roots) {
  const hash = createHash('sha256');
  for (const { kind, root, serviceId } of roots) {
    hash.update(`${kind}\0${root}\0${serviceId ?? ''}\0`);
    await hashTree(root, root, hash, { skipRootConfig: true });
  }
  return hash.digest('hex');
}

export async function watchAuthoringRootChanges({
  roots,
  initialFingerprint,
  initialCodeFingerprint,
  pollIntervalMs,
  onChange,
  wait = delay,
}) {
  let fingerprint = initialFingerprint ?? await rootsFingerprint(roots);
  let codeFingerprint = initialCodeFingerprint ?? await rootsCodeFingerprint(roots);
  for (;;) {
    await wait(pollIntervalMs);
    const next = await rootsFingerprint(roots);
    if (next === fingerprint) {
      continue;
    }
    const nextCodeFingerprint = await rootsCodeFingerprint(roots);
    const kind = nextCodeFingerprint === codeFingerprint ? 'config' : 'code';
    await onChange({ kind, codeFingerprint: nextCodeFingerprint });
    fingerprint = next;
    codeFingerprint = nextCodeFingerprint;
  }
}

async function hashTree(root, current, hash, { skipRootConfig = false } = {}) {
  const entries = await readdir(current, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) {
      continue;
    }
    const path = join(current, entry.name);
    if (entry.isDirectory()) {
      await hashTree(root, path, hash, { skipRootConfig });
    } else if (entry.isFile()) {
      if (
        skipRootConfig
        && current === root
        && isServiceConfigFileName(entry.name)
      ) {
        continue;
      }
      hash.update(path.slice(root.length));
      hash.update(await readFile(path));
    }
  }
}

function isServiceConfigFileName(name) {
  return name === 'config.yml'
    || (name.startsWith('config.') && name.endsWith('.yml'));
}

function coordinate(reference, fields) {
  if (!isPlainObject(reference) || fields.some((field) => typeof reference[field] !== 'string')) {
    throw new Error(`authoring publish did not return coordinates ${fields.join(', ')}`);
  }
  return fields.map((field) => reference[field]).join('@');
}

function rejectDuplicateCoordinate(owners, key, root) {
  const existing = owners.get(key);
  if (existing !== undefined && existing !== root) {
    throw new Error(`dev registry roots ${existing} and ${root} publish duplicate coordinate ${key}`);
  }
  owners.set(key, root);
}

function parseNonNegativeInteger(value, label) {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value !== 'string' || !/^(?:0|[1-9][0-9]*)$/.test(value)) {
    throw new Error(`${label} must be a non-negative integer`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${label} exceeds the safe integer range`);
  }
  return parsed;
}

function parsePositiveInteger(value, label) {
  const parsed = typeof value === 'number' ? value : parseNonNegativeInteger(value, label);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error(`${label} must be a positive integer`);
  }
  return parsed;
}

function printResult(result, json) {
  if (json) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  console.log(JSON.stringify(result));
}

function structuredFingerprint(value) {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}
