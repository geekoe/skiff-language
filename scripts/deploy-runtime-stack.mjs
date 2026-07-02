#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  renderRouterConfig,
  renderRuntimeConfig,
  renderTelemetryConfig,
} from './lib/runtime-stack-config.mjs';
import { sourceKeyFromInputs } from './lib/source-key.mjs';

const DEFAULT_REMOTE_HOME = '/root';
const DEFAULT_NODE_BIN = `${DEFAULT_REMOTE_HOME}/.local/share/fnm/node-versions/v22.22.1/installation/bin`;
const DEFAULT_TELEMETRY_MONGO_URL = 'mongodb://127.0.0.1:27017';
const DEFAULT_TELEMETRY_DB = 'skiff';
const DEFAULT_TELEMETRY_HOST = '127.0.0.1';
const DEFAULT_TELEMETRY_PORT = '4002';
const DEFAULT_TELEMETRY_PATH = '/telemetry';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const skiffRoot = path.resolve(scriptDir, '..');

const args = parseArgs(process.argv.slice(2));
const deploySelection = selectedDeployTargetsFrom(args.only || 'all');
const remote = args.remote || process.env.SKIFF_DEPLOY_REMOTE;
if (!remote) {
  throw new Error('deploy remote is required; pass --remote <user@host> or set SKIFF_DEPLOY_REMOTE');
}
const buildRoot = path.resolve(args.buildRoot || path.join(skiffRoot, 'build', 'runtime-stack'));
const buildManifestPath = path.resolve(args.buildManifest || path.join(buildRoot, 'manifest.json'));
const buildManifest = await readBuildManifest(buildManifestPath);

const remoteHome = args.remoteHome || process.env.SKIFF_DEPLOY_REMOTE_HOME || DEFAULT_REMOTE_HOME;
const remoteSkiff = args.remoteSkiff || process.env.SKIFF_DEPLOY_REMOTE_SKIFF || `${remoteHome}/skiff`;
const remoteNodeBin =
  args.nodeBin ||
  process.env.SKIFF_DEPLOY_NODE_BIN ||
  DEFAULT_NODE_BIN.replace(DEFAULT_REMOTE_HOME, remoteHome);
const telemetryHost =
  args.telemetryHost ||
  process.env.SKIFF_TELEMETRY_HOST ||
  DEFAULT_TELEMETRY_HOST;
const telemetryPort =
  args.telemetryPort ||
  process.env.SKIFF_TELEMETRY_PORT ||
  DEFAULT_TELEMETRY_PORT;
const telemetryPath = normalizePath(
  args.telemetryPath ||
    process.env.SKIFF_TELEMETRY_PATH ||
    DEFAULT_TELEMETRY_PATH,
);
const telemetryMemory =
  readOptionalBoolean(args.telemetryMemory, '--telemetry-memory') ??
  readOptionalBoolean(process.env.SKIFF_TELEMETRY_IN_MEMORY, 'SKIFF_TELEMETRY_IN_MEMORY') ??
  false;
const telemetryMongoUrl =
  args.telemetryMongoUrl ||
  process.env.SKIFF_TELEMETRY_MONGO_URL ||
  process.env.MONGO_URL ||
  DEFAULT_TELEMETRY_MONGO_URL;
const telemetryDb =
  args.telemetryDb ||
  process.env.SKIFF_TELEMETRY_DB ||
  DEFAULT_TELEMETRY_DB;
const telemetryTtlDays =
  args.telemetryTtlDays ||
  process.env.SKIFF_TELEMETRY_TTL_DAYS;
const telemetryEndpoint =
  args.telemetryEndpoint ||
  process.env.SKIFF_TELEMETRY_ENDPOINT ||
  `ws://127.0.0.1:${telemetryPort}${telemetryPath}`;
const serviceDbMongoUrl =
  args.serviceDbMongoUrl ||
  process.env.SKIFF_SERVICE_DB_MONGO_URL ||
  process.env.SERVICE_DB_MONGO_URL;
const remoteSsh = [remote];

