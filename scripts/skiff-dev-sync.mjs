#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  defaultAssemblyActivationUrl,
  maxExpectedAssemblyGeneration,
  requestAssemblyActivation,
  runConfigSnapshotAuthoring,
  runCompilerAuthoring,
} from './lib/package-service-authoring.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const defaultDevHome = resolve(process.env.SKIFF_DEV_HOME ?? join(skiffRoot, '.skiff-instance', 'dev-home'));
const defaultRegistryPath = join(defaultDevHome, 'watch.json');
const defaultArtifactRoot = join(defaultDevHome, 'artifacts');
const registrySchemaVersion = 'skiff-package-service-dev-registry-v1';
const ignoredDirectories = new Set(['.git', 'build', 'node_modules', 'target']);
const devBuildStates = new WeakMap();

const usage = `usage: node skiff-dev-sync.mjs [--watch] [--root <package-root>]... [--config <path>] [--artifact-root <dir>] [--environment <name>] [--activation-url <url>] [--expected-generation <n>] [--activation-id <id>] [--poll-interval-ms <ms>] [--build-only] [--json]`;

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
  const registry = await readDevRegistry(options.config, { allowMissing: options.roots.length > 0 });
  const roots = await normalizedRoots([...registry.roots, ...options.roots]);
  const environment = options.environment ?? registry.environment;
  const run = () => runDevSyncOnce({
    roots,
    environment,
    artifactRoot: options.artifactRoot,
    activationUrl: options.activationUrl,
    activationId: options.activationId,
    expectedGeneration: options.expectedGeneration,
    buildOnly: options.buildOnly,
    skiffRoot: dependencies.skiffRoot ?? skiffRoot,
    fetchImpl: dependencies.fetchImpl ?? fetch,
    compilerRunner: dependencies.compilerRunner ?? runCompilerAuthoring,
    configSnapshotRunner: dependencies.configSnapshotRunner ?? runConfigSnapshotAuthoring,
  });

  if (!options.watch) {
    const result = await run();
    printResult(result, options.json);
    return result;
  }

  let expectedGeneration = options.expectedGeneration;
  const fingerprint = await rootsFingerprint(roots);
  let successfulCodeFingerprint = await rootsCodeFingerprint(roots);
  const initial = await runDevSyncOnce({
    roots,
    environment,
    artifactRoot: options.artifactRoot,
    activationUrl: options.activationUrl,
    activationId: options.activationId,
    expectedGeneration,
    buildOnly: options.buildOnly,
    skiffRoot: dependencies.skiffRoot ?? skiffRoot,
    fetchImpl: dependencies.fetchImpl ?? fetch,
    compilerRunner: dependencies.compilerRunner ?? runCompilerAuthoring,
    configSnapshotRunner: dependencies.configSnapshotRunner ?? runConfigSnapshotAuthoring,
  });
  let buildState = reusableDevBuildState(initial);
  expectedGeneration = nextExpectedGeneration(initial, expectedGeneration);
  printResult(initial, options.json);

  await watchAuthoringRootChanges({
    roots,
    initialFingerprint: fingerprint,
    initialCodeFingerprint: successfulCodeFingerprint,
    pollIntervalMs: options.pollIntervalMs,
    onChange: async ({ codeFingerprint: currentCodeFingerprint }) => {
      try {
        const configOnly = currentCodeFingerprint === successfulCodeFingerprint;
        const result = await runDevSyncOnce({
          roots,
          environment,
          artifactRoot: options.artifactRoot,
          activationUrl: options.activationUrl,
          activationId: undefined,
          expectedGeneration,
          buildOnly: options.buildOnly,
          skiffRoot: dependencies.skiffRoot ?? skiffRoot,
          fetchImpl: dependencies.fetchImpl ?? fetch,
          compilerRunner: dependencies.compilerRunner ?? runCompilerAuthoring,
          configSnapshotRunner: dependencies.configSnapshotRunner ?? runConfigSnapshotAuthoring,
          buildState: configOnly ? buildState : undefined,
        });
        buildState = reusableDevBuildState(result);
        successfulCodeFingerprint = currentCodeFingerprint;
        expectedGeneration = nextExpectedGeneration(result, expectedGeneration);
        printResult(result, options.json);
      } catch (error) {
        console.error(`dev sync rejected: ${formatError(error)}`);
      }
    },
  });
}

