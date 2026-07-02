import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { identityCliBinaryName, runtimeBinaryName } from './dev-runtime-paths.mjs';
import { parseSimpleYamlObject } from './simple-yaml.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..', '..');

export const defaultInstancePorts = {
  routerHttp: 4100,
  routerControl: 4101,
  telemetry: 4102,
  mongo: 27017,
};

export function defaultInstanceConfigPath(repoRoot = skiffRoot) {
  return join(repoRoot, '.skiff-instance', 'config.yml');
}

export function defaultInstanceConfigText() {
  return [
    '# Local Skiff instance config.',
    '# Paths are resolved relative to this config file.',
    '# This file is separate from service/package skiff.yml configuration.',
    'devHome: dev-home',
    'cargoTargetDir: ~/.cache/skiff/cargo-target',
    '',
    'ports:',
    `  routerHttp: ${defaultInstancePorts.routerHttp}`,
    `  routerControl: ${defaultInstancePorts.routerControl}`,
    `  telemetry: ${defaultInstancePorts.telemetry}`,
    `  mongo: ${defaultInstancePorts.mongo}`,
    '',
    'components:',
    '  runtime: stable',
    '  identityCli: stable',
    '  router: worktree',
    '  telemetry: worktree',
    '  mongo: disabled',
    '  watch: disabled',
    '',
    'stable:',
    '  runtimeBinary: ~/.skiff/dev/bin/skiff-runtime',
    '  identityCli: ~/.skiff/dev/bin/skiff-artifact-identity',
    '',
    'telemetry:',
    '  memory: true',
    '',
    'mongo:',
    '  binary: mongod',
    '  dbPath: service-db',
    '',
    'watch:',
    '  config: watch.json',
    '',
  ].join('\n');
}

export async function readInstanceConfig({ configPath, repoRoot = skiffRoot }) {
  const paths = instanceBasePaths({ configPath, repoRoot });
  const raw = await readFile(paths.configPath, 'utf8');
  return normalizeInstanceConfig(parseSimpleYamlObject(raw, paths.configPath), {
    ...paths,
    repoRoot,
  });
}

export function defaultInstanceConfig({ configPath, repoRoot = skiffRoot }) {
  const paths = instanceBasePaths({ configPath, repoRoot });
  return normalizeInstanceConfig(parseSimpleYamlObject(defaultInstanceConfigText(), paths.configPath), {
    ...paths,
    repoRoot,
  });
}

export function instanceBasePaths({ configPath, repoRoot = skiffRoot }) {
  const resolvedConfigPath = resolve(configPath ?? defaultInstanceConfigPath(repoRoot));
  const instanceRoot = dirname(resolvedConfigPath);
  return {
    repoRoot: resolve(repoRoot),
    configPath: resolvedConfigPath,
    instanceRoot,
    pidDir: join(instanceRoot, 'pids'),
    logDir: join(instanceRoot, 'logs'),
    buildRoot: join(instanceRoot, 'build'),
  };
}

export function instanceSummary(config) {
  return {
    configPath: config.paths.configPath,
    instanceRoot: config.paths.instanceRoot,
    devHome: config.paths.devHome,
    artifactRoot: config.paths.artifactRoot,
    serviceBuildRoot: config.paths.serviceBuildRoot,
    runtimeConfig: config.paths.runtimeConfig,
    runtimeHome: config.paths.runtimeHome,
    binDir: config.paths.binDir,
    runtimeBinary: config.paths.runtimeBinary,
    identityCli: config.paths.identityCli,
    routerConfig: config.paths.routerConfig,
    telemetryConfig: config.paths.telemetryConfig,
    serviceDbPath: config.paths.serviceDbPath,
    watchConfig: config.paths.watchConfig,
    pidDir: config.paths.pidDir,
    logDir: config.paths.logDir,
    buildRoot: config.paths.buildRoot,
    cargoTargetDir: config.paths.cargoTargetDir,
    routerHttpUrl: config.urls.routerHttp,
    routerControlUrl: config.urls.routerControl,
    routerRuntimeUrl: config.urls.routerRuntime,
    routerReloadUrl: config.urls.routerReload,
    telemetryUrl: config.urls.telemetry,
    components: config.components,
  };
}

