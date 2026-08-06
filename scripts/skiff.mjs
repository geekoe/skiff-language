#!/usr/bin/env node
import { constants as fsConstants } from 'node:fs';
import { access, chmod, lstat, mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoBuildEnv, cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { runAttachedCommand } from './lib/command-execution.mjs';
import { runAuthoringObjectCommand } from './lib/package-service-authoring.mjs';
import { runAssemblyStateSyncCommand } from './lib/assembly-state-sync.mjs';
import { runDevRegistryCommand } from './lib/package-service-dev-registry.mjs';
import { devRuntimePaths } from './lib/dev-runtime-paths.mjs';
import {
  runInIsolatedTestRuntime,
  shouldUseIsolatedTestRuntime,
} from './lib/isolated-test-runtime.mjs';
import { renderIsolatedRuntimeLogEvidence } from './lib/isolated-test-runtime-log-evidence.mjs';
import { runOwnedCommand } from './lib/owned-command.mjs';
import {
  countTestCases,
  deriveBaseServices,
  discoverTestFiles,
  partitionTestFiles,
  planSourcePublish,
  publishSources,
  readSourceManifest,
  renderPlan,
  resolveBasePair,
  runShardedTests,
} from './lib/test-orchestration.mjs';
import {
  defaultProjectPackageDir,
  readProjectPackageDirs,
} from './lib/project-config.mjs';
import { renderRouterConfig, renderRuntimeConfig, renderTelemetryConfig } from './lib/runtime-stack-config.mjs';
import { DEFAULT_ACTIVATION_PREPARE_TIMEOUT_MS } from './lib/activation-timeout.mjs';
import { buildStack } from './lib/stack-build.mjs';
import { parseStackConfigDirArg } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = dirname(scriptDir);
const homeDir = process.env.HOME || process.env.USERPROFILE || '.';
const defaultDevHome = join(skiffRoot, '.stack', 'dev-home');
const defaultResolvedDevHome = resolveDevHome(process.env.SKIFF_DEV_HOME);
const defaultWatchRegistryPath = join(defaultResolvedDevHome, 'watch.json');
const defaultBinDir = join(homeDir, 'bin');
const defaultDevControlUrl = 'http://127.0.0.1:4001';
const defaultLocalMongoUrl = 'mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false';

const usage = `usage:
  skiff test <package-root-or-file>... --artifact-root <dir> [--base-assembly <identity> --base-config-snapshot <identity>] [--sources <manifest.json>] [--fresh] [--plan] [--shards <n>] [--max-cases <n>] [--live --activation-url <url> --ingress-url <url> --profile <id> --expected-generation <n>] [--deny-skips] [--require-tests]
  skiff project init [root] [--force]
  skiff project paths [root] [--json]
  skiff dev init --http-max-request-bytes <bytes> --http-max-response-bytes <bytes> [--activation-prepare-timeout-ms <ms>] [--dev-home <dir>] [--bin-dir <dir>] [--service-db-mongo-url <url>] [--telemetry-db <db>] [--telemetry-mongo-url <url>] [--force] [--no-bin]
  skiff dev paths [--dev-home <dir>] [--json]
  skiff dev status [--config <path>] [--control-url <url>]
  skiff service dev registry list [--config <path>]
  skiff service dev registry add <package-or-service-root> [--profile <name>] [--config <path>]
  skiff service dev registry remove <service-id-or-root> [--config <path>]
  skiff instance <up|restart|status|down|supervise|repair> [--runtime <dir>] [component]
  skiff watch [--once] [--runtime <dir>] --config <watchDir> [--poll-interval-ms <ms>] [--build-only] [--json]
  skiff package build <root> --artifact-root <dir> [--profile <name>] [--json]
  skiff package publish <root> --artifact-root <dir> [--profile <name>] [--json]
  skiff assembly <build|publish> --artifact-root <dir> --profile <name> [--root-deployment '<exact ServiceDeploymentRef JSON>']... [--json]
  skiff assembly activate --artifact-root <dir> --profile <name> [--root-deployment '<exact ServiceDeploymentRef JSON>']... --config-snapshot '<exact RuntimeConfigSnapshotRef JSON>' --expected-generation <n> [--activation-url <url>] [--activation-id <id>] [--json]
  skiff assembly sync-state --artifact-root <dir> --profile <name> --activation-url <url> --mongo-url <url> [--json]
  skiff stack build --configDir <dir> [--profile debug|release]
  skiff stack init --configDir <dir>
  skiff stack deploy --configDir <dir>
  skiff stack status --configDir <dir>
  skiff stack validate --configDir <dir>

The dev registry watches only explicitly listed package and service roots;
service package dependencies are not discovered as local source roots.
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
    case 'watch':
      await run('node', [join(scriptDir, 'skiff-watch.mjs'), ...args], process.cwd());
      return;
    case 'service':
      await serviceCommand(args);
      return;
    case 'instance':
      await run('node', [join(scriptDir, 'skiff-instance.mjs'), ...args], process.cwd());
      return;
    case 'package':
      await packageCommand(args);
      return;
    case 'assembly':
      if (args[0] === 'sync-state') {
        await runAssemblyStateSyncCommand(args.slice(1), { skiffRoot });
        return;
      }
      await runAuthoringObjectCommand(command, args, { skiffRoot });
      return;
    case 'stack':
      await stackCommand(args);
      return;
    default:
      throw new Error(`unknown command ${command}\n${usage}`);
  }
}

async function stackCommand(args) {
  const subcommand = args.shift();
  switch (subcommand) {
    case 'build':
      await stackBuild(args);
      return;
    case 'init':
      await run('node', [join(scriptDir, 'skiff-stack-init.mjs'), ...args], process.cwd());
      return;
    case 'deploy':
      await run('node', [join(scriptDir, 'deploy-runtime-stack.mjs'), ...args], process.cwd());
      return;
    case 'status':
      await run('node', [join(scriptDir, 'skiff-stack-status.mjs'), ...args], process.cwd());
      return;
    case 'validate':
      await run('node', [join(scriptDir, 'skiff-stack-validate.mjs'), ...args], process.cwd());
      return;
    default:
      throw new Error(`unknown stack command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function stackBuild(rawArgs) {
  const parsed = parseStackConfigDirArg(rawArgs, { options: ['--profile'] });
  const profileOverride = parsed.profile;
  if (
    profileOverride !== undefined
    && profileOverride !== 'debug'
    && profileOverride !== 'release'
  ) {
    throw new Error(`--profile must be "debug" or "release"; got ${profileOverride}`);
  }
  const result = await buildStack({
    configDir: parsed.configDir,
    skiffRoot,
    profileOverride,
  });
  console.log(JSON.stringify(result, null, 2));
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
    default:
      throw new Error(`unknown dev command ${subcommand || '(missing)'}\n${usage}`);
  }
}

