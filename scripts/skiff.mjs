#!/usr/bin/env node
import { constants as fsConstants } from 'node:fs';
import { access, chmod, lstat, mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { runAttachedCommand } from './lib/command-execution.mjs';
import { runAuthoringObjectCommand } from './lib/package-service-authoring.mjs';
import { runDevRegistryCommand } from './lib/package-service-dev-registry.mjs';
import { devRuntimePaths } from './lib/dev-runtime-paths.mjs';
import {
  runInIsolatedTestRuntime,
  shouldUseIsolatedTestRuntime,
} from './lib/isolated-test-runtime.mjs';
import { runOwnedCommand } from './lib/owned-command.mjs';
import {
  defaultProjectPackageDir,
  readProjectPackageDirs,
} from './lib/project-config.mjs';
import { renderRouterConfig, renderRuntimeConfig, renderTelemetryConfig } from './lib/runtime-stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const homeDir = process.env.HOME || process.env.USERPROFILE || '.';
const defaultDevHome = join(skiffRoot, '.skiff-instance', 'dev-home');
const defaultResolvedDevHome = resolveDevHome(process.env.SKIFF_DEV_HOME);
const defaultWatchRegistryPath = join(defaultResolvedDevHome, 'watch.json');
const defaultBinDir = join(homeDir, 'bin');
const defaultDevControlUrl = 'http://127.0.0.1:4001';
const defaultLocalMongoUrl = 'mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false';

const usage = `usage:
  skiff test <package-root-or-file> --artifact-root <dir> [--base-assembly <identity>] [--test-config-literals <json-file>] [--live --activation-url <url> --ingress-url <url> --environment <id> --expected-generation <n>] [--deny-skips] [--require-tests]
  skiff project init [root] [--force]
  skiff project paths [root] [--json]
  skiff dev init [--dev-home <dir>] [--bin-dir <dir>] [--service-db-mongo-url <url>] [--telemetry-db <db>] [--telemetry-mongo-url <url>] [--force] [--no-bin]
  skiff dev paths [--dev-home <dir>] [--json]
  skiff dev status [--config <path>] [--control-url <url>]
  skiff dev sync [--root <package|contract|deployment-root>]... [--config <path>] [--artifact-root <dir>] [--environment <name>] --expected-generation <n> [--activation-url <url>] [--activation-id <id>] [--build-only] [--json]
  skiff dev watch [--root <package|contract|deployment-root>]... [--config <path>] [--artifact-root <dir>] [--environment <name>] --expected-generation <n> [--activation-url <url>] [--poll-interval-ms <ms>] [--build-only] [--json]
  skiff dev registry list [--config <path>]
  skiff dev registry add <root> [--environment <name>] [--config <path>]
  skiff dev registry remove <root> [--config <path>]
  skiff instance init <config> [--force]
  skiff instance paths <config> [--json]
  skiff instance status <config> [--json]
  skiff instance doctor <config>
  skiff instance repair <config>
  skiff instance build <config>
  skiff instance refresh-binaries <config>
  skiff instance up <config> [--repair-owned-conflicts]
  skiff instance restart <config> [component]
  skiff instance supervise <config>
  skiff instance run <config>  # deprecated alias for supervise
  skiff instance down <config>
  skiff instance sync <config> [root] --expected-generation <n> [--environment <name>] [--activation-id <id>] [--build-only] [--json]
  skiff instance watch <config> [root] --expected-generation <n> [--environment <name>] [--poll-interval-ms <ms>] [--build-only] [--json]
  skiff package build <root> --artifact-root <dir> [--json]
  skiff package publish <root> --artifact-root <dir> [--json]
  skiff contract <build|publish> <root> --artifact-root <dir> [--json]
  skiff deployment <build|publish> <root> --artifact-root <dir> [--json]
  skiff assembly <build|publish> <root> --artifact-root <dir> [--json]
  skiff assembly activate <root> --artifact-root <dir> --expected-generation <n> [--activation-url <url>] [--activation-id <id>] [--json]
`;

try {
  await main(process.argv.slice(2));
} catch (error) {
  console.error(`error: ${formatError(error)}`);
  process.exitCode = 1;
}

async function main(args) {
  const command = args.shift();
  if (!command || command === '-h' || command === '--help') {
    console.log(usage);
    return;
  }

  switch (command) {
    case 'test':
      await test(args);
      return;
    case 'project':
      await projectCommand(args);
      return;
    case 'dev':
      await devCommand(args);
      return;
    case 'instance':
      await run('node', [join(scriptDir, 'skiff-instance.mjs'), ...args], process.cwd());
      return;
    case 'package':
      await packageCommand(args);
      return;
    case 'contract':
    case 'deployment':
    case 'assembly':
      await runAuthoringObjectCommand(command, args, { skiffRoot });
      return;
    default:
      throw new Error(`unknown command ${command}\n${usage}`);
  }
}

async function projectCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'init':
      await projectInit(args);
      return;
    case 'paths':
      await projectPaths(args);
      return;
    default:
      throw new Error(`unknown project command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function devCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'init':
      await devInit(args);
      return;
    case 'paths':
      await devPaths(args);
      return;
    case 'status':
      await devStatus(args);
      return;
    case 'sync':
      await run('node', [join(scriptDir, 'skiff-dev-sync.mjs'), ...args], process.cwd());
      return;
    case 'watch':
      await run('node', [join(scriptDir, 'skiff-dev-sync.mjs'), '--watch', ...args], process.cwd());
      return;
    case 'registry':
      await runDevRegistryCommand(args, { defaultConfig: defaultWatchRegistryPath });
      return;
    default:
      throw new Error(`unknown dev command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function projectInit(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--force']),
    optionsWithValues: new Set(),
  });
  if (args.positionals.length > 1) {
    throw new Error('skiff project init accepts at most one root path');
  }
  const root = resolve(args.positionals[0] ?? '.');
  const configPath = join(root, 'skiff.yml');
  const force = args.flags.has('--force');
  await mkdir(root, { recursive: true });
  const write = await writeDevInitFile(configPath, projectConfigTemplateFile().contents, force);
  await mkdir(join(root, defaultProjectPackageDir), { recursive: true });
  console.log(`${write.action}: ${write.path}`);
  console.log(`package store: ${join(root, defaultProjectPackageDir)}`);
}

