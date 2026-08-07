import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

import { stringify } from 'yaml';

export const INSTANCE_SPEC_SCHEMA_VERSION = 'skiff-instance-v1';

export async function generateLocalInstanceSpec({ stack, skiffRoot }) {
  if (stack.build.profile !== 'debug') {
    throw new Error('instance.yml is only generated for debug builds');
  }
  const manifestPath = join(stack.paths.buildRoot, 'manifest.json');
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(`failed to read build manifest at ${manifestPath}: ${error.message}`);
  }
  if (manifest.profile !== 'debug') {
    throw new Error(`build manifest profile is ${manifest.profile}; expected debug`);
  }
  const spec = localInstanceSpecFrom({ stack, skiffRoot, manifest });
  await ensureDevHomeDirs(spec.devHome);
  for (const file of ['router.yml', 'runtime.yml', 'telemetry.yml']) {
    await copyFile(
      join(stack.configDir, file),
      join(stack.paths.buildRoot, file),
    );
  }
  await writeFile(join(stack.paths.buildRoot, 'instance.yml'), stringify(spec));
  return spec;
}

export function localInstanceSpecFrom({ stack, skiffRoot, manifest }) {
  const buildRoot = stack.paths.buildRoot;
  const process = stack.build.process;
  const devHome = resolve(
    stack.configDir,
    process.devHome ?? join(buildRoot, 'dev-home'),
  );
  const mongoDbPath = resolve(
    devHome,
    process.mongoDbPath ?? 'mongo-data',
  );
  const routerPorts = routerPortsFrom(stack.router);
  const mongoPort = mongoPortFrom(stack.router);
  const binary = (unitName, fallback) => {
    const artifact = manifest.units?.[unitName]?.artifacts?.find(
      (item) => item.kind === 'binary',
    );
    if (!artifact) {
      return join(buildRoot, 'bin', fallback);
    }
    return resolve(skiffRoot, artifact.path);
  };
  const processes = [];
  if (process.mongo === 'managed') {
    processes.push({
      name: 'mongo',
      command: process.mongoBinary,
      args: [
        '--dbpath', mongoDbPath,
        '--port', String(mongoPort),
        '--replSet', 'rs0',
        '--bind_ip', '127.0.0.1',
      ],
      cwd: buildRoot,
      ports: [mongoPort],
      healthUrl: null,
    });
  }
  if (process.watch === 'managed') {
    processes.push({
      name: 'watch',
      command: 'node',
      args: [
        join(skiffRoot, 'scripts', 'skiff-watch.mjs'),
        '--runtime', buildRoot,
        '--config', join(stack.configDir, 'watch'),
      ],
      cwd: skiffRoot,
      ports: [],
      healthUrl: null,
    });
  }
  processes.push({
    name: 'router',
    command: binary('router', 'skiff-router'),
    args: [join(buildRoot, 'router.yml')],
    cwd: skiffRoot,
    ports: [routerPorts.http, routerPorts.control],
    healthUrl: `http://127.0.0.1:${routerPorts.control}/__router/health`,
  });
  processes.push({
    name: 'runtime',
    command: binary('runtime', 'skiff-runtime'),
    args: [join(buildRoot, 'runtime.yml')],
    cwd: skiffRoot,
    ports: [],
    healthUrl: null,
  });

  const keyringFile = stack.runtime?.serviceDb?.encryption?.keyringFile ?? null;
  return {
    schemaVersion: INSTANCE_SPEC_SCHEMA_VERSION,
    profile: stack.config.profile,
    buildRoot,
    devHome,
    artifactRoot: join(devHome, 'artifacts'),
    runtimeHome: join(devHome, 'runtime-home'),
    secretsDir: join(devHome, 'secrets'),
    pidDir: join(devHome, 'pids'),
    logDir: join(devHome, 'logs'),
    mongoDbPath,
    compilerBinary: binary('compiler', 'skiff-compiler'),
    keyringFile,
    env: { RUST_LOG: 'info' },
    processes,
  };
}

function routerPortsFrom(router) {
  const http = router?.http?.port;
  const control = router?.runtime?.port;
  if (!Number.isSafeInteger(http) || !Number.isSafeInteger(control)) {
    throw new Error('router.yml must declare http.port and runtime.port');
  }
  return { http, control };
}

function mongoPortFrom(router) {
  const url = router?.serviceDb?.mongoUrl;
  if (typeof url !== 'string') {
    return 27017;
  }
  const match = url.match(/^mongodb:\/\/[^/]+:(\d+)/);
  if (!match) {
    return 27017;
  }
  const port = Number(match[1]);
  if (!Number.isSafeInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`router.yml serviceDb.mongoUrl has an invalid port: ${url}`);
  }
  return port;
}

async function ensureDevHomeDirs(devHome) {
  for (const name of ['artifacts', 'runtime-home', 'secrets', 'pids', 'logs', 'mongo-data']) {
    await mkdir(join(devHome, name), { recursive: true, mode: name === 'secrets' ? 0o700 : undefined });
  }
  await mkdir(dirname(devHome), { recursive: true });
}
