#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { access, chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { hostname, tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import {
  devSyncCheckFlags,
  parseDevSyncArgs,
  renderDevSyncArgs,
  serviceDevSyncOptions,
  serviceDevWatchOptions,
} from './lib/dev-sync-args.mjs';
import { devRuntimePaths } from './lib/dev-runtime-paths.mjs';
import { isPublicationId, publicationStorageSegment } from './lib/publication-id.mjs';
import {
  defaultProjectPackageDir,
  readProjectPackageDirs,
  resolvePackageDirsForCommand,
} from './lib/project-config.mjs';
import {
  quoteYamlString,
  renderRouterConfig,
  renderRuntimeConfig,
  renderTelemetryConfig,
} from './lib/runtime-stack-config.mjs';
import { parseYamlStringScalar } from './lib/simple-yaml.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const homeDir = process.env.HOME || process.env.USERPROFILE || '.';
const globalConfigPath = join(homeDir, '.skiff', 'config.json');
const globalCredentialsPath = join(homeDir, '.skiff', 'credentials.json');
const packageCredentialService = 'skiff-package';
const serviceCredentialService = 'skiff-service';
const defaultDevHome = join(skiffRoot, '.skiff-instance', 'dev-home');
const defaultResolvedDevHome = resolveDevHome(process.env.SKIFF_DEV_HOME);
const defaultWatchRegistryPath = join(defaultResolvedDevHome, 'watch.json');
const defaultBinDir = join(homeDir, 'bin');
const defaultDevReloadUrl = 'http://127.0.0.1:4001/__skiff/reload-artifacts';
const defaultLocalMongoUrl = 'mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false';
const accountServiceSelector = { service: 'skiff.run/account', version: '0.1.0' };
const packageRegistryServiceSelector = { service: 'skiff.run/registry', version: '0.1.0' };
const packageRegistryNamePattern = /^[a-z][a-z0-9-]{0,38}[a-z0-9]$/;
const sourceIdentifierPattern = /^[A-Za-z_][A-Za-z0-9_]*$/;

const usage = `usage:
  skiff check <root> [--profile <name>] [--artifact-root <dir>] [--packages-dir <dir>]... [--service-artifact-root <dir>]...
  skiff test <root-or-file> [--profile <name>] [--live] [--allow-network] [--config <path>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--package-test-concurrency <n>]
  skiff project init [root] [--force]
  skiff project paths [root] [--json]
  skiff dev init [--dev-home <dir>] [--bin-dir <dir>] [--service-db-mongo-url <url>] [--telemetry-db <db>] [--telemetry-mongo-url <url>] [--force] [--no-bin]
  skiff dev paths [--dev-home <dir>] [--json]
  skiff dev reload [--config <path>] [--reload-url <url>]
  skiff dev status [--config <path>] [--reload-url <url>]
  skiff instance init <config> [--force]
  skiff instance paths <config> [--json]
  skiff instance status <config> [--json]
  skiff instance doctor <config>
  skiff instance repair <config>
  skiff instance build <config>
  skiff instance up <config> [--repair-owned-conflicts]
  skiff instance restart <config> [component]
  skiff instance supervise <config>
  skiff instance run <config>  # deprecated alias for supervise
  skiff instance down <config>
  skiff instance reload <config>
  skiff instance sync <config> [root] [--profile <name>] [--service-id <id>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--check|--check-sync]
  skiff instance watch <config> [root] [--profile <name>] [--service-id <id>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--poll-interval-ms <ms>]
  skiff service new --template <http-api|http-stream-proxy> <dir>
  skiff service route add --path <path> --handler <root.module.function> [--root <service-dir>]
  skiff service dev sync [root] [--profile <name>] [--artifact-root <dir>] [--service-artifact-root <dir>]... [--reload-url <url>] [--packages-dir <dir>]... [--check|--check-sync]
  skiff service dev watch [root] [--profile <name>] [--artifact-root <dir>] [--service-artifact-root <dir>]... [--reload-url <url>] [--packages-dir <dir>]... [--poll-interval-ms <ms>]
  skiff service dev clean [--root <service-dir>]
  skiff service dev registry list [--config <path>]
  skiff service dev registry add [root] [--profile <name>] [--service-id <id>] [--config <path>]
  skiff service dev registry remove <root-or-service-id> [--config <path>]
  skiff package remote use <url>
  skiff package remote current
  skiff package remote ping
  skiff package remote forget
  skiff package auth authorize [--remote <url>] [--no-open] [--web-url <url>]
  skiff package auth status
  skiff package auth revoke
  skiff package publish <root> [--wait] [--json]
  skiff package resolve <ref> [--json]
  skiff package pull <ref> [--out <dir>] [--revision <revisionId>] [--json]
  skiff package rollback <ref> --to <revisionId> [--json]
  skiff service remote use <url>
  skiff service remote current
  skiff service remote ping
  skiff service remote forget
  skiff service auth authorize [--remote <url>] [--no-open] [--web-url <url>]
  skiff service auth status
  skiff service auth revoke`;

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
    case 'check':
      await check(args);
      return;
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
    case 'service':
      await serviceCommand(args);
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
    case 'reload':
      await devReload(args);
      return;
    case 'status':
      await devStatus(args);
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

async function newService(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(),
    optionsWithValues: new Set(['--template']),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff service new requires exactly one output directory');
  }
  if (!args.options.template) {
    throw new Error('skiff service new requires --template <http-api|http-stream-proxy>');
  }

  const root = resolve(args.positionals[0]);
  const files = serviceTemplateFiles(args.options.template, root);
  await assertEmptyServiceScaffoldTarget(root);
  for (const file of files) {
    const path = join(root, file.path);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, file.contents);
  }

  console.log(`created ${args.options.template} service at ${root}`);
  for (const file of files) {
    console.log(`created: ${join(root, file.path)}`);
  }
}

async function newRoute(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(),
    optionsWithValues: new Set(['--handler', '--path', '--root']),
  });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const routePath = requiredPlainString(args.options.path, 'skiff service route add --path');
  const handler = requiredPlainString(args.options.handler, 'skiff service route add --handler');
  assertHttpRoutePath(routePath);
  const routeHandler = normalizeRouteHandler(handler);

  const root = resolve(args.options.root ?? process.cwd());
  const serviceConfigPath = join(root, 'service.yml');
  if (!await fileExists(serviceConfigPath)) {
    throw new Error(`${root} must contain service.yml`);
  }

  const serviceConfig = await readFile(serviceConfigPath, 'utf8');
  await writeFile(
    serviceConfigPath,
    appendHttpRouteToServiceConfig(serviceConfig, {
      path: routePath,
      handler: routeHandler.configHandler,
    }),
  );

  const sourceWrite = await ensureRouteHandlerStub(root, routeHandler);
  console.log(`updated: ${serviceConfigPath}`);
  console.log(`${sourceWrite.action}: ${sourceWrite.path}`);
}

function serviceBuildDir(serviceId) {
  return join(defaultResolvedDevHome, 'build', publicationStorageSegment(serviceId));
}

async function check(rawArgs) {
  const args = parseRootCommand(rawArgs, {
    optionsWithValues: new Set(['--profile', '--artifact-root', '--service-id']),
    repeatableOptionsWithValues: new Set(['--packages-dir', '--service-artifact-root']),
    flags: new Set(),
  });
  const kind = await detectRootKind(args.root);
  if (kind.kind === 'package') {
    throw new Error('skiff check does not support package roots yet; no pure package compile-check entry exists');
  }
  if (kind.kind !== 'service') {
    throw new Error(kind.message);
  }
  const packageDirs = await resolvePackageDirsForCommand({
    startPath: args.root,
    cliPackageDirs: args.options.packagesDir ?? [],
  });

  await run('node', [
    join(scriptDir, 'skiff-dev-sync.mjs'),
    ...renderDevSyncArgs({
      flags: new Set(['--check']),
      root: args.root,
      options: { ...args.options, packagesDir: packageDirs },
    }),
  ], process.cwd());
}

