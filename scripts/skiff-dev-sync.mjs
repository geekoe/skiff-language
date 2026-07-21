#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  defaultAssemblyActivationUrl,
  maxExpectedAssemblyGeneration,
  requestAssemblyActivation,
  runCompilerAuthoring,
} from './lib/package-service-authoring.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const defaultDevHome = resolve(process.env.SKIFF_DEV_HOME ?? join(skiffRoot, '.skiff-instance', 'dev-home'));
const defaultRegistryPath = join(defaultDevHome, 'watch.json');
const defaultArtifactRoot = join(defaultDevHome, 'artifacts');
const registrySchemaVersion = 'skiff-package-service-dev-registry-v1';
const ignoredDirectories = new Set(['.git', 'build', 'node_modules', 'target']);

const usage = `usage: node skiff-dev-sync.mjs [--watch] [--root <package|contract|deployment-root>]... [--config <path>] [--artifact-root <dir>] [--environment <name>] [--activation-url <url>] [--expected-generation <n>] [--activation-id <id>] [--poll-interval-ms <ms>] [--build-only] [--json]`;

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
  });

  if (!options.watch) {
    const result = await run();
    printResult(result, options.json);
    return result;
  }

  let expectedGeneration = options.expectedGeneration;
  let fingerprint = await rootsFingerprint(roots);
  let initial = await runDevSyncOnce({
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
  });
  expectedGeneration = nextExpectedGeneration(initial, expectedGeneration);
  printResult(initial, options.json);

  for (;;) {
    await delay(options.pollIntervalMs);
    const next = await rootsFingerprint(roots);
    if (next === fingerprint) {
      continue;
    }
    fingerprint = next;
    try {
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
      });
      expectedGeneration = nextExpectedGeneration(result, expectedGeneration);
      printResult(result, options.json);
    } catch (error) {
      console.error(`dev sync rejected: ${formatError(error)}`);
    }
  }
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
  if (!classified.some(({ kind }) => kind === 'deployment')) {
    throw new Error('dev sync requires at least one deployment root to form RuntimeAssembly roots');
  }
  await mkdir(artifactRoot, { recursive: true });

  const serviceContractReceipts = [];
  const packageArtifactReceipts = [];
  const serviceDeploymentReceipts = [];
  const coordinateOwners = new Map();

  for (const kind of ['contract', 'package', 'deployment']) {
    for (const entry of classified.filter((root) => root.kind === kind)) {
      const receipt = await compilerRunner({
        skiffRoot: compilerRoot,
        kind,
        action: 'publish',
        root: entry.root,
        artifactRoot,
      });
      if (kind === 'contract') {
        rejectDuplicateCoordinate(
          coordinateOwners,
          `contract:${coordinate(receipt.serviceContractReceipt?.contract, ['serviceId', 'contractVersion'])}`,
          entry.root,
        );
        serviceContractReceipts.push(receipt.serviceContractReceipt);
      } else if (kind === 'package') {
        rejectDuplicateCoordinate(
          coordinateOwners,
          `package:${coordinate(receipt.packageArtifactReceipt?.artifact, ['packageId', 'packageVersion'])}`,
          entry.root,
        );
        packageArtifactReceipts.push(receipt.packageArtifactReceipt);
      } else {
        serviceDeploymentReceipts.push(receipt.serviceDeploymentReceipt);
      }
    }
  }

  const rootDeployments = serviceDeploymentReceipts.map((receipt) => receipt?.deployment);
  if (rootDeployments.some((reference) => !isPlainObject(reference))) {
    throw new Error('deployment publish did not return an exact ServiceDeployment reference');
  }
  const temporary = await mkdtemp(join(tmpdir(), 'skiff-runtime-assembly-'));
  let assemblyReceipt;
  try {
    await writeFile(join(temporary, 'assembly.yml'), `${JSON.stringify({
      environment,
      rootDeployments,
    }, null, 2)}\n`);
    const result = await compilerRunner({
      skiffRoot: compilerRoot,
      kind: 'assembly',
      action: 'build',
      root: temporary,
      artifactRoot,
    });
    assemblyReceipt = result.runtimeAssemblyReceipt;
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  if (!isPlainObject(assemblyReceipt?.assembly)) {
    throw new Error('assembly build did not return an exact RuntimeAssembly reference');
  }

  const result = {
    serviceContractReceipts,
    packageArtifactReceipts,
    serviceDeploymentReceipts,
    runtimeAssemblyReceipt: assemblyReceipt,
  };
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
  });
  return {
    ...result,
    assemblyActivationReceipt: activation,
  };
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
  const candidates = [
    ['package', 'package.yml'],
    ['contract', 'contract.yml'],
    ['deployment', 'deployment.yml'],
  ];
  const matches = [];
  for (const [kind, file] of candidates) {
    try {
      const metadata = await stat(join(absolute, file));
      if (metadata.isFile()) {
        matches.push(kind);
      }
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
  }
  if (matches.length !== 1) {
    throw new Error(`${absolute} must contain exactly one of package.yml, contract.yml, or deployment.yml`);
  }
  return { kind: matches[0], root: absolute };
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

async function hashTree(root, current, hash) {
  const entries = await readdir(current, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) {
      continue;
    }
    const path = join(current, entry.name);
    if (entry.isDirectory()) {
      await hashTree(root, path, hash);
    } else if (entry.isFile()) {
      hash.update(path.slice(root.length));
      hash.update(await readFile(path));
    }
  }
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