function normalizeInstanceConfig(raw, context) {
  const devHome = resolveConfigPath(
    context.instanceRoot,
    readString(raw.devHome, 'devHome', 'dev-home'),
  );
  const cargoTargetDir = resolveHome(readString(
    raw.cargoTargetDir,
    'cargoTargetDir',
    '~/.cache/skiff/cargo-target',
  ));
  const ports = normalizePorts(raw.ports);
  const components = normalizeComponents(raw.components);
  const stable = normalizeStable(raw.stable);
  const telemetry = normalizeTelemetry(raw.telemetry);
  const mongo = normalizeMongo(raw.mongo, devHome);
  const watch = normalizeWatch(raw.watch, devHome);
  const binDir = join(devHome, 'bin');
  const runtimeHome = join(devHome, 'runtime-home');

  return {
    schemaVersion: 'skiff-instance-v1',
    paths: {
      repoRoot: resolve(context.repoRoot),
      configPath: context.configPath,
      instanceRoot: context.instanceRoot,
      devHome,
      artifactRoot: join(devHome, 'artifacts'),
      serviceBuildRoot: join(devHome, 'build'),
      runtimeConfig: join(devHome, 'runtime.yml'),
      runtimeHome,
      binDir,
      runtimeBinary: join(binDir, runtimeBinaryName()),
      identityCli: join(binDir, identityCliBinaryName()),
      routerConfig: join(devHome, 'router.yml'),
      telemetryConfig: join(devHome, 'telemetry.yml'),
      serviceDbPath: mongo.dbPath,
      watchConfig: watch.config,
      pidDir: context.pidDir,
      logDir: context.logDir,
      buildRoot: context.buildRoot,
      cargoTargetDir,
    },
    ports,
    components,
    stable,
    telemetry,
    mongo,
    watch,
    urls: {
      routerHttp: `http://127.0.0.1:${ports.routerHttp}`,
      routerControl: `http://127.0.0.1:${ports.routerControl}`,
      routerRuntime: `ws://127.0.0.1:${ports.routerControl}/runtime`,
      routerReload: `http://127.0.0.1:${ports.routerControl}/__skiff/reload-artifacts`,
      telemetry: `ws://127.0.0.1:${ports.telemetry}/telemetry`,
    },
  };
}

function normalizePorts(value) {
  const ports = isRecord(value) ? value : {};
  return {
    routerHttp: readPort(ports.routerHttp, 'ports.routerHttp', defaultInstancePorts.routerHttp),
    routerControl: readPort(ports.routerControl, 'ports.routerControl', defaultInstancePorts.routerControl),
    telemetry: readPort(ports.telemetry, 'ports.telemetry', defaultInstancePorts.telemetry),
    mongo: readPort(ports.mongo, 'ports.mongo', defaultInstancePorts.mongo),
  };
}

function normalizeComponents(value) {
  const components = isRecord(value) ? value : {};
  const runtime = readEnum(components.runtime, 'components.runtime', ['stable', 'worktree'], 'stable');
  const identityCli = readEnum(components.identityCli, 'components.identityCli', ['stable', 'worktree'], 'stable');
  const router = readEnum(components.router, 'components.router', ['worktree'], 'worktree');
  const telemetry = readEnum(components.telemetry, 'components.telemetry', ['worktree', 'disabled'], 'worktree');
  const mongo = readEnum(components.mongo, 'components.mongo', ['managed', 'disabled'], 'disabled');
  const watch = readEnum(components.watch, 'components.watch', ['managed', 'disabled'], 'disabled');
  return { runtime, identityCli, router, telemetry, mongo, watch };
}

function normalizeStable(value) {
  const stable = isRecord(value) ? value : {};
  return {
    runtimeBinary: resolveHome(readString(
      stable.runtimeBinary,
      'stable.runtimeBinary',
      '~/.skiff/dev/bin/skiff-runtime',
    )),
    identityCli: resolveHome(readString(
      stable.identityCli,
      'stable.identityCli',
      '~/.skiff/dev/bin/skiff-artifact-identity',
    )),
  };
}

function normalizeTelemetry(value) {
  const telemetry = isRecord(value) ? value : {};
  return {
    memory: readBoolean(telemetry.memory, 'telemetry.memory', true),
  };
}

function normalizeMongo(value, devHome) {
  const mongo = isRecord(value) ? value : {};
  return {
    binary: readString(mongo.binary, 'mongo.binary', 'mongod'),
    dbPath: resolveConfigPath(devHome, readString(mongo.dbPath, 'mongo.dbPath', 'service-db')),
  };
}

function normalizeWatch(value, devHome) {
  const watch = isRecord(value) ? value : {};
  return {
    config: resolveConfigPath(devHome, readString(watch.config, 'watch.config', 'watch.json')),
  };
}

function readString(value, label, fallback) {
  if (value === undefined || value === null) {
    return fallback;
  }
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function readBoolean(value, label, fallback) {
  if (value === undefined || value === null) {
    return fallback;
  }
  if (typeof value !== 'boolean') {
    throw new Error(`${label} must be a boolean`);
  }
  return value;
}

function readPort(value, label, fallback) {
  if (value === undefined || value === null) {
    return fallback;
  }
  const port = typeof value === 'number' ? value : Number(value);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`${label} must be a TCP port`);
  }
  return port;
}

function readEnum(value, label, allowed, fallback) {
  const item = readString(value, label, fallback);
  if (!allowed.includes(item)) {
    throw new Error(`${label} must be one of ${allowed.join(', ')}`);
  }
  return item;
}

function resolveConfigPath(baseDir, value) {
  const expanded = resolveHome(value);
  return isAbsolute(expanded) ? expanded : resolve(baseDir, expanded);
}

function resolveHome(value) {
  if (value === '~') {
    return homedir();
  }
  if (value.startsWith('~/')) {
    return join(homedir(), value.slice(2));
  }
  return value;
}

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