await validateSelectedBuildUnits(deploySelection);

const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'skiff-runtime-stack-deploy-'));
try {
  const configDir = path.join(tempRoot, 'config');
  await writeRouterConfig(path.join(configDir, 'router.yml'), remoteSkiff, {
    telemetryEndpoint,
    serviceDbMongoUrl,
  });
  await writeRuntimeConfig(path.join(configDir, 'runtime.yml'), remoteSkiff);
  await writeTelemetryConfig(path.join(configDir, 'telemetry.yml'), {
    host: telemetryHost,
    port: telemetryPort,
    path: telemetryPath,
    memory: telemetryMemory,
    mongoUrl: telemetryMongoUrl,
    db: telemetryDb,
    ttlDays: telemetryTtlDays,
  });
  await writeEcosystem(path.join(tempRoot, 'ecosystem.config.cjs'), {
    remoteSkiff,
    remoteNodeBin,
  });

  await remoteExec(`mkdir -p ${remoteSkiff}/{artifacts,bin,config,logs,telemetry,router,scripts,runtime-home}`);

  if (deploySelection.has('router')) {
    await rsync(`${skiffRoot}/router/`, `${remote}:${remoteSkiff}/router/`, [
      '--exclude', 'node_modules',
      '--exclude', '.playwright-profile',
      '--exclude', '.browser-screenshot',
      '--exclude', 'router.yml',
    ]);
    await rsync(
      path.join(configDir, 'router.yml'),
      `${remote}:${remoteSkiff}/config/router.yml`,
      [],
      { delete: false },
    );
  }

  if (deploySelection.has('runtime')) {
    await rsync(
      path.join(configDir, 'runtime.yml'),
      `${remote}:${remoteSkiff}/config/runtime.yml`,
      [],
      { delete: false },
    );
    await uploadBinary(
      await binaryPathFor('runtime', args.runtimeBinary),
      `${remoteSkiff}/bin/skiff-runtime`,
    );
  }

  if (deploySelection.has('artifact-identity')) {
    await uploadBinary(
      await binaryPathFor('artifact-identity'),
      `${remoteSkiff}/bin/skiff-artifact-identity`,
    );
  }

  if (deploySelection.has('telemetry')) {
    await rsync(
      path.join(configDir, 'telemetry.yml'),
      `${remote}:${remoteSkiff}/config/telemetry.yml`,
      [],
      { delete: false },
    );
    await rsync(`${skiffRoot}/telemetry/`, `${remote}:${remoteSkiff}/telemetry/`, [
      '--exclude', 'node_modules',
      '--exclude', 'dist',
      '--exclude', 'telemetry.yml',
    ]);
  }

  if (deploySelection.size > 0) {
    await rsync(`${tempRoot}/ecosystem.config.cjs`, `${remote}:${remoteSkiff}/ecosystem.config.cjs`);
  }

  if (deploySelection.has('router')) {
    await remoteExec(`cd ${remoteSkiff}/router && PATH=${remoteNodeBin}:$PATH pnpm install --prod=false --ignore-scripts`);
  }
  if (deploySelection.has('telemetry')) {
    await remoteExec(`cd ${remoteSkiff}/telemetry && PATH=${remoteNodeBin}:$PATH pnpm install --prod=false --ignore-scripts`);
  }

  await reloadPm2Apps({ remoteSkiff, remoteNodeBin }, deploySelection);

  console.log(JSON.stringify({
    remote,
    only: args.only || 'all',
    buildManifest: buildManifestPath,
    buildCommit: buildManifest.commit,
    remoteSkiff,
    router: `${remoteSkiff}/router`,
    runtimeHome: `${remoteSkiff}/runtime-home`,
    config: `${remoteSkiff}/config`,
    binaries: `${remoteSkiff}/bin`,
    telemetry: {
      httpUrl: `http://${telemetryHost}:${telemetryPort}`,
      telemetryUrl: telemetryEndpoint,
      memory: telemetryMemory,
      database: telemetryDb,
      config: `${remoteSkiff}/config/telemetry.yml`,
    },
    serviceDb: {
      configured: serviceDbMongoUrl !== undefined,
      mongoUrl: serviceDbMongoUrl ?? null,
    },
    deployed: Object.fromEntries(
      [...deploySelection].map((unit) => [
        unit,
        buildManifest.units?.[unit]
          ? {
              commit: buildManifest.units[unit].commit,
              sourceKey: buildManifest.units[unit].sourceKey,
              artifacts: buildManifest.units[unit].artifacts,
            }
          : { legacyBinary: true },
      ]),
    ),
  }, null, 2));
} finally {
  await rm(tempRoot, { recursive: true, force: true });
}