async function projectPaths(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--json']),
    optionsWithValues: new Set(),
  });
  if (args.positionals.length > 1) {
    throw new Error('skiff project paths accepts at most one root path');
  }
  const startPath = resolve(args.positionals[0] ?? '.');
  const project = await readProjectPackageDirs(startPath);
  const result = {
    projectRoot: project.projectRoot ?? null,
    configPath: project.configPath ?? null,
    configPaths: project.configPaths,
    packageDirs: project.packageDirs,
  };
  if (args.flags.has('--json')) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  for (const [key, value] of Object.entries(result)) {
    console.log(`${key}: ${Array.isArray(value) ? value.join(', ') : value}`);
  }
}

async function test(rawArgs) {
  const args = parseRootCommand(rawArgs, {
    optionsWithValues: new Set([
      '--artifact-root',
      '--base-assembly',
      '--test-config-literals',
      '--activation-url',
      '--ingress-url',
      '--environment',
      '--expected-generation',
    ]),
    flags: new Set(['--live', '--deny-skips', '--require-tests']),
  });
  const live = args.flags.has('--live');
  const liveTargetKeys = [
    'activationUrl', 'ingressUrl', 'environment', 'expectedGeneration',
  ];
  if (!live && liveTargetKeys.some((key) => args.options[key] !== undefined)) {
    throw new Error(
      'non-live skiff test owns activation, ingress, environment, and generation targets',
    );
  }
  if (args.options.artifactRoot === undefined) {
    throw new Error('skiff test requires --artifact-root');
  }
  const explicitArtifactRoot = resolve(args.options.artifactRoot);
  await requireExistingDirectory(explicitArtifactRoot, 'skiff test --artifact-root');
  if (live) {
    for (const key of liveTargetKeys) {
      if (args.options[key] === undefined) {
        throw new Error(`live skiff test requires --${key.replace(/[A-Z]/g, (value) => `-${value.toLowerCase()}`)}`);
      }
    }
    if (!/^(?:0|[1-9][0-9]*)$/.test(args.options.expectedGeneration)) {
      throw new Error('--expected-generation must be a non-negative integer');
    }
    validateCanonicalTestUrl(
      args.options.activationUrl,
      '/__skiff/activate-assembly',
      '--activation-url',
    );
    validateCanonicalTestUrl(args.options.ingressUrl, '/', '--ingress-url');
    if (!/^[A-Za-z0-9._-]{1,200}$/.test(args.options.environment)) {
      throw new Error('--environment must be a canonical ASCII token');
    }
  }
  const kind = await detectRootKind(args.root);
  if (kind.kind !== 'package' && kind.kind !== 'file') {
    throw new Error(kind.message);
  }

  const testArgs = [
    'run',
    '--locked',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-test-runner',
    '--',
    args.root,
  ];
  if (!shouldUseIsolatedTestRuntime(live)) {
    testArgs.push('--live');
  }
  testArgs.push('--artifact-root', explicitArtifactRoot);
  testArgs.push('--platform-source-root', skiffRoot);
  if (args.options.baseAssembly !== undefined) {
    testArgs.push('--base-assembly', args.options.baseAssembly);
  }
  if (args.options.testConfigLiterals !== undefined) {
    testArgs.push('--test-config-literals', resolve(args.options.testConfigLiterals));
  }
  if (live) {
    testArgs.push(
      '--activation-url', args.options.activationUrl,
      '--ingress-url', args.options.ingressUrl,
      '--environment', args.options.environment,
      '--expected-generation', args.options.expectedGeneration,
    );
  }
  if (args.flags.has('--deny-skips')) {
    testArgs.push('--deny-skips');
  }
  if (args.flags.has('--require-tests')) {
    testArgs.push('--require-tests');
  }
  if (live) {
    await run('cargo', testArgs, skiffRoot);
    return;
  }
  await runInIsolatedTestRuntime({
    skiffRoot,
    runTest: (isolatedEnv, signal) => runOwnedCommand('cargo', testArgs, {
      cwd: skiffRoot,
      env: isolatedEnv,
      signal,
    }),
  });
}