async function test(rawArgs) {
  const args = parseRootCommand(rawArgs, {
    optionsWithValues: new Set(['--profile', '--config', '--package-test-concurrency']),
    repeatableOptionsWithValues: new Set(['--packages-dir', '--service-artifact-root']),
    flags: new Set(['--live', '--allow-network']),
  });
  const kind = await detectRootKind(args.root);
  if (kind.kind !== 'service' && kind.kind !== 'package' && kind.kind !== 'file') {
    throw new Error(kind.message);
  }

  const testArgs = [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--',
    args.root,
  ];
  if (args.options.profile) {
    testArgs.push('--profile', args.options.profile);
  }
  if (args.flags.has('--live')) {
    testArgs.push('--live');
  }
  if (args.flags.has('--allow-network')) {
    testArgs.push('--allow-network');
  }
  if (args.options.config) {
    testArgs.push('--config', args.options.config);
  }
  if (args.options.packageTestConcurrency) {
    testArgs.push('--package-test-concurrency', args.options.packageTestConcurrency);
  }
  const packageDirs = await resolvePackageDirsForCommand({
    startPath: args.root,
    cliPackageDirs: args.options.packagesDir ?? [],
  });
  for (const packageDir of packageDirs) {
    testArgs.push('--packages-dir', packageDir);
  }
  for (const serviceArtifactRoot of args.options.serviceArtifactRoot ?? []) {
    testArgs.push('--service-artifact-root', serviceArtifactRoot);
  }
  await run('cargo', testArgs, skiffRoot);
}

async function devSync(rawArgs) {
  const args = parseDevSyncArgs(rawArgs, {
    flags: devSyncCheckFlags,
    options: serviceDevSyncOptions,
    resolve,
  });
  await run('node', [
    join(scriptDir, 'skiff-dev-sync.mjs'),
    ...renderDevSyncArgs(args),
  ], process.cwd());
}

async function devWatchRun(rawArgs) {
  const args = parseDevSyncArgs(rawArgs, {
    flags: [],
    options: serviceDevWatchOptions,
    resolve,
  });
  await run('node', [
    join(scriptDir, 'skiff-dev-sync.mjs'),
    ...renderDevSyncArgs(args, { prefix: ['--watch'] }),
  ], process.cwd());
}

async function devWatchList(rawArgs) {
  const args = parseDevWatchRegistryArgs(rawArgs, {
    optionsWithValues: new Set(['--config']),
  });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }

  const registryPath = devWatchRegistryPath(args.options.config);
  const registry = await readDevWatchRegistry(registryPath);
  if (registry.services.length === 0) {
    console.log(`no services registered in ${registryPath}`);
    return;
  }

  console.log(`services registered in ${registryPath}:`);
  for (const service of registry.services) {
    console.log(`- ${service.serviceId} (${service.profile}) ${service.root}`);
  }
}

async function devWatchAdd(rawArgs) {
  const args = parseDevWatchRegistryArgs(rawArgs, {
    optionsWithValues: new Set(['--config', '--profile', '--service-id']),
  });
  if (args.positionals.length > 1) {
    throw new Error('skiff service dev registry add accepts at most one root path');
  }

  const root = resolve(args.positionals[0] ?? '.');
  const serviceId = args.options.serviceId ?? await readServiceId(root);
  if (!validPublicationId(serviceId)) {
    throw new Error(`service id ${serviceId} must be a publication id`);
  }

  const service = {
    root,
    serviceId,
    profile: args.options.profile ?? 'dev',
  };
  const registryPath = devWatchRegistryPath(args.options.config);
  const registry = await readDevWatchRegistry(registryPath);
  registry.services = registry.services.filter((entry) =>
    entry.root !== root && entry.serviceId !== serviceId);
  registry.services.push(service);
  registry.services.sort(compareWatchServices);
  await writeDevWatchRegistry(registryPath, registry);
  console.log(`registered ${service.serviceId} (${service.profile}) at ${service.root}`);
}