async function serviceCommand(args) {
  const subcommand = args.shift();
  if (subcommand !== 'dev') {
    throw new Error(`unknown service command ${subcommand || '(missing)'}\n${usage}`);
  }
  const action = args.shift();
  if (action !== 'registry') {
    throw new Error(`unknown service dev command ${action || '(missing)'}\n${usage}`);
  }
  await runDevRegistryCommand(args, { defaultConfig: defaultWatchRegistryPath });
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
      '--base-config-snapshot',
      '--activation-url',
      '--ingress-url',
      '--profile',
      '--expected-generation',
      '--sources',
      '--shards',
      '--max-cases',
    ]),
    flags: new Set(['--live', '--deny-skips', '--require-tests', '--fresh', '--plan']),
  });
  const live = args.flags.has('--live');
  const liveTargetKeys = [
    'activationUrl', 'ingressUrl', 'profile', 'expectedGeneration',
  ];
  if (!live && liveTargetKeys.some((key) => args.options[key] !== undefined)) {
    throw new Error(
      'non-live skiff test owns activation, ingress, profile, and generation targets',
    );
  }
  if (args.options.artifactRoot === undefined) {
    throw new Error('skiff test requires --artifact-root');
  }
  if (
    (args.options.baseAssembly === undefined)
    !== (args.options.baseConfigSnapshot === undefined)
  ) {
    throw new Error(
      '--base-assembly and --base-config-snapshot must be provided together',
    );
  }
  const orchestrated = args.options.sources !== undefined || args.options.shards !== undefined;
  const planOnly = args.flags.has('--plan');
  const fresh = args.flags.has('--fresh');
  const sharded = args.options.shards !== undefined;
  if (fresh && args.options.sources === undefined) {
    throw new Error('--fresh requires --sources');
  }
  if (planOnly && args.options.sources === undefined) {
    throw new Error('--plan requires --sources');
  }
  if (live && orchestrated) {
    throw new Error('--sources, --fresh, --plan, and --shards cannot be combined with --live');
  }
  if (args.options.shards !== undefined && !/^[1-9][0-9]*$/.test(args.options.shards)) {
    throw new Error('--shards must be a positive integer');
  }
  if (args.options.maxCases !== undefined && !/^[1-9][0-9]*$/.test(args.options.maxCases)) {
    throw new Error('--max-cases must be a positive integer');
  }
  const explicitArtifactRoot = resolve(args.options.artifactRoot);
  if (orchestrated && !planOnly) {
    await mkdir(explicitArtifactRoot, { recursive: true });
  } else if (!orchestrated) {
    await requireExistingDirectory(explicitArtifactRoot, 'skiff test --artifact-root');
  }
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
    if (!/^[A-Za-z0-9._-]{1,200}$/.test(args.options.profile)) {
      throw new Error('--profile must be a canonical ASCII token');
    }
  }
  if (!sharded) {
    const kinds = await Promise.all(args.roots.map(detectRootKind));
    if (args.roots.length > 1 && kinds.some((kind) => kind.kind !== 'file')) {
      throw new Error('multiple skiff test roots must be explicit test files');
    }
    const kind = kinds[0];
    if (kind.kind !== 'package' && kind.kind !== 'file') {
      throw new Error(kind.message);
    }
  }

  let manifest;
  if (args.options.sources !== undefined) {
    manifest = await readSourceManifest(resolve(args.options.sources));
  }

  let shards;
  let testLabel;
  if (sharded) {
    const files = await discoverTestFiles(args.roots);
    if (files.length === 0) {
      throw new Error('no *.test.skiff test files found under the given roots');
    }
    const counts = await countTestCases(files);
    const totalCases = counts.reduce((total, count) => total + count, 0);
    shards = partitionTestFiles(files, counts, Number(args.options.shards));
    testLabel = `${args.roots.join('、')}：${files.length} 个测试文件 / ${totalCases} 个 case / ${shards.length} 个 shard`;
  } else {
    testLabel = `${args.roots.join('、')}：直接运行`;
  }

  let baseLabel;
  if (args.options.baseAssembly !== undefined) {
    baseLabel = `explicit：${args.options.baseAssembly} / ${args.options.baseConfigSnapshot}`;
  } else if (args.options.sources !== undefined) {
    const derived = deriveBaseServices(manifest, args.roots);
    baseLabel = `resolve from store：${derived.baseServices.map((entry) => entry.coordinate).join('、')}`;
  } else {
    baseLabel = 'none';
  }

  if (orchestrated) {
    const plan = await planSourcePublish({
      manifest,
      store: explicitArtifactRoot,
      fresh,
    });
    console.log(renderPlan({
      mode: plan.mode,
      store: explicitArtifactRoot,
      sourceEntries: plan.entries,
      testLabel,
      baseLabel,
    }));
    if (planOnly) {
      return;
    }
    if (args.options.sources !== undefined) {
      await publishSources({
        skiffRoot,
        store: explicitArtifactRoot,
        manifest,
        entries: plan.entries,
        env: cargoBuildEnv(skiffRoot),
        log: console.log,
      });
    }
  }

  let baseAssembly = args.options.baseAssembly;
  let baseConfigSnapshot = args.options.baseConfigSnapshot;
  if (baseAssembly === undefined && args.options.sources !== undefined) {
    const resolved = await resolveBasePair({
      skiffRoot,
      manifest,
      store: explicitArtifactRoot,
      testRoots: args.roots,
      env: cargoBuildEnv(skiffRoot),
    });
    baseAssembly = resolved.baseAssembly;
    baseConfigSnapshot = resolved.baseConfigSnapshot;
  }

  if (sharded) {
    const passed = await runShardedTests({
      skiffRoot,
      shards,
      store: explicitArtifactRoot,
      baseAssembly,
      baseConfigSnapshot,
      maxCases: args.options.maxCases === undefined
        ? undefined
        : Number(args.options.maxCases),
      env: cargoBuildEnv(skiffRoot),
      cwd: skiffRoot,
    });
    if (!passed) {
      throw new Error('sharded tests failed');
    }
    return;
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
    ...args.roots,
  ];
  if (!shouldUseIsolatedTestRuntime(live)) {
    testArgs.push('--live');
  }
  testArgs.push('--artifact-root', explicitArtifactRoot);
  testArgs.push('--platform-source-root', skiffRoot);
  if (baseAssembly !== undefined) {
    testArgs.push('--base-assembly', baseAssembly);
    testArgs.push('--base-config-snapshot', baseConfigSnapshot);
  }
  if (live) {
    testArgs.push(
      '--activation-url', args.options.activationUrl,
      '--ingress-url', args.options.ingressUrl,
      '--profile', args.options.profile,
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
  const isolatedProfile = baseConfigSnapshot === undefined
    ? undefined
    : await baseConfigSnapshotProfile(explicitArtifactRoot, baseConfigSnapshot);
  await runInIsolatedTestRuntime({
    skiffRoot,
    profile: isolatedProfile,
    runTest: (isolatedEnv, signal) => runOwnedCommand('cargo', testArgs, {
      cwd: skiffRoot,
      env: isolatedEnv,
      signal,
    }),
  });
}

async function baseConfigSnapshotProfile(artifactRoot, snapshotId) {
  const marker = 'skiff-runtime-config-snapshot-v1:';
  if (typeof snapshotId !== 'string' || !snapshotId.startsWith(marker)) {
    throw new Error(`--base-config-snapshot must be a ${marker} identity`);
  }
  const suffix = snapshotId.slice(marker.length);
  if (!/^[0-9a-f]{32}$/.test(suffix)) {
    throw new Error('--base-config-snapshot identity suffix must be 32 hex chars');
  }
  const path = join(artifactRoot, 'runtime-config', 'snapshots', `${suffix}.json`);
  let document;
  try {
    document = JSON.parse(await readFile(path, 'utf8'));
  } catch (error) {
    throw new Error(
      `cannot read base config snapshot ${snapshotId} at ${path}: ${error.code || error.message}`,
    );
  }
  const profile = document?.profile;
  if (typeof profile !== 'string' || profile.length === 0) {
    throw new Error(`base config snapshot ${snapshotId} has no profile`);
  }
  return profile;
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
  const httpMaxRequestBytes = readRequiredPositiveSafeInteger(
    args.options.httpMaxRequestBytes,
    '--http-max-request-bytes',
  );
  const httpMaxResponseBytes = readRequiredPositiveSafeInteger(
    args.options.httpMaxResponseBytes,
    '--http-max-response-bytes',
  );
  const activationPrepareTimeoutMs = readOptionalPositiveSafeInteger(
    args.options.activationPrepareTimeoutMs,
    '--activation-prepare-timeout-ms',
  ) ?? DEFAULT_ACTIVATION_PREPARE_TIMEOUT_MS;

  await mkdir(artifactRoot, { recursive: true });
  await mkdir(runtimeHome, { recursive: true });
  await mkdir(runtimePaths.runtimeBinDir, { recursive: true });

  const writes = [];
  writes.push(await writeDevInitFile(join(devHome, 'router.yml'), routerDevConfig({
    artifactRoot,
    serviceDbMongoUrl,
    httpMaxRequestBytes,
    httpMaxResponseBytes,
    activationPrepareTimeoutMs,
  }), force));
  writes.push(await writeDevInitFile(join(devHome, 'runtime.yml'), runtimeDevConfig({
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
  const roots = [];
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
    roots.push(resolve(arg));
  }
  if (roots.length === 0) {
    throw new Error('missing root path');
  }
  return { flags, options, roots };
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
      '--activation-prepare-timeout-ms',
      '--dev-home',
      '--http-max-request-bytes',
      '--http-max-response-bytes',
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
  const entryNames = new Set(entries.map((entry) => entry.name));
  const files = new Set(entries.filter((entry) => entry.isFile()).map((entry) => entry.name));
  const hasPackage = files.has('package.yml');
  const hasService = files.has('service.yml');
  const externalServiceControlFiles = ['http.yml', 'websocket.yml']
    .filter((file) => entryNames.has(file));
  if (externalServiceControlFiles.length > 0 && !hasService) {
    return {
      kind: 'missing',
      message: `${root} contains external service control file(s) ${externalServiceControlFiles.join(', ')}; external service control files require service.yml to declare the service role`,
    };
  }
  if (hasPackage) {
    return { kind: 'package' };
  }
  if (hasService) {
    return {
      kind: 'missing',
      message: `${root} contains service.yml but no package.yml; service.yml adds a service role to a Package and cannot define a source root`,
    };
  }
  return { kind: 'missing', message: `${root} must contain package.yml` };
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
    artifactsPath: options.artifactRoot,
    devReload: true,
    requestTimeoutMs: 20000,
    activationPrepareTimeoutMs: options.activationPrepareTimeoutMs,
    httpPort: 4000,
    httpMaxRequestBytes: options.httpMaxRequestBytes,
    httpMaxResponseBytes: options.httpMaxResponseBytes,
    runtimePort: 4001,
    runtimePath: '/runtime',
    serviceDbMongoUrl: options.serviceDbMongoUrl,
    telemetryEndpoint: 'ws://127.0.0.1:4002/telemetry',
  });
}

function readRequiredPositiveSafeInteger(value, option) {
  const integer = Number(value);
  if (value === undefined || !Number.isSafeInteger(integer) || integer <= 0) {
    throw new Error(`${option} must be a positive safe integer`);
  }
  return integer;
}

function readOptionalPositiveSafeInteger(value, option) {
  if (value === undefined) {
    return undefined;
  }
  return readRequiredPositiveSafeInteger(value, option);
}

function runtimeDevConfig(options) {
  return renderRuntimeConfig({
    routerUrl: 'ws://127.0.0.1:4001/runtime',
    runtimeHome: options.runtimeHome,
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

function run(command, args, cwd, options = {}) {
  return runAttachedCommand(command, args, { cwd, env: process.env, ...options });
}

function formatError(error) {
  const message = error?.message || String(error);
  const evidence = renderIsolatedRuntimeLogEvidence(error);
  return evidence.length === 0 ? message : `${message}\n${evidence}`;
}