function validateCanonicalTestUrl(value, expectedPath, option) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${option} must be an absolute http:// URL`);
  }
  if (
    url.protocol !== 'http:'
    || url.username !== ''
    || url.password !== ''
    || url.search !== ''
    || url.hash !== ''
    || url.pathname !== expectedPath
  ) {
    throw new Error(`${option} must point exactly to ${expectedPath}`);
  }
}

async function devInit(rawArgs) {
  const args = parseDevInitArgs(rawArgs);
  const devHome = resolve(args.options.devHome ?? defaultResolvedDevHome);
  const artifactRoot = join(devHome, 'artifacts');
  const runtimeHome = join(devHome, 'runtime-home');
  const runtimePaths = devRuntimePaths({ devHome });
  const binDir = resolve(args.options.binDir ?? defaultBinDir);
  const serviceDbMongoUrl =
    args.options.serviceDbMongoUrl ??
    process.env.SKIFF_SERVICE_DB_MONGO_URL ??
    process.env.SERVICE_DB_MONGO_URL ??
    defaultLocalMongoUrl;
  const telemetryMongoUrl =
    args.options.telemetryMongoUrl ??
    process.env.SKIFF_TELEMETRY_MONGO_URL ??
    process.env.MONGO_URL ??
    defaultLocalMongoUrl;
  const telemetryDb =
    args.options.telemetryDb ??
    process.env.SKIFF_TELEMETRY_DB ??
    'skiff_telemetry';
  const force = args.flags.has('--force');

  await mkdir(artifactRoot, { recursive: true });
  await mkdir(runtimeHome, { recursive: true });
  await mkdir(runtimePaths.runtimeBinDir, { recursive: true });

  const writes = [];
  writes.push(await writeDevInitFile(join(devHome, 'router.yml'), routerDevConfig({
    artifactRoot,
    ecosystemStoreCliPath: runtimePaths.ecosystemStoreCli,
    identityCliPath: runtimePaths.identityCli,
    serviceDbMongoUrl,
  }), force));
  writes.push(await writeDevInitFile(join(devHome, 'runtime.yml'), runtimeDevConfig({
    artifactRoot,
    runtimeHome,
  }), force));
  writes.push(await writeDevInitFile(join(devHome, 'telemetry.yml'), telemetryDevConfig({
    telemetryDb,
    telemetryMongoUrl,
  }), force));

  if (!args.flags.has('--no-bin')) {
    await mkdir(binDir, { recursive: true });
    const wrapperPath = join(binDir, 'skiff');
    writes.push(await writeDevInitFile(wrapperPath, skiffWrapperScript(), force, { executable: true }));
    if (!pathContains(binDir)) {
      console.warn(`warning: ${binDir} is not on PATH`);
    }
  }

  console.log(`dev home: ${devHome}`);
  for (const write of writes) {
    console.log(`${write.action}: ${write.path}`);
  }
}