async function devWatchRemove(rawArgs) {
  const args = parseDevWatchRegistryArgs(rawArgs, {
    optionsWithValues: new Set(['--config']),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff service dev registry remove requires exactly one root path or service id');
  }

  const target = args.positionals[0];
  const absoluteTarget = target.includes('/') || target === '.' || target === '..'
    ? resolve(target)
    : null;
  const registryPath = devWatchRegistryPath(args.options.config);
  const registry = await readDevWatchRegistry(registryPath);
  const before = registry.services.length;
  registry.services = registry.services.filter((entry) =>
    entry.serviceId !== target && (absoluteTarget === null || entry.root !== absoluteTarget));
  const removed = before - registry.services.length;
  if (removed === 0) {
    console.log(`no registered service matched ${target}`);
    return;
  }
  await writeDevWatchRegistry(registryPath, registry);
  console.log(`removed ${removed} service(s) from ${registryPath}`);
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
    'skiff';
  const force = args.flags.has('--force');

  await mkdir(artifactRoot, { recursive: true });
  await mkdir(runtimeHome, { recursive: true });
  await mkdir(runtimePaths.runtimeBinDir, { recursive: true });

  const writes = [];
  writes.push(await writeDevInitFile(join(devHome, 'router.yml'), routerDevConfig({
    artifactRoot,
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

async function devReload(rawArgs) {
  const args = parseDevConfigArgs(rawArgs);
  const config = await loadDevConfig(args.config);
  const reloadUrl = args.reloadUrl ?? process.env.SKIFF_DEV_RELOAD_URL ?? config.reloadUrl ?? defaultDevReloadUrl;
  const response = await fetch(reloadUrl, { method: 'POST' });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`router reload returned HTTP ${response.status}${body ? `: ${body}` : ''}`);
  }
  console.log(`requested router reload at ${reloadUrl}`);
  if (body.trim()) {
    console.log(body.trim());
  }
  await devPruneAfterReload(reloadUrl);
}

async function devPruneAfterReload(reloadUrl) {
  const pruneUrl = controlUrlFromReloadUrl(reloadUrl, '/__router/prune-runtimes');
  const response = await fetch(pruneUrl, { method: 'POST' });
  const body = await response.text();
  if (!response.ok) {
    console.warn(`warning: router runtime prune returned HTTP ${response.status}${body ? `: ${body}` : ''}`);
    return;
  }
  try {
    const result = JSON.parse(body);
    if (typeof result.deletedCount === 'number') {
      console.log(`pruned ${result.deletedCount} stale runtime(s) at ${pruneUrl}`);
      return;
    }
  } catch {
    // Fall through to the generic success message.
  }
  console.log(`requested router runtime prune at ${pruneUrl}`);
}

async function devStatus(rawArgs) {
  const args = parseDevConfigArgs(rawArgs);
  const config = await loadDevConfig(args.config);
  const reloadUrl = args.reloadUrl ?? process.env.SKIFF_DEV_RELOAD_URL ?? config.reloadUrl ?? defaultDevReloadUrl;
  const statusUrl = controlUrlFromReloadUrl(reloadUrl, '/__router/health');
  const response = await fetch(statusUrl);
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`router health returned HTTP ${response.status}${body ? `: ${body}` : ''}`);
  }
  printResponseBody(body);
}

async function devClean(rawArgs) {
  const args = parseDevConfigArgs(rawArgs, { allowRoot: true });
  const root = resolve(process.cwd(), args.root ?? '.');

  const removed = [];

  // Current layout: per-service build output lives under the writable dev home,
  // keyed by the service id, alongside a sibling <dir>.lock.
  const serviceId = await readServiceId(root);
  const buildDir = serviceBuildDir(serviceId);
  for (const target of [buildDir, `${buildDir}.lock`]) {
    if (await fileExists(target)) {
      await rm(target, { recursive: true, force: true });
      removed.push(target);
    }
  }

  // Legacy layout: older builds wrote build/ and build.lock/ into the project
  // root. Clean those up too so migrated projects don't keep stale directories.
  for (const target of [join(root, 'build'), join(root, 'build.lock')]) {
    if (await fileExists(target)) {
      await rm(target, { recursive: true, force: true });
      removed.push(target);
    }
  }

  if (removed.length === 0) {
    console.log(`nothing to clean for ${serviceId}`);
    return;
  }
  for (const target of removed) {
    console.log(`removed ${target}`);
  }
}

async function packageCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'remote':
      await remoteCommand('package', args);
      return;
    case 'auth':
      await remoteAuthCommand('package', args);
      return;
    case 'publish':
      await publishPackage(args);
      return;
    case 'resolve':
      await packageResolve(args);
      return;
    case 'pull':
      await packagePull(args);
      return;
    case 'rollback':
      await packageRollback(args);
      return;
    default:
      throw new Error(`unknown package command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function serviceCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'new':
      await newService(args);
      return;
    case 'route':
      await serviceRouteCommand(args);
      return;
    case 'dev':
      await serviceDevCommand(args);
      return;
    case 'remote':
      await remoteCommand('service', args);
      return;
    case 'auth':
      await remoteAuthCommand('service', args);
      return;
    case 'publish':
      serviceCommandNotImplemented('publish');
      return;
    case 'status':
      serviceCommandNotImplemented('status');
      return;
    case 'releases':
      serviceCommandNotImplemented('releases');
      return;
    case 'rollback':
      serviceCommandNotImplemented('rollback');
      return;
    default:
      throw new Error(`unknown service command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function serviceRouteCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'add':
      await newRoute(args);
      return;
    default:
      throw new Error(`unknown service route command ${subcommand || '(missing)'}; expected add\n${usage}`);
  }
}

async function serviceDevCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'sync':
      await devSync(args);
      return;
    case 'watch':
      await devWatchRun(args);
      return;
    case 'clean':
      await devClean(args);
      return;
    case 'registry':
      await serviceDevRegistryCommand(args);
      return;
    default:
      throw new Error(`unknown service dev command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function serviceDevRegistryCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'list':
      await devWatchList(args);
      return;
    case 'add':
      await devWatchAdd(args);
      return;
    case 'remove':
      await devWatchRemove(args);
      return;
    default:
      throw new Error(`unknown service dev registry command ${subcommand || '(missing)'}\n${usage}`);
  }
}

function serviceCommandNotImplemented(command) {
  throw new Error(`skiff service ${command} is not implemented yet; service remote and auth are configured separately from package`);
}

function serviceTemplateFiles(template, root) {
  const serviceName = serviceSlugFromRoot(root);
  switch (template) {
    case 'http-api':
      return [
        projectConfigTemplateFile(),
        {
          path: 'service.yml',
          contents: [
            `id: example.com/${serviceName}`,
            'version: 0.1.0',
            'api:',
            '  todos: api.todos',
            'http:',
            '  routes:',
            '    - path: /todos',
            '      handler: root.internal.todos.create',
            '',
          ].join('\n'),
        },
        {
          path: 'api/todos.skiff',
          contents: [
            'export type CreateTodoRequest {',
            '  title: string,',
            '}',
            '',
            'export type Todo {',
            '  id: string,',
            '  title: string,',
            '  completed: bool,',
            '}',
            '',
            'export type CreateTodoResponse {',
            '  todo: Todo,',
            '}',
            '',
          ].join('\n'),
        },
        {
          path: 'internal/todos.skiff',
          contents: [
            'export function create(input: root.api.todos.CreateTodoRequest) -> root.api.todos.CreateTodoResponse {',
            '  return {',
            '    todo: {',
            '      id: "todo-1",',
            '      title: input.title,',
            '      completed: false,',
            '    },',
            '  }',
            '}',
            '',
          ].join('\n'),
        },
      ];
    case 'http-stream-proxy':
      return [
        projectConfigTemplateFile(),
        {
          path: 'service.yml',
          contents: [
            `id: example.com/${serviceName}`,
            'version: 0.1.0',
            'api:',
            '  proxy: api.proxy',
            'http:',
            '  routes:',
            '    - method: GET',
            '      path: /proxy',
            '      handler: root.internal.proxy.stream',
            '',
          ].join('\n'),
        },
        {
          path: 'api/proxy.skiff',
          contents: [
            'export type ProxyTarget {',
            '  url: string,',
            '}',
            '',
          ].join('\n'),
        },
        {
          path: 'internal/proxy.skiff',
          contents: [
            'import std',
            '',
            'export function stream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
            '  const upstream = std.http.HttpClientRequest {',
            '    method: request.method,',
            '    url: "https://example.com",',
            '    headers: std.http.forwardableHeaders(request.headers),',
            '    body: request.body,',
            '    timeoutMs: null,',
            '  }',
            '',
            '  const response = std.http.stream(upstream)',
            '  emit(std.http.streamStart(response.status, response.headers))',
            '  for chunk in response.body {',
            '    emit(std.http.streamChunk(chunk))',
            '  }',
            '  emit(std.http.streamEnd())',
            '  return null',
            '}',
            '',
          ].join('\n'),
        },
      ];
    default:
      throw new Error(`unknown service template ${template}; expected http-api or http-stream-proxy`);
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

function serviceSlugFromRoot(root) {
  const base = root.split(/[\\/]/).filter((part) => part.length > 0).at(-1) ?? 'service';
  const slug = base
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
  if (/^[a-z][a-z0-9-]*[a-z0-9]$/.test(slug)) {
    return slug;
  }
  if (/^[a-z][a-z0-9]?$/.test(slug)) {
    return slug;
  }
  return 'service';
}

async function assertEmptyServiceScaffoldTarget(root) {
  try {
    const metadata = await lstat(root);
    if (!metadata.isDirectory()) {
      throw new Error(`${root} exists and is not a directory`);
    }
    const entries = await readdir(root);
    if (entries.length > 0) {
      throw new Error(`${root} must be empty before scaffolding a service`);
    }
  } catch (error) {
    if (error?.code === 'ENOENT') {
      await mkdir(root, { recursive: true });
      return;
    }
    throw error;
  }
}

function assertHttpRoutePath(path) {
  if (!path.startsWith('/') || path.includes('\0') || path.includes('\n') || path.includes('\r')) {
    throw new Error('HTTP route path must start with / and stay on one line');
  }
  if (path.includes('?') || path.includes('#')) {
    throw new Error('HTTP route path must not contain query or fragment');
  }
  if (path.includes('{') || path.includes('}') || path.includes(':') || path.includes('*')) {
    throw new Error('HTTP route path must be a literal URL path');
  }
}

function normalizeRouteHandler(handler) {
  const parts = handler.split('.');
  if (parts.length < 3 || parts[0] !== 'root' || !parts.every((part) => sourceIdentifierPattern.test(part))) {
    throw new Error('route handler must look like root.module.function');
  }
  const functionName = parts.at(-1);
  const explicitInternal = parts[1] === 'internal';
  const moduleSegments = explicitInternal ? parts.slice(2, -1) : parts.slice(1, -1);
  if (moduleSegments.length === 0) {
    throw new Error('route handler must include an internal module path');
  }
  const configHandler = explicitInternal
    ? handler
    : ['root', 'internal', ...moduleSegments, functionName].join('.');
  return {
    configHandler,
    functionName,
    moduleSegments,
  };
}

function appendHttpRouteToServiceConfig(source, route) {
  const newline = source.includes('\r\n') ? '\r\n' : '\n';
  const hadTrailingNewline = source.endsWith('\n');
  const lines = source.split(/\r?\n/);
  if (hadTrailingNewline && lines.at(-1) === '') {
    lines.pop();
  }

  const httpIndex = lines.findIndex((line) => /^http:\s*(?:#.*)?$/.test(line));
  if (httpIndex === -1) {
    if (lines.length > 0 && lines.at(-1).trim() !== '') {
      lines.push('');
    }
    lines.push(...renderHttpRouteBlock(route));
    return `${lines.join(newline)}${newline}`;
  }

  const httpEnd = findTopLevelBlockEnd(lines, httpIndex + 1);
  const routesIndex = findHttpRoutesLine(lines, httpIndex + 1, httpEnd);
  if (routesIndex === -1) {
    lines.splice(httpEnd, 0, '  routes:', ...renderHttpRouteEntry('    ', route));
    return `${lines.join(newline)}${newline}`;
  }

  const routeIndent = leadingWhitespace(lines[routesIndex]);
  const routeListIndent = `${routeIndent}  `;
  if (/^\s*routes:\s*\[\]\s*(?:#.*)?$/.test(lines[routesIndex])) {
    lines[routesIndex] = `${routeIndent}routes:`;
    lines.splice(routesIndex + 1, 0, ...renderHttpRouteEntry(routeListIndent, route));
    return `${lines.join(newline)}${newline}`;
  }
  if (!new RegExp(`^${escapeRegExp(routeIndent)}routes:\\s*(?:#.*)?$`).test(lines[routesIndex])) {
    throw new Error('http.routes must be a YAML block list or [] before skiff service route add can update it');
  }

  const insertIndex = findIndentedBlockEnd(lines, routesIndex + 1, routeIndent.length);
  lines.splice(insertIndex, 0, ...renderHttpRouteEntry(routeListIndent, route));
  return `${lines.join(newline)}${newline}`;
}

function findTopLevelBlockEnd(lines, startIndex) {
  for (let index = startIndex; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.trim() !== '' && /^[A-Za-z_][A-Za-z0-9_-]*\s*:/.test(line)) {
      return index;
    }
  }
  return lines.length;
}

function findHttpRoutesLine(lines, startIndex, endIndex) {
  for (let index = startIndex; index < endIndex; index += 1) {
    const line = lines[index];
    const indent = leadingWhitespace(line);
    if (indent.length > 0 && /^\s*routes:\s*/.test(line)) {
      return index;
    }
  }
  return -1;
}