export async function runDevSyncOnce({
  roots,
  environment,
  artifactRoot,
  activationUrl = defaultAssemblyActivationUrl,
  activationId,
  expectedGeneration,
  buildOnly = false,
  skiffRoot: compilerRoot = skiffRoot,
  fetchImpl = fetch,
  compilerRunner = runCompilerAuthoring,
  configSnapshotRunner = runConfigSnapshotAuthoring,
  buildState,
}) {
  assertEnvironment(environment);
  if (
    !buildOnly
    && (
      !Number.isSafeInteger(expectedGeneration)
      || Object.is(expectedGeneration, -0)
      || expectedGeneration < 0
      || expectedGeneration > maxExpectedAssemblyGeneration
    )
  ) {
    throw new Error(
      `dev sync activation expected generation must be between 0 and ${maxExpectedAssemblyGeneration}`,
    );
  }
  const classified = await normalizedRoots(roots);
  await mkdir(artifactRoot, { recursive: true });
  const build = buildState ?? await buildDevAssembly({
    classified,
    environment,
    artifactRoot,
    compilerRoot,
    compilerRunner,
  });
  validateReusableBuildState(build, { environment, artifactRoot });
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
    profile: environment,
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
  const activation = await requestAssemblyActivation({
    fetchImpl,
    activationUrl,
    activationId,
    expectedGeneration,
    environment,
    assembly: assemblyReceipt.assembly,
    configSnapshot: configSnapshotReceipt.snapshot,
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
  environment,
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
    environment,
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
  if (serviceDeploymentReceipts.length === 0) {
    throw new Error('dev sync requires at least one service package root to form RuntimeAssembly roots');
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
    environment,
  });
  return {
    environment,
    artifactRoot,
    serviceContractReceipts,
    packageArtifactReceipts,
    serviceDeploymentReceipts,
    serviceConfigSources,
    assemblyReceipt: assemblyResult.runtimeAssemblyReceipt,
  };
}

function validateReusableBuildState(state, { environment, artifactRoot }) {
  if (
    !isPlainObject(state)
    || state.environment !== environment
    || state.artifactRoot !== artifactRoot
    || !Array.isArray(state.serviceContractReceipts)
    || !Array.isArray(state.packageArtifactReceipts)
    || !Array.isArray(state.serviceDeploymentReceipts)
    || !Array.isArray(state.serviceConfigSources)
    || !isPlainObject(state.assemblyReceipt)
  ) {
    throw new Error('dev sync reusable build state does not match this environment and artifact root');
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

export async function readDevRegistry(path = defaultRegistryPath, { allowMissing = false } = {}) {
  const registryPath = resolve(path);
  let value;
  try {
    value = JSON.parse(await readFile(registryPath, 'utf8'));
  } catch (error) {
    if (allowMissing && error?.code === 'ENOENT') {
      return { schemaVersion: registrySchemaVersion, environment: 'dev', roots: [] };
    }
    throw new Error(`failed to read dev registry ${registryPath}: ${formatError(error)}`);
  }
  if (!isPlainObject(value)) {
    throw new Error(`${registryPath} must contain a JSON object`);
  }
  const fields = Object.keys(value).sort();
  const expected = ['environment', 'roots', 'schemaVersion'];
  if (JSON.stringify(fields) !== JSON.stringify(expected)) {
    throw new Error(`${registryPath} fields must be exactly ${expected.join(', ')}`);
  }
  if (value.schemaVersion !== registrySchemaVersion) {
    throw new Error(`${registryPath} schemaVersion must be ${registrySchemaVersion}`);
  }
  assertEnvironment(value.environment);
  if (!Array.isArray(value.roots)) {
    throw new Error(`${registryPath} roots must be an array`);
  }
  return {
    schemaVersion: registrySchemaVersion,
    environment: value.environment,
    roots: await normalizedRoots(value.roots, registryPath),
  };
}

export async function writeDevRegistry(path, registry) {
  const registryPath = resolve(path);
  const roots = await normalizedRoots(registry.roots, registryPath);
  assertEnvironment(registry.environment);
  await mkdir(dirname(registryPath), { recursive: true });
  await writeFile(registryPath, `${JSON.stringify({
    schemaVersion: registrySchemaVersion,
    environment: registry.environment,
    roots,
  }, null, 2)}\n`);
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
      `${absolute} contains service config file(s) ${configFiles.join(', ')}; environment config belongs only to a Package with service.yml`,
    );
  }
  const legacy = present.filter(
    (file) => file === 'contract.yml' || file === 'deployment.yml',
  );
  if (legacy.length > 0) {
    throw new Error(`${absolute} contains retired independent authoring file(s): ${legacy.join(', ')}`);
  }
  return { kind: 'package', root: absolute };
}

export function parseDevSyncArgs(rawArgs) {
  const result = {
    roots: [],
    config: defaultRegistryPath,
    artifactRoot: defaultArtifactRoot,
    activationUrl: defaultAssemblyActivationUrl,
    expectedGeneration: undefined,
    activationId: undefined,
    environment: undefined,
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
    ['--environment', 'environment'],
    ['--expected-generation', 'expectedGeneration'],
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
  result.expectedGeneration = parseNonNegativeInteger(result.expectedGeneration, '--expected-generation');
  if (result.expectedGeneration > maxExpectedAssemblyGeneration) {
    throw new Error(`--expected-generation must not exceed ${maxExpectedAssemblyGeneration}`);
  }
  result.pollIntervalMs = parsePositiveInteger(result.pollIntervalMs, '--poll-interval-ms');
  if (!result.buildOnly && result.expectedGeneration === undefined) {
    throw new Error('--expected-generation is required unless --build-only is used');
  }
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
    if (fields.some((field) => field !== 'kind' && field !== 'root') || !fields.includes('root')) {
      throw new Error(`${label} root ${index} fields must be root and optional kind`);
    }
    const detected = await classifyAuthoringRoot(value.root);
    if (value.kind !== undefined && value.kind !== detected.kind) {
      throw new Error(`${value.root} is ${detected.kind}, not declared kind ${value.kind}`);
    }
    roots.push(detected);
  }
  roots.sort((left, right) => left.kind.localeCompare(right.kind) || left.root.localeCompare(right.root));
  for (let index = 1; index < roots.length; index += 1) {
    if (roots[index - 1].root === roots[index].root) {
      throw new Error(`dev registry contains duplicate root ${roots[index].root}`);
    }
  }
  return roots;
}

async function rootsFingerprint(roots) {
  const hash = createHash('sha256');
  for (const { kind, root } of roots) {
    hash.update(`${kind}\0${root}\0`);
    await hashTree(root, root, hash);
  }
  return hash.digest('hex');
}

async function rootsCodeFingerprint(roots) {
  const hash = createHash('sha256');
  for (const { kind, root } of roots) {
    hash.update(`${kind}\0${root}\0`);
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
    fingerprint = next;
    const nextCodeFingerprint = await rootsCodeFingerprint(roots);
    const kind = nextCodeFingerprint === codeFingerprint ? 'config' : 'code';
    codeFingerprint = nextCodeFingerprint;
    await onChange({ kind, codeFingerprint });
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

function nextExpectedGeneration(result, fallback) {
  const response = result?.assemblyActivationReceipt?.response;
  const candidates = [response?.committed?.generation, response?.generation, response?.candidateGeneration];
  return candidates.find((value) => Number.isSafeInteger(value) && value >= 0) ?? fallback;
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

function assertEnvironment(value) {
  if (typeof value !== 'string' || !/^(?!\.{1,2}$)[A-Za-z0-9._-]{1,200}$/.test(value)) {
    throw new Error('environment must use only letters, digits, dot, dash, or underscore');
  }
}

function printResult(result, json) {
  if (json) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  console.log(JSON.stringify(result));
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}