async function devPaths(rawArgs) {
  const args = parseDevPathsArgs(rawArgs);
  const devHome = resolve(args.options.devHome ?? defaultResolvedDevHome);
  const paths = devRuntimePaths({ devHome });
  const result = {
    devHome: paths.devHome,
    artifactRoot: paths.artifactRoot,
    serviceBuildRoot: paths.serviceBuildRoot,
    runtimeConfig: paths.runtimeConfig,
    runtimeHome: paths.runtimeHome,
    runtimeBinDir: paths.runtimeBinDir,
    runtimeBinary: paths.runtimeBinary,
    ecosystemStoreCli: paths.ecosystemStoreCli,
    identityCli: paths.identityCli,
    cargoTargetDir: cargoTargetDir(skiffRoot),
  };
  if (args.flags.has('--json')) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  for (const [key, value] of Object.entries(result)) {
    console.log(`${key}: ${value}`);
  }
}

async function devStatus(rawArgs) {
  const args = parseDevConfigArgs(rawArgs);
  const config = await loadDevConfig(args.config);
  const controlUrl = args.controlUrl
    ?? process.env.SKIFF_DEV_CONTROL_URL
    ?? config.controlUrl
    ?? defaultDevControlUrl;
  const statusUrl = controlEndpointUrl(controlUrl, '/__router/health');
  const response = await fetch(statusUrl);
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`router health returned HTTP ${response.status}${body ? `: ${body}` : ''}`);
  }
  printResponseBody(body);
}

async function packageCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'build':
      await runAuthoringObjectCommand('package', [subcommand, ...args], { skiffRoot });
      return;
    case 'publish':
      await runAuthoringObjectCommand('package', [subcommand, ...args], { skiffRoot });
      return;
    default:
      throw new Error(`unknown package command ${subcommand || '(missing)'}\n${usage}`);
  }
}

function projectConfigTemplateFile() {
  return {
    path: 'skiff.yml',
    contents: [
      'packageDirs:',
      `  - ${defaultProjectPackageDir}`,
      '',
    ].join('\n'),
  };
}