function findIndentedBlockEnd(lines, startIndex, parentIndentLength) {
  for (let index = startIndex; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.trim() === '') {
      continue;
    }
    if (leadingWhitespace(line).length <= parentIndentLength) {
      return index;
    }
  }
  return lines.length;
}

function renderHttpRouteBlock(route) {
  return [
    'http:',
    '  routes:',
    ...renderHttpRouteEntry('    ', route),
  ];
}

function renderHttpRouteEntry(indent, route) {
  return [
    `${indent}- path: ${quoteYamlString(route.path)}`,
    `${indent}  handler: ${quoteYamlString(route.handler)}`,
  ];
}

async function ensureRouteHandlerStub(root, routeHandler) {
  const sourcePath = join(root, 'internal', ...routeHandler.moduleSegments.slice(0, -1), `${routeHandler.moduleSegments.at(-1)}.skiff`);
  const stub = routeHandlerStub(routeHandler);
  try {
    const source = await readFile(sourcePath, 'utf8');
    if (sourceHasFunction(source, routeHandler.functionName)) {
      return { action: 'kept', path: sourcePath };
    }
    const separator = source.endsWith('\n') ? '\n' : '\n\n';
    await writeFile(sourcePath, `${source}${separator}${stub}`);
    return { action: 'updated', path: sourcePath };
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      throw error;
    }
    await mkdir(dirname(sourcePath), { recursive: true });
    await writeFile(sourcePath, stub);
    return { action: 'created', path: sourcePath };
  }
}

function routeHandlerStub(routeHandler) {
  const typePrefix = pascalCase([...routeHandler.moduleSegments.slice(-1), routeHandler.functionName].join('_'));
  const requestType = `${typePrefix}Request`;
  const responseType = `${typePrefix}Response`;
  return [
    `export type ${requestType} {}`,
    '',
    `export type ${responseType} {`,
    '  ok: bool,',
    '}',
    '',
    `export function ${routeHandler.functionName}(input: ${requestType}) -> ${responseType} {`,
    '  return { ok: true }',
    '}',
    '',
  ].join('\n');
}

function sourceHasFunction(source, functionName) {
  return new RegExp(`(?:^|\\n)\\s*(?:export\\s+)?function\\s+${escapeRegExp(functionName)}\\s*\\(`).test(source);
}

function pascalCase(value) {
  const words = value
    .split(/[^A-Za-z0-9]+/)
    .map((word) => word.trim())
    .filter((word) => word.length > 0);
  const result = words
    .map((word) => `${word[0].toUpperCase()}${word.slice(1)}`)
    .join('');
  return result || 'Route';
}

function leadingWhitespace(value) {
  return value.match(/^\s*/)?.[0] ?? '';
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

async function remoteCommand(kind, args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'use':
      await remoteUse(kind, args);
      return;
    case 'current':
      await remoteCurrent(kind, args);
      return;
    case 'ping':
      await remotePing(kind, args);
      return;
    case 'forget':
      await remoteForget(kind, args);
      return;
    default:
      throw new Error(`unknown ${kind} remote command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function remoteUse(kind, args) {
  if (args.length !== 1) {
    throw new Error(`skiff ${kind} remote use requires a URL`);
  }
  const remoteUrl = normalizeUrl(args[0]);
  await writeGlobalConfig({ [remoteSpec(kind).configKey]: remoteUrl });
  console.log(`${kind} remote: ${remoteUrl}`);
}

async function remoteCurrent(kind, args) {
  if (args.length !== 0) {
    throw new Error(`skiff ${kind} remote current does not accept arguments`);
  }
  const remoteUrl = await resolveRemoteUrl(kind);
  console.log(`${kind} remote: ${remoteUrl}`);
}

async function remotePing(kind, args) {
  if (args.length !== 0) {
    throw new Error(`skiff ${kind} remote ping does not accept arguments`);
  }
  const spec = remoteSpec(kind);
  const options = spec.pingMethod === 'POST' ? { body: {} } : {};
  const result = await remoteRequest(kind, spec.pingMethod, spec.pingPath, options);
  printJsonResult(result, false);
}

async function remoteForget(kind, args) {
  if (args.length !== 0) {
    throw new Error(`skiff ${kind} remote forget does not accept arguments`);
  }
  const spec = remoteSpec(kind);
  const config = await readGlobalConfig();
  const remoteUrl = typeof config[spec.configKey] === 'string' && config[spec.configKey].length > 0
    ? normalizeUrl(config[spec.configKey])
    : null;
  if (remoteUrl) {
    await deleteRemoteToken(kind, remoteUrl);
  }
  await deleteGlobalConfigKeys([spec.configKey]);
  console.log(`forgot ${kind} remote${remoteUrl ? ` ${remoteUrl}` : ''}`);
}

async function remoteAuthCommand(kind, args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'authorize':
      await remoteAuthAuthorize(kind, args);
      return;
    case 'status':
      await remoteAuthStatus(kind, args);
      return;
    case 'revoke':
      await remoteAuthRevoke(kind, args);
      return;
    default:
      throw new Error(`unknown ${kind} auth command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function remoteAuthAuthorize(kind, rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--no-open']),
    optionsWithValues: new Set(['--remote', '--web-url']),
  });
  if (args.positionals.length !== 0) {
    throw new Error(`skiff ${kind} auth authorize does not accept positional arguments`);
  }

  const spec = remoteSpec(kind);
  const remoteUrl = args.options.remote
    ? normalizeUrl(args.options.remote)
    : await resolveRemoteUrl(kind);
  if (args.options.remote) {
    await writeGlobalConfig({ [spec.configKey]: remoteUrl });
  }
  const start = await remoteRequest(kind, 'POST', spec.authStartPath, {
    remoteUrl,
    selector: spec.authSelector,
    body: { label: `Skiff ${kind} CLI on ${hostname()}` },
  });
  const deviceCode = stringField(start, 'deviceCode', `${kind} authorization response`);
  const userCode = stringField(start, 'userCode', `${kind} authorization response`);
  const intervalSeconds = numberField(start, 'intervalSeconds') ?? 2;
  const expiresAt = Number(stringField(start, 'expiresAt', `${kind} authorization response`));
  const authorizeUrl = args.options.webUrl
    ? appendRemoteAuthorizationParams(args.options.webUrl, kind, remoteUrl, deviceCode, userCode)
    : defaultRemoteAuthorizationUrl(kind, remoteUrl, deviceCode, userCode);

  console.log(`${kind} remote: ${remoteUrl}`);
  console.log(`open: ${authorizeUrl}`);
  console.log(`code: ${userCode}`);
  if (!args.flags.has('--no-open')) {
    await openBrowser(authorizeUrl);
  }

  const deadline = Number.isFinite(expiresAt) ? expiresAt : Date.now() + 10 * 60 * 1000;
  while (Date.now() < deadline) {
    await delay(Math.max(1, intervalSeconds) * 1000);
    const poll = await remoteRequest(kind, 'POST', spec.authTokenPath, {
      remoteUrl,
      selector: spec.authSelector,
      body: { deviceCode },
    });
    const status = stringField(poll, 'status', `${kind} authorization response`);
    if (status === 'pending') {
      continue;
    }
    if (status === 'approved') {
      const token = stringField(poll, 'token', `${kind} authorization response`);
      await writeRemoteToken(kind, remoteUrl, token);
      const user = objectField(poll, 'user');
      const email = user ? optionalStringField(user, 'email') : null;
      console.log(`authorized${email ? ` as ${email}` : ''}`);
      return;
    }
    throw new Error(`${kind} authorization ${status || 'failed'}`);
  }
  throw new Error(`${kind} authorization expired`);
}

async function remoteAuthStatus(kind, args) {
  if (args.length !== 0) {
    throw new Error(`skiff ${kind} auth status does not accept arguments`);
  }
  const remoteUrl = await resolveRemoteUrl(kind);
  const token = await readRemoteToken(kind, remoteUrl);
  console.log(`${kind} remote: ${remoteUrl}`);
  console.log(`${kind} authorization: ${token ? 'present' : 'missing'}`);
}

async function remoteAuthRevoke(kind, args) {
  if (args.length !== 0) {
    throw new Error(`skiff ${kind} auth revoke does not accept arguments`);
  }
  const remoteUrl = await resolveRemoteUrl(kind);
  await deleteRemoteToken(kind, remoteUrl);
  console.log(`revoked local ${kind} authorization for ${remoteUrl}`);
}

async function publishPackage(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--wait', '--json']),
    optionsWithValues: new Set(),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff package publish requires a package root');
  }

  const root = resolve(args.positionals[0]);
  const manifestPath = join(root, 'package.yml');
  const manifestText = await readFile(manifestPath, 'utf8');
  const manifest = parsePackageManifest(manifestText, manifestPath);
  assertRegistryPackageId(manifest.id);
  const authorityDomain = packageAuthorityDomain(manifest.id);

  const sourceArchive = await createPackageSourceArchive(root);
  try {
    const upload = await remoteRequest('package', 'POST', '/org/packages/uploads/create', {
      body: {
        authorityDomain,
        packageId: manifest.id,
        version: manifest.version,
        sourceArchiveHash: sourceArchive.hash,
        sourceArchiveSize: sourceArchive.size,
        contentType: 'application/gzip',
      },
      requireIdentity: true,
    });
    const sourceArchiveUploadToken = stringField(upload, 'uploadToken', 'package upload response');
    const sourceArchiveKey = optionalStringField(upload, 'sourceArchiveKey');
    await uploadPackageSourceArchive(upload, sourceArchive);

    const publishResponse = await remoteRequest('package', 'POST', '/org/packages/publish', {
      body: {
        authorityDomain,
        packageId: manifest.id,
        version: manifest.version,
        visibility: manifest.visibility,
        description: manifest.description,
        repositoryUrl: manifest.repositoryUrl,
        sourceArchiveHash: sourceArchive.hash,
        sourceArchiveSize: sourceArchive.size,
        sourceArchiveUploadToken,
      },
      requireIdentity: true,
    });

    let result = publishResponse;
    if (args.flags.has('--wait')) {
      const revision = objectField(publishResponse, 'revision');
      const build = objectField(publishResponse, 'build');
      const revisionId = optionalStringField(publishResponse, 'revisionId')
        ?? stringField(revision, 'revisionId', 'package publish response');
      const buildId = optionalStringField(publishResponse, 'buildId')
        ?? stringField(build, 'buildId', 'package publish response');
      const artifactArchive = packageBuildArtifactArchive(manifest, sourceArchive, revisionId, buildId, sourceArchiveKey);
      const completeResponse = await remoteRequest('package', 'POST', '/packages/builds/complete', {
        body: {
          packageId: manifest.id,
          version: manifest.version,
          revisionId,
          buildId,
          packageUnitPath: artifactArchive.packageUnitPath,
          packageUnitHash: artifactArchive.packageUnitHash,
          abiIdentity: artifactArchive.abiIdentity,
          compilerVersion: 'skiff-cli-live-test-shim@0.1.0',
          artifactArchiveHash: artifactArchive.artifactArchiveHash,
          artifactArchiveKey: artifactArchive.artifactArchiveKey,
          provenanceJson: artifactArchive.provenanceJson,
        },
        requireIdentity: true,
      });
      result = {
        ...publishResponse,
        build: objectField(completeResponse, 'build') ?? objectField(publishResponse, 'build'),
        pointer: objectField(completeResponse, 'pointer'),
        buildComplete: completeResponse,
      };
    }

    printJsonResult(result, args.flags.has('--json'));
  } finally {
    await rm(sourceArchive.tmpDir, { recursive: true, force: true });
  }
}