async function validateSelectedBuildUnits(selection) {
  for (const unitName of selection) {
    if (unitName === 'router' || unitName === 'telemetry') {
      await assertCurrentVerifiedTsUnit(unitName);
      continue;
    }
    if (unitName === 'runtime' && args.runtimeBinary) {
      continue;
    }
    await assertBuiltUnit(unitName, 'rs');
  }
}

async function assertBuiltUnit(unitName, kind) {
  const unit = buildManifest.units?.[unitName];
  if (!unit) {
    throw new Error(`${unitName} is missing from ${buildManifestPath}; run build-runtime-stack.mjs --only ${unitName}`);
  }
  if (kind && unit.kind !== kind) {
    throw new Error(`${unitName} in ${buildManifestPath} has kind ${unit.kind}; expected ${kind}`);
  }
  for (const artifact of unit.artifacts || []) {
    if (!await isFile(resolveArtifactPath(artifact.path))) {
      throw new Error(`${unitName} build artifact is missing: ${artifact.path}`);
    }
  }
}

async function assertCurrentVerifiedTsUnit(unitName) {
  await assertBuiltUnit(unitName, 'ts');
  const unit = buildManifest.units[unitName];
  const current = await sourceKeyFromInputs({
    repoRoot: skiffRoot,
    component: unitName,
    inputs: unit.inputs?.map((input) => input.path) || [unitName],
  });
  if (unit.sourceKey !== current.sourceKey) {
    throw new Error(
      `${unitName} source differs from ${buildManifestPath}; run build-runtime-stack.mjs --only ${unitName} before deploying`,
    );
  }
}

async function binaryPathFor(unitName, override) {
  if (override) {
    const resolved = path.resolve(override);
    if (!await isFile(resolved)) {
      throw new Error(`${unitName} binary does not exist: ${override}`);
    }
    return resolved;
  }
  const unit = buildManifest.units?.[unitName];
  const artifact = unit?.artifacts?.find((item) => item.kind === 'binary');
  if (!artifact) {
    throw new Error(`${unitName} has no binary artifact in ${buildManifestPath}`);
  }
  const resolved = resolveArtifactPath(artifact.path);
  if (!await isFile(resolved)) {
    throw new Error(`${unitName} binary artifact does not exist: ${artifact.path}`);
  }
  return resolved;
}

function resolveArtifactPath(relativePath) {
  return path.resolve(skiffRoot, relativePath);
}

async function readBuildManifest(file) {
  let value;
  try {
    value = JSON.parse(await readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') {
      if (canDeployWithoutBuildManifest(deploySelection)) {
        return {
          schemaVersion: 'skiff-runtime-stack-build-v1',
          target: null,
          commit: null,
          units: {},
        };
      }
      throw new Error(`build manifest not found at ${file}; run scripts/build-runtime-stack.mjs first`);
    }
    throw error;
  }
  if (value.schemaVersion !== 'skiff-runtime-stack-build-v1') {
    throw new Error(`${file} is not a skiff runtime stack build manifest`);
  }
  return value;
}

function canDeployWithoutBuildManifest(selection) {
  for (const unitName of selection) {
    if (unitName === 'router' || unitName === 'telemetry') {
      return false;
    }
    if (unitName === 'runtime' && !args.runtimeBinary) {
      return false;
    }
  }
  return true;
}