function parseRegistryCommandArgs(rawArgs, spec) {
  const options = {};
  const flags = new Set();
  const positionals = [];
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    const equalsIndex = arg.indexOf('=');
    const optionName = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex);
    if (spec.flags.has(optionName)) {
      if (equalsIndex !== -1) {
        throw new Error(`${optionName} does not accept a value`);
      }
      flags.add(optionName);
      continue;
    }
    if (spec.optionsWithValues.has(optionName)) {
      const value = equalsIndex === -1 ? rawArgs[index + 1] : arg.slice(equalsIndex + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${optionName} requires a value`);
      }
      options[toCamelOption(optionName)] = value;
      if (equalsIndex === -1) {
        index += 1;
      }
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`unknown option ${arg}`);
    }
    positionals.push(arg);
  }
  return { flags, options, positionals };
}

function parseRootCommand(rawArgs, spec) {
  const options = {};
  const flags = new Set();
  let root;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (spec.flags.has(arg)) {
      if (flags.has(arg)) {
        throw new Error(`${arg} was provided more than once`);
      }
      flags.add(arg);
      continue;
    }
    const equalsIndex = arg.indexOf('=');
    const optionName = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex);
    if (spec.unsupportedOptionsWithValues?.has(optionName)) {
      if (equalsIndex === -1 && rawArgs[index + 1] && !rawArgs[index + 1].startsWith('--')) {
        index += 1;
      }
      throw new Error(spec.unsupportedOptionsWithValues.get(optionName));
    }
    if (spec.repeatableOptionsWithValues?.has(optionName)) {
      const value = equalsIndex === -1 ? rawArgs[index + 1] : arg.slice(equalsIndex + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${optionName} requires a value`);
      }
      const key = toCamelOption(optionName);
      options[key] ??= [];
      options[key].push(resolve(value));
      if (equalsIndex === -1) {
        index += 1;
      }
      continue;
    }
    if (spec.optionsWithValues.has(optionName)) {
      const value = equalsIndex === -1 ? rawArgs[index + 1] : arg.slice(equalsIndex + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${optionName} requires a value`);
      }
      const key = toCamelOption(optionName);
      if (Object.hasOwn(options, key)) {
        throw new Error(`${optionName} was provided more than once`);
      }
      options[key] = value;
      if (equalsIndex === -1) {
        index += 1;
      }
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`unknown option ${arg}`);
    }
    if (root !== undefined) {
      throw new Error(`unexpected argument ${arg}`);
    }
    root = resolve(arg);
  }
  if (!root) {
    throw new Error('missing root path');
  }
  return { flags, options, root };
}

function resolveDevHome(envValue) {
  if (envValue) {
    const trimmed = envValue.trim();
    if (trimmed.length > 0) {
      return resolve(trimmed);
    }
  }
  return defaultDevHome;
}

function parseDevConfigArgs(rawArgs) {
  const result = {
    config: process.env.SKIFF_DEV_CONFIG ?? process.env.SKIFF_DEV_SYNC_CONFIG,
    controlUrl: undefined,
  };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--config') {
      result.config = resolve(requireNext(rawArgs, index, '--config'));
      index += 1;
    } else if (arg.startsWith('--config=')) {
      result.config = resolve(arg.slice('--config='.length));
    } else if (arg === '--control-url') {
      result.controlUrl = requireNext(rawArgs, index, '--control-url');
      index += 1;
    } else if (arg.startsWith('--control-url=')) {
      result.controlUrl = arg.slice('--control-url='.length);
    } else {
      throw new Error(`unknown option ${arg}`);
    }
  }
  return result;
}

function parseDevInitArgs(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--force', '--no-bin']),
    optionsWithValues: new Set([
      '--bin-dir',
      '--dev-home',
      '--service-db-mongo-url',
      '--telemetry-db',
      '--telemetry-mongo-url',
    ]),
  });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  return args;
}

function parseDevPathsArgs(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--json']),
    optionsWithValues: new Set(['--dev-home']),
  });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  return args;
}

async function detectRootKind(root) {
  let info;
  try {
    info = await lstat(root);
  } catch (error) {
    return { kind: 'missing', message: `failed to inspect root ${root}: ${formatError(error)}` };
  }
  if (info.isFile()) {
    return { kind: 'file' };
  }
  if (!info.isDirectory()) {
    return { kind: 'missing', message: `${root} must be a file or directory` };
  }
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch (error) {
    return { kind: 'missing', message: `failed to inspect root ${root}: ${formatError(error)}` };
  }
  const files = new Set(entries.filter((entry) => entry.isFile()).map((entry) => entry.name));
  const hasPackage = files.has('package.yml');
  const hasService = files.has('service.yml');
  if (hasPackage && hasService) {
    return { kind: 'ambiguous', message: `${root} contains both package.yml and service config` };
  }
  if (hasPackage) {
    return { kind: 'package' };
  }
  if (hasService) {
    return { kind: 'service' };
  }
  return { kind: 'missing', message: `${root} must contain package.yml or service.yml` };
}

async function loadDevConfig(path) {
  if (path === undefined) {
    return {};
  }
  const configPath = resolve(path);
  try {
    const raw = JSON.parse(await readFile(configPath, 'utf8'));
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
      throw new Error(`${configPath} must be a JSON object`);
    }
    return {
      artifactRoot: raw.artifactRoot,
      configPath,
      controlUrl: raw.controlUrl,
    };
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { configPath };
    }
    throw error;
  }
}

async function writeDevInitFile(path, contents, force, options = {}) {
  await mkdir(dirname(path), { recursive: true });
  if (!force && await fileExists(path)) {
    if (options.executable) {
      await chmod(path, 0o755);
    }
    return { action: 'kept', path };
  }
  await writeFile(path, contents, options.executable ? { mode: 0o755 } : undefined);
  if (options.executable) {
    await chmod(path, 0o755);
  }
  return { action: force ? 'wrote' : 'created', path };
}

async function fileExists(path) {
  try {
    await access(path, fsConstants.F_OK);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

async function requireExistingDirectory(path, source) {
  try {
    if ((await stat(path)).isDirectory()) {
      return;
    }
  } catch {
    // Render the same fail-closed diagnostic for missing and unreadable paths.
  }
  throw new Error(`${source} must be an existing directory: ${path}`);
}

function routerDevConfig(options) {
  return renderRouterConfig({
    profile: 'dev',
    host: '0.0.0.0',
    environment: 'dev',
    artifactRoots: [options.artifactRoot],
    ecosystemStoreCliPath: options.ecosystemStoreCliPath,
    identityCliPath: options.identityCliPath,
    devReload: true,
    requestTimeoutMs: 20000,
    httpPort: 4000,
    runtimePort: 4001,
    runtimePath: '/runtime',
    serviceDbMongoUrl: options.serviceDbMongoUrl,
    telemetryEndpoint: 'ws://127.0.0.1:4002/telemetry',
  });
}

function runtimeDevConfig(options) {
  return renderRuntimeConfig({
    routerUrl: 'ws://127.0.0.1:4001/runtime',
    runtimeHome: options.runtimeHome,
    environment: 'dev',
    artifactRoot: options.artifactRoot,
  });
}

function telemetryDevConfig(options) {
  return renderTelemetryConfig({
    host: '127.0.0.1',
    port: 4002,
    path: '/telemetry',
    emitMemory: false,
    mongo: {
      url: options.telemetryMongoUrl,
      database: options.telemetryDb,
    },
  });
}

function skiffWrapperScript() {
  return [
    '#!/usr/bin/env bash',
    `exec node ${shellQuote(join(scriptDir, 'skiff.mjs'))} "$@"`,
    '',
  ].join('\n');
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

function pathContains(directory) {
  const absoluteDirectory = resolve(directory);
  return (process.env.PATH ?? '')
    .split(':')
    .some((entry) => entry.length > 0 && resolve(entry) === absoluteDirectory);
}

function controlEndpointUrl(controlUrl, pathname) {
  const url = new URL(controlUrl);
  url.pathname = pathname;
  url.search = '';
  url.hash = '';
  return url.toString();
}

function printResponseBody(body) {
  const trimmed = body.trim();
  if (!trimmed) {
    return;
  }
  try {
    console.log(JSON.stringify(JSON.parse(trimmed), null, 2));
  } catch {
    console.log(trimmed);
  }
}

function toCamelOption(optionName) {
  return optionName.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
}

function requireNext(args, index, optionName) {
  const value = args[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${optionName} requires a value`);
  }
  return value;
}

function run(command, args, cwd) {
  return runAttachedCommand(command, args, { cwd, env: process.env });
}

function formatError(error) {
  return error?.message || String(error);
}