async function packageResolve(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--json']),
    optionsWithValues: new Set(),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff package resolve requires a package ref');
  }
  const result = await remoteRequest('package', 'POST', '/packages/resolve', {
    body: { ref: args.positionals[0] },
  });
  printJsonResult(result, args.flags.has('--json'));
}

async function packagePull(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--json']),
    optionsWithValues: new Set(['--out', '--revision']),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff package pull requires a package ref');
  }

  const query = args.options.revision
    ? { revisionId: args.options.revision }
    : { ref: args.positionals[0] };
  const result = await remoteRequest('package', 'POST', '/packages/download', { body: query });
  const outDir = args.options.out
    ? resolve(args.options.out)
    : await packagePullTargetDir(result);
  if (!args.options.out) {
    await assertPackagePullTarget(outDir);
  }
  const files = await materializeSourceArchiveDownload(result, outDir);
  const output = {
    ...result,
    materializedPath: outDir,
    materializedFiles: files,
  };
  if (args.flags.has('--json')) {
    printJsonResult(output, true);
    return;
  }
  console.log(`pulled ${files.length} file(s) to ${outDir}`);
}

async function packagePullTargetDir(result) {
  const revision = result?.revision;
  const id = optionalStringField(revision, 'packageId')
    ?? optionalStringField(revision, 'id')
    ?? optionalStringField(result, 'packageId');
  const version = optionalStringField(revision, 'version')
    ?? optionalStringField(result, 'version');
  if (typeof id !== 'string' || id.length === 0 || typeof version !== 'string' || version.length === 0) {
    throw new Error('pull response did not include package id and version');
  }
  const project = await readProjectPackageDirs(process.cwd());
  if (project.packageDirs.length === 0) {
    throw new Error(
      `skiff package pull without --out requires ${project.configPath ?? 'a skiff.yml or skiff.local.yml'} with at least one packageDirs entry`
    );
  }
  const root = project.packageDirs[0];
  await mkdir(root, { recursive: true });
  return join(root, ...packageStorePathParts(id, version));
}

function packageStorePathParts(id, version) {
  if (!validPackageId(id)) {
    throw new Error(`package id ${id} cannot be materialized as a package store path`);
  }
  if (version.length === 0 || version === '.' || version === '..' || version.includes('/') || version.includes('\\')) {
    throw new Error(`package version ${version} cannot be materialized as a package store path`);
  }
  return [publicationStorageSegment(id), version];
}

async function assertPackagePullTarget(path) {
  try {
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink()) {
      throw new Error(`refusing to overwrite package symlink ${path}; remove it or choose another packages-dir`);
    }
    if (!metadata.isDirectory()) {
      throw new Error(`package pull target ${path} exists and is not a directory`);
    }
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
}