async function writeRouterConfig(file, remoteSkiff, options) {
  await mkdirp(path.dirname(file));
  await writeFile(file, renderRouterConfig({
    profile: 'prod',
    host: '127.0.0.1',
    artifactRoots: [`${remoteSkiff}/artifacts`],
    identityCliPath: `${remoteSkiff}/bin/skiff-artifact-identity`,
    releaseMode: true,
    devReload: false,
    requestTimeoutMs: 20000,
    httpPort: 4000,
    runtimePort: 4001,
    runtimePath: '/runtime',
    telemetryEndpoint: options.telemetryEndpoint,
    serviceDbMongoUrl: options.serviceDbMongoUrl,
  }));
}

async function writeRuntimeConfig(file, remoteSkiff) {
  await mkdirp(path.dirname(file));
  await writeFile(file, renderRuntimeConfig({
    routerUrl: 'ws://127.0.0.1:4001/runtime',
    runtimeHome: `${remoteSkiff}/runtime-home`,
    httpResponseMaxBytes: 8388608,
  }));
}

async function writeTelemetryConfig(file, options) {
  await mkdirp(path.dirname(file));
  await writeFile(file, renderTelemetryConfig({
    host: options.host,
    port: options.port,
    path: options.path,
    memory: options.memory,
    emitMemory: options.memory,
    mongo: options.memory
      ? undefined
      : {
          url: options.mongoUrl,
          database: options.db,
          ttlDays: options.ttlDays,
        },
  }));
}

async function writeEcosystem(file, options) {
  await writeFile(file, `const NODE_BIN = '${options.remoteNodeBin}';

module.exports = {
  apps: [
    {
      name: 'skiff-router',
      cwd: '${options.remoteSkiff}/router',
      script: 'src/router/server.ts',
      interpreter: NODE_BIN + '/node',
      interpreter_args: '--import tsx',
      args: '--config ${options.remoteSkiff}/config/router.yml --release-mode',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        NODE_ENV: 'production',
      },
    },
    {
      name: 'skiff-telemetry',
      cwd: '${options.remoteSkiff}/telemetry',
      script: 'src/main.ts',
      interpreter: NODE_BIN + '/node',
      interpreter_args: '--import tsx',
      args: '--config ${options.remoteSkiff}/config/telemetry.yml',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        NODE_ENV: 'production',
      },
    },
    {
      name: 'skiff-runtime',
      cwd: '${options.remoteSkiff}',
      script: '${options.remoteSkiff}/bin/skiff-runtime',
      args: '${options.remoteSkiff}/config/runtime.yml',
      interpreter: 'none',
      watch: false,
      autorestart: true,
      max_restarts: 5,
      restart_delay: 2000,
      env: {
        RUST_LOG: 'info',
      },
    },
  ],
};
`);
}

async function uploadBinary(source, remotePath) {
  await rsync(source, `${remote}:${remotePath}`, [], { delete: false });
  await remoteExec(`chmod +x ${remotePath}`);
}

async function reloadPm2Apps(options, selectedApps) {
  if (selectedApps.size === 0) {
    return;
  }
  const apps = orderedPm2Apps(selectedApps);
  if (apps.length === 0) {
    return;
  }
  const pm2 = `PATH=${options.remoteNodeBin}:$PATH pm2`;
  await deleteLegacyPm2Apps(pm2);
  for (const app of apps) {
    await remoteExec([
      `cd ${options.remoteSkiff} && ${pm2} startOrReload ecosystem.config.cjs`,
      `--only ${pm2AppName(app)} --update-env`,
    ].join(' '));
  }
  await remoteExec(`${pm2} save`);
}

function orderedPm2Apps(selectedApps) {
  const order = ['telemetry', 'router', 'runtime'];
  return order.filter((app) => selectedApps.has(app));
}

async function deleteLegacyPm2Apps(pm2) {
  const legacyPrefix = ['bai', 'ma'].join('');
  const legacyApps = [
    `${legacyPrefix}-router`,
    `${legacyPrefix}-telemetry`,
    `${legacyPrefix}-runtime`,
  ];
  for (const legacyApp of legacyApps) {
    await remoteExec(`${pm2} delete ${legacyApp} || true`);
  }
}