async function packageRollback(rawArgs) {
  const args = parseRegistryCommandArgs(rawArgs, {
    flags: new Set(['--json']),
    optionsWithValues: new Set(['--to']),
  });
  if (args.positionals.length !== 1) {
    throw new Error('skiff package rollback requires a package ref');
  }
  if (!args.options.to) {
    throw new Error('skiff package rollback requires --to <revisionId>');
  }
  const ref = parsePackageRef(args.positionals[0]);
  assertRegistryPackageId(ref.id);
  const result = await remoteRequest('package', 'POST', '/org/packages/rollback', {
    body: {
      packageId: ref.id,
      version: ref.version,
      toRevisionId: args.options.to,
    },
    requireIdentity: true,
  });
  printJsonResult(result, args.flags.has('--json'));
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

async function remoteRequest(kind, method, pathname, options = {}) {
  const remoteUrl = options.remoteUrl ?? await resolveRemoteUrl(kind);
  const spec = remoteSpec(kind);
  const selector = options.selector ?? spec.selector;
  const url = serviceEndpointUrl(remoteUrl, pathname, options.query, selector);
  const headers = await remoteIdentityHeaders(kind, remoteUrl, { requireIdentity: options.requireIdentity });
  const requestOptions = { headers };
  if (Object.hasOwn(options, 'body')) {
    requestOptions.body = options.body;
  }
  return jsonServiceRequest(`${kind} remote`, method, url, requestOptions);
}

async function jsonServiceRequest(serviceLabel, method, url, options = {}) {
  const headers = { ...(options.headers ?? {}) };
  let body;
  if (Object.hasOwn(options, 'body')) {
    headers['content-type'] = 'application/json; charset=utf-8';
    body = JSON.stringify(options.body);
  }

  let response;
  try {
    response = await fetch(url, { method, headers, body });
  } catch (error) {
    throw new Error(`${serviceLabel} request failed for ${url}: ${formatError(error)}`);
  }

  const responseText = await response.text();
  const parsed = parseJsonResponse(responseText);
  if (!response.ok) {
    const detail = errorDetail(responseText, parsed);
    throw new Error(`${serviceLabel} ${method} ${url.pathname} returned HTTP ${response.status}${detail}`);
  }
  return parsed ?? responseText;
}

function serviceEndpointUrl(baseUrl, pathname, query = {}, selector = null) {
  const base = new URL(baseUrl);
  const url = new URL(base.toString());
  const basePath = base.pathname.endsWith('/') ? base.pathname.slice(0, -1) : base.pathname;
  const endpointPath = pathname.startsWith('/') ? pathname : `/${pathname}`;
  url.pathname = `${basePath && basePath !== '/' ? basePath : ''}${endpointPath}`;
  url.hash = '';
  for (const [key, value] of Object.entries({ ...(selector ?? {}), ...(query ?? {}) })) {
    if (value !== undefined && value !== null) {
      url.searchParams.set(key, value);
    }
  }
  return url;
}

async function remoteIdentityHeaders(kind, remoteUrl, options = {}) {
  const headers = { accept: 'application/json' };
  const token = await readRemoteToken(kind, remoteUrl);
  if (token) {
    headers.authorization = `Bearer ${token}`;
  } else if (options.requireIdentity) {
    throw new Error(`${kind} authorization is required; run "skiff ${kind} auth authorize" or set ${remoteSpec(kind).envToken}`);
  }
  return headers;
}

async function readRemoteToken(kind, remoteUrl) {
  const spec = remoteSpec(kind);
  if (process.env[spec.envToken] && process.env[spec.envToken].length > 0) {
    return process.env[spec.envToken];
  }
  const keychainToken = await readMacOSKeychainServiceToken(spec.credentialService, remoteUrl);
  if (keychainToken) {
    return keychainToken;
  }
  const credentials = await readGlobalCredentials();
  const token = credentials?.[spec.credentialsKey]?.[remoteUrl];
  return typeof token === 'string' && token.length > 0 ? token : null;
}

async function writeRemoteToken(kind, remoteUrl, token) {
  const spec = remoteSpec(kind);
  if (process.platform === 'darwin' && await writeMacOSKeychainServiceToken(spec.credentialService, remoteUrl, token)) {
    return;
  }
  const credentials = await readGlobalCredentials();
  const tokens = {
    ...(isPlainObject(credentials[spec.credentialsKey]) ? credentials[spec.credentialsKey] : {}),
    [remoteUrl]: token,
  };
  await writeGlobalCredentials({ ...credentials, [spec.credentialsKey]: tokens });
}

async function deleteRemoteToken(kind, remoteUrl) {
  const spec = remoteSpec(kind);
  await deleteMacOSKeychainServiceToken(spec.credentialService, remoteUrl);
  const credentials = await readGlobalCredentials();
  if (!isPlainObject(credentials[spec.credentialsKey]) || !Object.hasOwn(credentials[spec.credentialsKey], remoteUrl)) {
    return;
  }
  const tokens = { ...credentials[spec.credentialsKey] };
  delete tokens[remoteUrl];
  await writeGlobalCredentials({ ...credentials, [spec.credentialsKey]: tokens });
}

async function readGlobalCredentials() {
  try {
    const parsed = JSON.parse(await readFile(globalCredentialsPath, 'utf8'));
    return isPlainObject(parsed) ? parsed : {};
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return {};
    }
    throw error;
  }
}

async function writeGlobalCredentials(credentials) {
  await mkdir(dirname(globalCredentialsPath), { recursive: true });
  await writeFile(globalCredentialsPath, `${JSON.stringify(credentials, null, 2)}\n`, { mode: 0o600 });
  await chmod(globalCredentialsPath, 0o600);
}

async function readMacOSKeychainServiceToken(service, account) {
  if (process.platform !== 'darwin') {
    return null;
  }
  const result = await spawnCapture('security', [
    'find-generic-password',
    '-a',
    account,
    '-s',
    service,
    '-w',
  ]);
  return result.code === 0 && result.stdout.trim().length > 0 ? result.stdout.trim() : null;
}

async function writeMacOSKeychainServiceToken(service, account, token) {
  if (process.platform !== 'darwin') {
    return false;
  }
  const result = await spawnCapture('security', [
    'add-generic-password',
    '-a',
    account,
    '-s',
    service,
    '-w',
    token,
    '-U',
  ]);
  return result.code === 0;
}

async function deleteMacOSKeychainServiceToken(service, account) {
  if (process.platform !== 'darwin') {
    return;
  }
  await spawnCapture('security', [
    'delete-generic-password',
    '-a',
    account,
    '-s',
    service,
  ]);
}

function spawnCapture(command, args) {
  return new Promise((resolvePromise) => {
    const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      resolvePromise({ code: -1, stdout, stderr: formatError(error) });
    });
    child.on('exit', (code) => {
      resolvePromise({ code: code ?? -1, stdout, stderr });
    });
  });
}

async function openBrowser(url) {
  const command = process.platform === 'darwin'
    ? 'open'
    : process.platform === 'win32'
      ? 'cmd'
      : 'xdg-open';
  const args = process.platform === 'win32' ? ['/c', 'start', '', url] : [url];
  const child = spawn(command, args, {
    detached: true,
    stdio: 'ignore',
  });
  child.on('error', () => {});
  child.unref();
}

function defaultRemoteAuthorizationUrl(kind, remoteUrl, deviceCode, userCode) {
  const url = new URL(remoteUrl);
  url.pathname = '/';
  url.hash = '';
  url.search = '';
  url.searchParams.set('cli_device_code', deviceCode);
  url.searchParams.set('cli_user_code', userCode);
  url.searchParams.set(`${kind}_remote_url`, remoteUrl);
  if (kind === 'package') {
    url.searchParams.set('registry_url', remoteUrl);
  }
  return url.toString();
}

function appendRemoteAuthorizationParams(webUrl, kind, remoteUrl, deviceCode, userCode) {
  const url = new URL(webUrl);
  url.searchParams.set('cli_device_code', deviceCode);
  url.searchParams.set('cli_user_code', userCode);
  url.searchParams.set(`${kind}_remote_url`, remoteUrl);
  if (kind === 'package') {
    url.searchParams.set('registry_url', remoteUrl);
  }
  return url.toString();
}

function stringField(value, key, label = 'response') {
  if (!isPlainObject(value) || typeof value[key] !== 'string' || value[key].length === 0) {
    throw new Error(`${label} missing ${key}`);
  }
  return value[key];
}

function optionalStringField(value, key) {
  return isPlainObject(value) && typeof value[key] === 'string' ? value[key] : null;
}

function numberField(value, key) {
  return isPlainObject(value) && typeof value[key] === 'number' ? value[key] : null;
}