async function mkdirp(dir) {
  const { mkdir } = await import('node:fs/promises');
  await mkdir(dir, { recursive: true });
}

function remoteExec(command) {
  return run('ssh', [...remoteSsh, command], skiffRoot);
}

function rsync(source, destination, extra = [], options = { delete: true }) {
  return run(
    'rsync',
    ['-az', ...(options.delete ? ['--delete'] : []), ...extra, source, destination],
    skiffRoot,
  );
}

function run(command, commandArgs, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
      cwd,
      env: process.env,
      stdio: 'inherit',
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} ${commandArgs.join(' ')} failed with ${signal || code}`));
    });
  });
}

async function isFile(file) {
  try {
    return (await stat(file)).isFile();
  } catch (error) {
    if (error.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function selectedDeployTargetsFrom(rawOnly) {
  const values = rawOnly.split(',').map((value) => value.trim()).filter(Boolean);
  const selected = new Set();
  for (const value of values) {
    for (const app of expandDeploySelector(value)) {
      selected.add(app);
    }
  }
  return selected;
}

function expandDeploySelector(rawOnly) {
  switch (rawOnly) {
    case 'all':
      return ['telemetry', 'router', 'runtime', 'artifact-identity'];
    case 'runtime':
      return ['runtime', 'artifact-identity'];
    case 'router':
      return ['router', 'artifact-identity'];
    case 'artifact-identity':
    case 'telemetry':
      return [rawOnly];
    default:
      throw new Error(
        `invalid --only ${rawOnly}; deploy supports all, runtime, router, artifact-identity, or telemetry. compiler is a build-only unit.`,
      );
  }
}

function pm2AppName(component) {
  switch (component) {
    case 'router':
      return 'skiff-router';
    case 'telemetry':
      return 'skiff-telemetry';
    case 'runtime':
      return 'skiff-runtime';
    default:
      throw new Error(`unknown component ${component}`);
  }
}

function normalizePath(value) {
  return value.startsWith('/') ? value : `/${value}`;
}

function parseArgs(rawArgs) {
  const result = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    const key = optionKey(arg);
    if (!key) {
      throw new Error(`unknown argument ${arg}`);
    }
    const value = rawArgs[index + 1];
    if (!value || value.startsWith('--')) {
      throw new Error(`${arg} requires a value`);
    }
    result[key] = value;
    index += 1;
  }
  return result;
}

function optionKey(arg) {
  switch (arg) {
    case '--remote':
      return 'remote';
    case '--only':
      return 'only';
    case '--remote-home':
      return 'remoteHome';
    case '--remote-skiff':
      return 'remoteSkiff';
    case '--node-bin':
      return 'nodeBin';
    case '--telemetry-mongo-url':
      return 'telemetryMongoUrl';
    case '--service-db-mongo-url':
      return 'serviceDbMongoUrl';
    case '--telemetry-db':
      return 'telemetryDb';
    case '--telemetry-host':
      return 'telemetryHost';
    case '--telemetry-port':
      return 'telemetryPort';
    case '--telemetry-path':
      return 'telemetryPath';
    case '--telemetry-ttl-days':
      return 'telemetryTtlDays';
    case '--telemetry-endpoint':
      return 'telemetryEndpoint';
    case '--telemetry-memory':
      return 'telemetryMemory';
    case '--runtime-binary':
      return 'runtimeBinary';
    case '--build-root':
      return 'buildRoot';
    case '--build-manifest':
      return 'buildManifest';
    default:
      return null;
  }
}

function readOptionalBoolean(value, name) {
  if (value === undefined || value === null) {
    return undefined;
  }
  const normalized = String(value).trim().toLowerCase();
  if (normalized === 'true' || normalized === '1') {
    return true;
  }
  if (normalized === 'false' || normalized === '0') {
    return false;
  }
  throw new Error(`${name} must be true or false`);
}