function objectField(value, key) {
  return isPlainObject(value) && isPlainObject(value[key]) ? value[key] : null;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function parseJsonResponse(text) {
  if (!text.trim()) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function errorDetail(responseText, parsed) {
  if (parsed?.error?.message) {
    return `: ${parsed.error.message}`;
  }
  if (responseText.trim()) {
    return `: ${responseText.trim()}`;
  }
  return '';
}

function printJsonResult(result, json) {
  if (json || typeof result !== 'string') {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  printResponseBody(result);
}

function parsePackageManifest(text, path) {
  const values = {};
  for (const line of text.split(/\r?\n/)) {
    if (/^\s*(#.*)?$/.test(line) || /^---\s*$/.test(line)) {
      continue;
    }
    const match = /^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/.exec(line);
    if (!match) {
      continue;
    }
    const [, key, rawValue] = match;
    if (key === 'id' || key === 'version' || key === 'visibility' || key === 'description' || key === 'repositoryUrl') {
      values[key] = parseYamlStringScalar(rawValue);
    }
  }
  if (!values.id || !values.version) {
    throw new Error(`${path} must define top-level id and version`);
  }
  if (!validPackageId(values.id) || !validVersion(values.version)) {
    throw new Error(`${path} contains an invalid package id or version`);
  }
  if (values.visibility && values.visibility !== 'public' && values.visibility !== 'private') {
    throw new Error(`${path} visibility must be public or private`);
  }
  return {
    id: values.id,
    version: values.version,
    visibility: values.visibility,
    description: values.description,
    repositoryUrl: values.repositoryUrl,
  };
}

async function readPackageManifest(root) {
  const manifestPath = join(root, 'package.yml');
  return parsePackageManifest(await readFile(manifestPath, 'utf8'), manifestPath);
}

async function createPackageSourceArchive(root) {
  const absoluteRoot = resolve(root);
  const tmpDir = await mkdtemp(join(tmpdir(), 'skiff-package-source-'));
  const archivePath = join(tmpDir, 'source.tgz');
  const filesPath = join(tmpDir, 'files.txt');
  const files = await collectPackageSourceArchivePaths(absoluteRoot);
  await writeFile(filesPath, `${files.join('\n')}\n`);
  await runTar(['-czf', archivePath, '-C', absoluteRoot, '-T', filesPath], 'create package source archive');
  const bytes = await readFile(archivePath);
  return {
    tmpDir,
    path: archivePath,
    bytes,
    hash: `sha256:${sha256Buffer(bytes)}`,
    size: bytes.length,
  };
}

async function collectPackageSourceArchivePaths(root) {
  const files = ['package.yml'];
  await collectSkiffFilePaths(root, root, files);
  return files.sort((left, right) => left.localeCompare(right));
}

async function collectSkiffFilePaths(root, directory, files) {
  const entries = (await readdir(directory, { withFileTypes: true }))
    .sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (entry.name === 'package.yml') {
      continue;
    }
    const entryPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (shouldSkipSourceArchiveDirectory(entry.name)) {
        continue;
      }
      await collectSkiffFilePaths(root, entryPath, files);
      continue;
    }
    if (!entry.isFile() || !entry.name.endsWith('.skiff')) {
      continue;
    }
    const relPath = relative(root, entryPath).split(sep).join('/');
    safeArchivePathParts(relPath);
    files.push(relPath);
  }
}

function shouldSkipSourceArchiveDirectory(name) {
  const lower = name.toLowerCase();
  return name.startsWith('.')
    || lower === 'node_modules'
    || lower === 'build'
    || lower === 'dist'
    || lower === 'out'
    || lower === 'target'
    || lower === 'coverage'
    || lower === 'tmp'
    || lower === 'temp'
    || lower === 'cache'
    || lower.includes('cache');
}

async function uploadPackageSourceArchive(upload, archive) {
  const uploadUrl = optionalStringField(upload, 'uploadUrl');
  if (uploadUrl) {
    await uploadArchiveToUrl(uploadUrl, upload, archive);
    return;
  }
  const uploadToken = stringField(upload, 'uploadToken', 'package upload response');
  await remoteRequest('package', 'POST', '/org/packages/uploads/put', {
    body: {
      uploadToken,
      sourceArchiveKey: optionalStringField(upload, 'sourceArchiveKey'),
      sourceArchiveHash: archive.hash,
      sourceArchiveSize: archive.size,
      contentType: 'application/gzip',
      sourceArchiveBase64: archive.bytes.toString('base64'),
    },
    requireIdentity: true,
  });
}

async function uploadArchiveToUrl(uploadUrl, upload, archive) {
  const headers = { 'content-type': 'application/gzip' };
  if (isPlainObject(upload.headers)) {
    for (const [key, value] of Object.entries(upload.headers)) {
      if (typeof value === 'string') {
        headers[key] = value;
      }
    }
  }
  const method = optionalStringField(upload, 'uploadMethod') ?? 'PUT';
  let response;
  try {
    response = await fetch(uploadUrl, { method, headers, body: archive.bytes });
  } catch (error) {
    throw new Error(`package source archive upload failed for ${uploadUrl}: ${formatError(error)}`);
  }
  if (!response.ok) {
    throw new Error(`package source archive upload returned HTTP ${response.status}: ${await response.text()}`);
  }
}

function packageBuildArtifactArchive(manifest, sourceArchive, revisionId, buildId, sourceArchiveKey) {
  const sourceHashValue = sourceArchive.hash.replace(/^sha256:/, '');
  const packageUnitPath = `units/packages/${publicationStorageSegment(manifest.id)}/${manifest.version}/${sourceHashValue}.json`;
  const artifactSeed = JSON.stringify({
    schemaVersion: 'skiff-cli-package-artifact-v1',
    packageId: manifest.id,
    version: manifest.version,
    revisionId,
    buildId,
    packageUnitPath,
    sourceArchiveHash: sourceArchive.hash,
    sourceArchiveKey,
  });
  const artifactArchiveHash = `sha256:${sha256Text(artifactSeed)}`;
  const artifactHashValue = artifactArchiveHash.replace(/^sha256:/, '');
  const packageUnitHash = `sha256:${sha256Text(`${packageUnitPath}\n${artifactArchiveHash}`)}`;
  return {
    packageUnitPath,
    packageUnitHash,
    abiIdentity: `skiff-package-abi:${packageUnitHash}`,
    artifactArchiveHash,
    artifactArchiveKey: `blobs/sha256/${artifactHashValue}.tgz`,
    provenanceJson: JSON.stringify({
      builder: 'skiff-cli-live-test-shim',
      sourceArchiveHash: sourceArchive.hash,
      sourceArchiveKey,
      revisionId,
      buildId,
    }),
  };
}

async function materializeSourceArchiveDownload(result, outDir) {
  const archive = await packageDownloadArchiveBytes(result);
  const expectedHash = packageDownloadArchiveHash(result);
  if (expectedHash && archive.hash !== expectedHash) {
    throw new Error(`downloaded archive hash mismatch: expected ${expectedHash}, got ${archive.hash}`);
  }
  const expectedSize = packageDownloadArchiveSize(result);
  if (expectedSize !== null && archive.bytes.length !== expectedSize) {
    throw new Error(`downloaded archive size mismatch: expected ${expectedSize}, got ${archive.bytes.length}`);
  }
  return extractTgzArchive(archive.bytes, outDir);
}

async function packageDownloadArchiveBytes(result) {
  const base64 = optionalStringField(result, 'sourceArchiveBase64');
  if (base64) {
    const bytes = Buffer.from(base64, 'base64');
    return { bytes, hash: `sha256:${sha256Buffer(bytes)}` };
  }
  const url = optionalStringField(result, 'downloadUrl');
  if (!url) {
    throw new Error('package download response did not include a source archive URL or bytes');
  }
  let response;
  try {
    response = await fetch(url);
  } catch (error) {
    throw new Error(`package archive download failed for ${url}: ${formatError(error)}`);
  }
  if (!response.ok) {
    throw new Error(`package archive download returned HTTP ${response.status}: ${await response.text()}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  return { bytes, hash: `sha256:${sha256Buffer(bytes)}` };
}

function packageDownloadArchiveHash(result) {
  return optionalStringField(result, 'sourceArchiveHash');
}

function packageDownloadArchiveSize(result) {
  return numberField(result, 'sourceArchiveSize');
}

async function extractTgzArchive(bytes, outDir) {
  const absoluteOut = resolve(outDir);
  await mkdir(absoluteOut, { recursive: true });
  const tmpDir = await mkdtemp(join(tmpdir(), 'skiff-package-pull-'));
  const archivePath = join(tmpDir, 'source.tgz');
  await writeFile(archivePath, bytes);
  try {
    const list = await runTar(['-tzf', archivePath], 'inspect package source archive');
    const files = list.stdout
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.length > 0 && line !== './' && !line.endsWith('/'));
    for (const file of files) {
      safeArchivePathParts(file);
    }
    await runTar(['-xzf', archivePath, '-C', absoluteOut], 'extract package source archive');
    return files;
  } finally {
    await rm(tmpDir, { recursive: true, force: true });
  }
}

async function runTar(args, label) {
  const result = await spawnCapture('tar', args);
  if (result.code !== 0) {
    throw new Error(`failed to ${label}: ${result.stderr.trim() || result.stdout.trim() || `tar exited ${result.code}`}`);
  }
  return result;
}

function safeArchivePathParts(relPath) {
  if (typeof relPath !== 'string' || relPath.length === 0 || relPath.includes('\0') || relPath.includes('\n') || relPath.includes('\r')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  if (relPath.startsWith('/') || relPath.includes('\\')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  const parts = relPath.split('/');
  if (parts.some((part) => part.length === 0 || part === '.' || part === '..')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  return parts;
}

function parsePackageRef(ref) {
  const parts = ref.split('@');
  if (parts.length !== 2 || !validPackageId(parts[0]) || !validVersion(parts[1])) {
    throw new Error(`invalid package ref ${ref}; expected id@version`);
  }
  return { id: parts[0], version: parts[1] };
}

function assertRegistryPackageId(packageId) {
  const parts = packageId.split('/');
  if (parts.length !== 2 || !parts[0] || !parts[1] || !packageRegistryNamePattern.test(parts[1])) {
    throw new Error(`invalid package id ${packageId}; expected authority/name with name matching ${packageRegistryNamePattern}`);
  }
}

function packageAuthorityDomain(packageId) {
  return packageId.split('/')[0];
}

function validPackageId(packageId) {
  return isPublicationId(packageId);
}

function validVersion(version) {
  return version.length > 0
    && !version.includes('/')
    && !version.includes('\\')
    && !version.includes('..');
}

function sha256Text(value) {
  return createHash('sha256').update(value).digest('hex');
}

function sha256Buffer(value) {
  return createHash('sha256').update(value).digest('hex');
}

function parseRootCommand(rawArgs, spec) {
  const options = {};
  const flags = new Set();
  let root;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (spec.flags.has(arg)) {
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
      options[toCamelOption(optionName)] = value;
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

function parseDevWatchRegistryArgs(rawArgs, spec) {
  return parseRegistryCommandArgs(rawArgs, {
    flags: new Set(),
    optionsWithValues: spec.optionsWithValues,
  });
}

function devWatchRegistryPath(configPath) {
  return resolve(configPath ?? defaultWatchRegistryPath);
}

async function readDevWatchRegistry(registryPath) {
  let raw;
  try {
    raw = JSON.parse(await readFile(registryPath, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { services: [] };
    }
    throw error;
  }
  if (!isPlainObject(raw)) {
    throw new Error(`${registryPath} must be a JSON object`);
  }
  if (raw.services === undefined) {
    return { services: [] };
  }
  if (!Array.isArray(raw.services)) {
    throw new Error(`${registryPath} services must be an array`);
  }
  const services = raw.services.map((entry, index) => normalizeWatchService(entry, `${registryPath} services[${index}]`));
  services.sort(compareWatchServices);
  return { services };
}

async function writeDevWatchRegistry(registryPath, registry) {
  await mkdir(dirname(registryPath), { recursive: true });
  await writeFile(registryPath, `${JSON.stringify({
    services: registry.services.map(watchServiceJson),
  }, null, 2)}\n`);
}

function watchServiceJson(service) {
  const result = {
    root: service.root,
    serviceId: service.serviceId,
    profile: service.profile,
  };
  if ((service.packageDirs?.length ?? 0) > 0) {
    result.packageDirs = service.packageDirs;
  }
  return result;
}

function normalizeWatchService(value, label) {
  if (!isPlainObject(value)) {
    throw new Error(`${label} must be an object`);
  }
  const root = requiredPlainString(value.root, `${label}.root`);
  const serviceId = requiredPlainString(value.serviceId, `${label}.serviceId`);
  const profile = value.profile === undefined
    ? 'dev'
    : requiredPlainString(value.profile, `${label}.profile`);
  if (!validPublicationId(serviceId)) {
    throw new Error(`${label}.serviceId must be a publication id`);
  }
  const packageDirs = value.packageDirs === undefined
    ? undefined
    : readPlainStringList(value.packageDirs, `${label}.packageDirs`).map((path) => resolve(path));
  return {
    root: resolve(root),
    serviceId,
    profile,
    packageDirs,
  };
}

function readPlainStringList(value, label) {
  if (!Array.isArray(value)) {
    throw new Error(`${label} must be an array`);
  }
  return value.map((item, index) => requiredPlainString(item, `${label}[${index}]`));
}

function compareWatchServices(left, right) {
  return left.root.localeCompare(right.root)
    || left.serviceId.localeCompare(right.serviceId)
    || left.profile.localeCompare(right.profile);
}

async function readServiceId(root) {
  const serviceConfigPath = join(root, 'service.yml');
  let source;
  try {
    source = await readFile(serviceConfigPath, 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`${root} must contain service.yml`);
    }
    throw error;
  }
  const match = source.match(/^id:\s*(.+?)\s*$/m);
  if (!match) {
    throw new Error(`${serviceConfigPath} must declare top-level id`);
  }
  return parseYamlStringScalar(match[1]);
}

function requiredPlainString(value, label) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function validPublicationId(publicationId) {
  return isPublicationId(publicationId);
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

function parseDevConfigArgs(rawArgs, options = {}) {
  const result = {
    config: process.env.SKIFF_DEV_CONFIG ?? process.env.SKIFF_DEV_SYNC_CONFIG,
    reloadUrl: undefined,
    root: undefined,
  };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--config') {
      result.config = resolve(requireNext(rawArgs, index, '--config'));
      index += 1;
    } else if (arg.startsWith('--config=')) {
      result.config = resolve(arg.slice('--config='.length));
    } else if (arg === '--reload-url') {
      result.reloadUrl = requireNext(rawArgs, index, '--reload-url');
      index += 1;
    } else if (arg.startsWith('--reload-url=')) {
      result.reloadUrl = arg.slice('--reload-url='.length);
    } else if (options.allowRoot && arg === '--root') {
      result.root = requireNext(rawArgs, index, '--root');
      index += 1;
    } else if (options.allowRoot && arg.startsWith('--root=')) {
      result.root = arg.slice('--root='.length);
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
      reloadUrl: raw.reloadUrl,
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

function routerDevConfig(options) {
  return renderRouterConfig({
    profile: 'dev',
    host: '0.0.0.0',
    artifactRoots: [options.artifactRoot],
    identityCliPath: options.identityCliPath,
    devReload: true,
    requestTimeoutMs: 20000,
    httpPort: 4000,
    runtimePort: 4001,
    runtimePath: '/runtime',
    serviceDbMongoUrl: options.serviceDbMongoUrl,
    telemetryEndpoint: 'ws://127.0.0.1:4002/telemetry',
    rewrite: [
      { host: 'account.localhost', service: 'skiff.run/account', version: '0.1.0' },
      { host: 'registry.localhost', service: 'skiff.run/registry', version: '0.1.0' },
    ],
  });
}

function runtimeDevConfig(options) {
  return renderRuntimeConfig({
    routerUrl: 'ws://127.0.0.1:4001/runtime',
    runtimeHome: options.runtimeHome,
    artifactRoots: [options.artifactRoot],
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

function controlUrlFromReloadUrl(reloadUrl, pathname) {
  const url = new URL(reloadUrl);
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

async function readGlobalConfig() {
  try {
    return JSON.parse(await readFile(globalConfigPath, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return {};
    }
    throw error;
  }
}

async function writeGlobalConfig(config) {
  await mkdir(dirname(globalConfigPath), { recursive: true });
  let current = {};
  try {
    await access(globalConfigPath, fsConstants.F_OK);
    current = await readGlobalConfig();
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      throw error;
    }
  }
  await writeFile(globalConfigPath, `${JSON.stringify({ ...current, ...config }, null, 2)}\n`);
}

async function deleteGlobalConfigKeys(keys) {
  const current = await readGlobalConfig();
  let changed = false;
  for (const key of keys) {
    if (Object.hasOwn(current, key)) {
      delete current[key];
      changed = true;
    }
  }
  if (changed) {
    await mkdir(dirname(globalConfigPath), { recursive: true });
    await writeFile(globalConfigPath, `${JSON.stringify(current, null, 2)}\n`);
  }
}

async function resolveRemoteUrl(kind) {
  const spec = remoteSpec(kind);
  if (process.env[spec.envUrl] && process.env[spec.envUrl].length > 0) {
    return normalizeUrl(process.env[spec.envUrl]);
  }
  const config = await readGlobalConfig();
  if (typeof config[spec.configKey] !== 'string' || config[spec.configKey].length === 0) {
    throw new Error(`no ${kind} remote configured; run "skiff ${kind} remote use <url>" first`);
  }
  return normalizeUrl(config[spec.configKey]);
}

function remoteSpec(kind) {
  if (kind === 'package') {
    return {
      authStartPath: '/cli/authorize/start',
      authTokenPath: '/cli/authorize/token',
      authSelector: accountServiceSelector,
      configKey: 'packageRemoteUrl',
      credentialService: packageCredentialService,
      credentialsKey: 'packageRemoteTokens',
      envToken: 'SKIFF_PACKAGE_TOKEN',
      envUrl: 'SKIFF_PACKAGE_REMOTE_URL',
      pingMethod: 'POST',
      pingPath: '/ping',
      selector: packageRegistryServiceSelector,
    };
  }
  if (kind === 'service') {
    return {
      authStartPath: '/cli/authorize/start',
      authTokenPath: '/cli/authorize/token',
      configKey: 'serviceRemoteUrl',
      credentialService: serviceCredentialService,
      credentialsKey: 'serviceRemoteTokens',
      envToken: 'SKIFF_SERVICE_TOKEN',
      envUrl: 'SKIFF_SERVICE_REMOTE_URL',
      pingMethod: 'POST',
      pingPath: '/services/health',
    };
  }
  throw new Error(`unknown remote kind ${kind}`);
}

function normalizeUrl(value) {
  const url = new URL(value);
  url.hash = '';
  return url.toString();
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
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd,
      stdio: 'inherit',
      env: process.env,
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise();
        return;
      }
      reject(new Error(`${command} exited with ${signal ?? code}`));
    });
  });
}

function formatError(error) {
  return error?.message || String(error);
}
