#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, readdir, readFile, realpath, rename, rm, stat, writeFile } from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, extname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoBuildEnv } from './lib/cargo-target-dir.mjs';
import { isPublicationId, publicationStorageSegment } from './lib/publication-id.mjs';
import { readProjectPackageDirs } from './lib/project-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');
const defaultDevHome = join(homedir(), '.skiff', 'dev');
const defaultWritableDevHome = resolveDevHome(process.env.SKIFF_DEV_HOME);
const defaultArtifactRoot = join(defaultWritableDevHome, 'artifacts');
const defaultBuildRoot = join(defaultWritableDevHome, 'build');
const defaultReloadUrl = 'http://127.0.0.1:4001/__skiff/reload-artifacts';
const defaultCompilerManifest = join(skiffRoot, 'compiler', 'Cargo.toml');
const defaultSharedInputs = [join(skiffRoot, 'prelude'), join(skiffRoot, 'std')];
const defaultPollIntervalMs = 500;
const lockTimeoutMs = 120_000;
const artifactRootContentDirs = new Set(['assemblies', 'bundles', 'contracts', 'files', 'units']);
const artifactPathKeys = new Set(['artifactPath', 'assemblyPath', 'bundlePath', 'fileIrPath', 'path', 'schemaPath', 'unitPath']);
const generatedArtifactRootEntries = new Set(['indexes', ...artifactRootContentDirs]);
const rootConfigSourcePattern = /^config(?:\.[A-Za-z_][A-Za-z0-9_-]*)?(?:\.secret)?\.yml$/;

const cli = parseCli(process.argv.slice(2));

if ([cli.watch, cli.check, cli.checkSync].filter(Boolean).length > 1) {
  throw new Error('--watch, --check and --check-sync cannot be used together');
}

const config = await loadConfig(cli);

if (cli.watch) {
  await watch(cli, config, cli.pollIntervalMs);
} else if (cli.check) {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-check-'));
  try {
    await buildAll(config, {
      syncShared: false,
      reloadRouter: false,
      targetDirForService: (service) => join(temp, publicationStorageSegment(service.serviceId)),
    });
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
} else if (cli.checkSync) {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-build-check-'));
  const artifactRoot = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-artifact-check-'));
  try {
    await seedSyncCheckRoot(artifactRoot, config.services);
    await buildAll(config, {
      syncShared: true,
      reloadRouter: false,
      syncRoot: artifactRoot,
      targetDirForService: (service) => join(temp, publicationStorageSegment(service.serviceId)),
    });
    const syncCheckConfig = {
      ...config,
      artifactRoot,
    };
    await seedMissingDevReloadPointer(artifactRoot, config.services);
    await assertBrokenConfiguredServiceOutput(syncCheckConfig, 'missing dev reload pointer');
    await buildAllUntilStable(syncCheckConfig, {
      syncShared: true,
      reloadRouter: false,
      targetDirForService: (service) => join(temp, publicationStorageSegment(service.serviceId)),
    });
    await seedMissingDevReloadPointerReference(artifactRoot, config.services);
    await assertBrokenConfiguredServiceOutput(syncCheckConfig, 'references missing service assembly');
    await buildAllUntilStable(syncCheckConfig, {
      syncShared: true,
      reloadRouter: false,
      targetDirForService: (service) => join(temp, publicationStorageSegment(service.serviceId)),
    });
    await assertSyncCheckRoot(artifactRoot, config.services);
  } finally {
    await rm(temp, { recursive: true, force: true });
    await rm(artifactRoot, { recursive: true, force: true });
  }
} else {
  await buildAll(config, { syncShared: true, reloadRouter: !cli.noReload });
}

async function watch(cli, initialConfig, pollIntervalMs) {
  let { config, fingerprint } = await buildWatchedConfigUntilStable(cli, initialConfig, {
    syncShared: true,
    reloadRouter: !cli.noReload,
  });
  let building = false;
  let pending = false;

  console.log(`[skiff-dev-sync] watching ${config.services.length} service(s)`);

  async function runWatchCycle() {
    if (building) {
      pending = true;
      return;
    }
    building = true;
    try {
      do {
        pending = false;
        const currentFingerprint = await inputFingerprint(config);
        if (currentFingerprint === fingerprint) {
          continue;
        }
        const previousConfig = config;
        const nextConfig = await loadConfig(cli);
        const stable = await buildWatchedConfigUntilStable(cli, nextConfig, {
          syncShared: true,
          reloadRouter: !cli.noReload,
          initialFingerprint: await inputFingerprint(nextConfig),
          pruneServiceIds: removedServiceIds(previousConfig.services, nextConfig.services),
        });
        config = stable.config;
        fingerprint = stable.fingerprint;
      } while (pending);
    } catch (error) {
      console.error(`[skiff-dev-sync] rebuild failed: ${formatError(error)}`);
    } finally {
      building = false;
    }
  }

  setInterval(() => {
    void runWatchCycle();
  }, pollIntervalMs);
}

async function buildWatchedConfigUntilStable(cli, initialConfig, options) {
  let config = initialConfig;
  let beforeBuild = options.initialFingerprint ?? await inputFingerprint(config);
  let pruneServiceIds = options.pruneServiceIds ?? [];
  while (true) {
    await buildAll(config, {
      ...options,
      pruneServiceIds,
    });
    const afterBuild = await inputFingerprint(config);
    if (afterBuild === beforeBuild) {
      return { config, fingerprint: afterBuild };
    }
    const previousConfig = config;
    config = await loadConfig(cli);
    pruneServiceIds = removedServiceIds(previousConfig.services, config.services);
    beforeBuild = await inputFingerprint(config);
    console.log('[skiff-dev-sync] inputs changed during build; rebuilding');
  }
}

async function buildAllUntilStable(config, options) {
  let beforeBuild = options.initialFingerprint ?? await inputFingerprint(config);
  while (true) {
    await buildAll(config, options);
    const afterBuild = await inputFingerprint(config);
    if (afterBuild === beforeBuild) {
      return afterBuild;
    }
    beforeBuild = afterBuild;
    console.log('[skiff-dev-sync] inputs changed during build; rebuilding');
  }
}

async function buildAll(config, options) {
  const syncRoot = options.syncRoot ?? config.artifactRoot;
  const results = [];
  for (const service of config.services) {
    const targetDir = options.targetDirForService?.(service) ?? defaultServiceBuildDir(service);
    results.push(await buildService(config, service, targetDir, {
      syncRoot,
      syncShared: options.syncShared,
    }));
  }
  if (options.syncShared && options.pruneServiceIds?.length > 0) {
    await removeDevReloadPointers(syncRoot, options.pruneServiceIds);
  }
  if (options.syncShared) {
    await assertConfiguredServiceOutputs(syncRoot, config.services);
  }
  if (options.syncShared && options.reloadRouter) {
    await reloadRouter(config.reloadUrl);
  }
  return results;
}

async function buildService(config, service, targetDir, options) {
  return withBuildLock(service, `${targetDir}.lock`, async () => {
    await rm(targetDir, { recursive: true, force: true });
    await mkdir(targetDir, { recursive: true });

    const serviceAssemblyPath = join(targetDir, 'service-assembly.json');
    const manifestPath = join(targetDir, 'router-manifest.json');
    const artifactRoot = join(targetDir, 'artifacts');

    await run('cargo', [
      ...compilerCargoPrefix(config),
      ...compilerBuildArgs(config, service, {
        artifactRoot,
        manifestPath,
        serviceAssemblyPath,
        serviceArtifactRoot: options.syncRoot,
      }),
    ], service.root, cargoBuildEnv(skiffRoot));

    const serviceAssembly = JSON.parse(await readFile(serviceAssemblyPath, 'utf8'));
    assertServiceAssembly(serviceAssembly, service.serviceId);
    await assertGeneratedArtifactRoot(artifactRoot);

    const assemblyHash = identityHash(serviceAssembly.service.assemblyIdentity);
    if (options.syncShared) {
      await syncArtifactRoot(service, artifactRoot, options.syncRoot);
      console.log(`[skiff-dev-sync] synced ${service.serviceId} ${assemblyHash.slice(0, 12)} to ${options.syncRoot}`);
    }
    console.log(`[skiff-dev-sync] compiled ${service.serviceId} ${assemblyHash.slice(0, 12)} to ${targetDir}`);

    return {
      artifactRoot,
      service,
      serviceAssembly,
    };
  });
}

function compilerCargoPrefix(config) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    config.compilerManifest,
    '--',
  ];
}

function compilerBuildArgs(config, service, paths) {
  const args = [
    service.root,
    '--out',
    paths.serviceAssemblyPath,
    '--manifest-out',
    paths.manifestPath,
    '--artifact-root',
    paths.artifactRoot,
    '--service-id',
    service.serviceId,
    '--profile',
    service.profile,
  ];
  if (paths.serviceArtifactRoot !== undefined) {
    args.push('--service-artifact-root', paths.serviceArtifactRoot);
  }
  for (const serviceArtifactRoot of config.serviceArtifactRoots) {
    args.push('--service-artifact-root', serviceArtifactRoot);
  }
  appendPackagesDirArgs(args, effectivePackageDirs(config, service));
  return args;
}

function effectivePackageDirs(config, service) {
  if (config.packageDirSource === 'cli') {
    return config.packageDirs;
  }
  if ((service.packageDirs?.length ?? 0) > 0) {
    return service.packageDirs;
  }
  if (config.packageDirSource === 'config') {
    return config.packageDirs;
  }
  return service.projectPackageDirs ?? config.packageDirs;
}

async function withBuildLock(service, lockDir, action) {
  await mkdir(dirname(lockDir), { recursive: true });
  const startedAt = Date.now();
  while (true) {
    try {
      await mkdir(lockDir);
      await writeFile(join(lockDir, 'owner.json'), JSON.stringify({
        pid: process.pid,
        serviceId: service.serviceId,
        startedAt: new Date().toISOString(),
      }, null, 2));
      break;
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
      if (Date.now() - startedAt > lockTimeoutMs) {
        throw new Error(`timed out waiting for ${lockDir}`);
      }
      await sleep(200);
    }
  }

  try {
    return await action();
  } finally {
    await rm(lockDir, { recursive: true, force: true });
  }
}

async function syncArtifactRoot(service, sourceRoot, targetRoot) {
  await assertGeneratedArtifactRoot(sourceRoot);
  await mkdir(targetRoot, { recursive: true });
  await copyContentAddressedArtifacts(sourceRoot, targetRoot);
  await copyMutableTreeIfPresent(join(sourceRoot, 'indexes'), join(targetRoot, 'indexes'));
  await syncServiceConfigSources(service, targetRoot);

  const [sourceIndex] = await serviceIndexFiles(sourceRoot, service.serviceId);
  const sourceIndexPath = sourceIndex.path;
  const indexBytes = await readFile(sourceIndexPath);
  const targetIndexPath = join(targetRoot, sourceIndex.artifactPath);
  await mkdir(dirname(targetIndexPath), { recursive: true });
  const tempIndexPath = await writeStagedFile(targetIndexPath, indexBytes);
  await rename(tempIndexPath, targetIndexPath);

  await removeStaleServiceIndexFiles(targetRoot, service.serviceId, new Set([sourceIndex.artifactPath]));
  await writeDevReloadPointer(targetRoot, service, devReloadPointerFromIndex(service, sourceIndex));
}

async function syncServiceConfigSources(service, targetRoot) {
  await removeRootConfigSourceCopies(targetRoot);
  for (const spec of defaultConfigSourceSpecs(service.profile)) {
    const sourcePath = join(service.root, spec.path);
    const targetPath = serviceConfigSourcePath(targetRoot, service.serviceId, spec.path);
    let sourceInfo;
    try {
      sourceInfo = await stat(sourcePath);
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
      await rm(targetPath, { force: true });
      continue;
    }
    if (!sourceInfo.isFile()) {
      await rm(targetPath, { force: true });
      continue;
    }
    const bytes = await readFile(sourcePath);
    await mkdir(dirname(targetPath), { recursive: true });
    const tempPath = await writeStagedFile(targetPath, bytes);
    await rename(tempPath, targetPath);
  }
}

function defaultConfigSourceSpecs(profile) {
  const specs = ['config.yml'];
  if (profile !== undefined && profile.length > 0) {
    assertConfigPathSegment(profile, `profile ${profile}`);
    specs.push(`config.${profile}.yml`);
    specs.push(`config.${profile}.secret.yml`);
  }
  return specs.map((path) => ({ path }));
}

function serviceConfigSourcePath(root, serviceId, configPath) {
  return join(root, 'configs', 'services', ...serviceIdPathSegments(serviceId), configPath);
}

function serviceIdPathSegments(serviceId) {
  if (!isPublicationId(serviceId)) {
    throw new Error(`service id ${serviceId} must be a publication id`);
  }
  return [publicationStorageSegment(serviceId)];
}

function defaultServiceBuildDir(service) {
  return join(defaultBuildRoot, publicationStorageSegment(service.serviceId));
}

function serviceIdJsonPath(root, prefixSegments, serviceId) {
  const segments = serviceIdPathSegments(serviceId);
  const fileName = segments.pop();
  return join(root, ...prefixSegments, ...segments, `${fileName}.json`);
}

function serviceIdDirectoryPath(root, prefixSegments, serviceId) {
  return join(root, ...prefixSegments, ...serviceIdPathSegments(serviceId));
}

function serviceScopedHashJsonPath(root, prefixSegments, serviceId, hash) {
  return join(serviceIdDirectoryPath(root, prefixSegments, serviceId), `${hash}.json`);
}

function toArtifactPath(path) {
  return path.split('\\').join('/');
}

async function removeRootConfigSourceCopies(targetRoot) {
  let entries;
  try {
    entries = await readdir(targetRoot, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  await Promise.all(entries.map(async (entry) => {
    if (!entry.isFile() || !rootConfigSourcePattern.test(entry.name)) {
      return;
    }
    await rm(join(targetRoot, entry.name), { force: true });
  }));
}

function assertConfigPathSegment(segment, label) {
  if (/^[A-Za-z_][A-Za-z0-9_-]*$/.test(segment) === false) {
    throw new Error(label);
  }
}

async function copyContentAddressedArtifacts(sourceRoot, targetRoot) {
  const entries = await readdir(sourceRoot, { withFileTypes: true });
  await Promise.all(entries
    .filter((entry) => entry.isDirectory() && artifactRootContentDirs.has(entry.name))
    .map((entry) => copyContentAddressedTree(
      join(sourceRoot, entry.name),
      join(targetRoot, entry.name),
    )));
}

async function copyContentAddressedTree(sourcePath, targetPath) {
  const info = await stat(sourcePath);
  if (info.isDirectory()) {
    await mkdir(targetPath, { recursive: true });
    const entries = await readdir(sourcePath, { withFileTypes: true });
    await Promise.all(entries.map((entry) => copyContentAddressedTree(
      join(sourcePath, entry.name),
      join(targetPath, entry.name),
    )));
    return;
  }
  if (!info.isFile()) {
    return;
  }
  await copyContentAddressedFile(sourcePath, targetPath);
}

async function copyMutableTreeIfPresent(sourcePath, targetPath) {
  let info;
  try {
    info = await stat(sourcePath);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  if (!info.isDirectory()) {
    return;
  }
  await copyMutableTree(sourcePath, targetPath);
}

async function copyMutableTree(sourcePath, targetPath) {
  const info = await stat(sourcePath);
  if (info.isDirectory()) {
    await mkdir(targetPath, { recursive: true });
    const entries = await readdir(sourcePath, { withFileTypes: true });
    await Promise.all(entries.map((entry) => copyMutableTree(
      join(sourcePath, entry.name),
      join(targetPath, entry.name),
    )));
    return;
  }
  if (!info.isFile()) {
    return;
  }
  const bytes = await readFile(sourcePath);
  await mkdir(dirname(targetPath), { recursive: true });
  const tempPath = await writeStagedFile(targetPath, bytes);
  await rename(tempPath, targetPath);
}

async function copyContentAddressedFile(sourcePath, targetPath) {
  const source = await readFile(sourcePath);
  try {
    const existing = await readFile(targetPath);
    if (Buffer.compare(source, existing) === 0) {
      return;
    }
    throw new Error(`content-addressed artifact conflict at ${targetPath}`);
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      throw error;
    }
  }

  await mkdir(dirname(targetPath), { recursive: true });
  const tempPath = `${targetPath}.${process.pid}.${Date.now()}.${randomUUID()}.tmp`;
  try {
    await writeFile(tempPath, source);
    await rename(tempPath, targetPath);
  } catch (error) {
    await rm(tempPath, { force: true });
    throw error;
  }
}

async function writeDevReloadPointer(root, service, pointer) {
  const pointerPath = serviceIdJsonPath(root, ['dev', 'services'], service.serviceId);
  await mkdir(dirname(pointerPath), { recursive: true });
  const tempPath = await writeStagedFile(
    pointerPath,
    Buffer.from(`${JSON.stringify(pointer, null, 2)}\n`),
  );
  await rename(tempPath, pointerPath);
}

async function removeDevReloadPointers(root, serviceIds) {
  for (const serviceId of serviceIds) {
    const pointerPath = serviceIdJsonPath(root, ['dev', 'services'], serviceId);
    await rm(pointerPath, { force: true });
    console.log(`[skiff-dev-sync] removed ${serviceId} dev reload pointer from ${root}`);
  }
}

async function readDevReloadPointer(root, service) {
  const pointerPath = serviceIdJsonPath(root, ['dev', 'services'], service.serviceId);
  try {
    return JSON.parse(await readFile(pointerPath, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`missing dev reload pointer for ${service.serviceId}`);
    }
    if (error instanceof SyntaxError) {
      throw new Error(`invalid dev reload pointer JSON for ${service.serviceId}`);
    }
    throw error;
  }
}

async function assertDevReloadPointerContract(root, service, pointer) {
  if (!isRecord(pointer)) {
    throw new Error(`dev reload pointer for ${service.serviceId} must be an object`);
  }
  if (pointer.serviceId !== service.serviceId) {
    throw new Error(`dev reload pointer serviceId mismatch for ${service.serviceId}`);
  }

  const assemblyPath = stringValue(pointer.serviceAssembly?.assemblyPath);
  if (assemblyPath === undefined) {
    throw new Error(`dev reload pointer for ${service.serviceId} is missing serviceAssembly.assemblyPath`);
  }
  if (!await isFile(join(root, assemblyPath))) {
    throw new Error(`dev reload pointer for ${service.serviceId} references missing service assembly ${assemblyPath}`);
  }

  if (pointer.serviceUnit !== undefined) {
    const unitPath = isRecord(pointer.serviceUnit)
      ? stringValue(pointer.serviceUnit.unitPath)
      : stringValue(pointer.serviceUnit);
    if (unitPath === undefined) {
      throw new Error(`dev reload pointer for ${service.serviceId} has invalid serviceUnit`);
    }
    if (!await isFile(join(root, unitPath))) {
      throw new Error(`dev reload pointer for ${service.serviceId} references missing service unit ${unitPath}`);
    }
  }
}

function removedServiceIds(previousServices, nextServices) {
  const nextServiceIds = new Set(nextServices.map((service) => service.serviceId));
  return [...new Set(previousServices
    .map((service) => service.serviceId)
    .filter((serviceId) => !nextServiceIds.has(serviceId)))]
    .sort((left, right) => left.localeCompare(right));
}

function devReloadPointerFromIndex(service, indexValue) {
  const serviceId = stringValue(indexValue?.serviceId);
  if (serviceId !== service.serviceId) {
    throw new Error(`synced index service id must be ${service.serviceId}`);
  }
  const protocolIdentity = stringValue(indexValue.contractIdentity);
  if (protocolIdentity === undefined) {
    throw new Error(`synced index for ${service.serviceId} is missing protocolIdentity`);
  }
  const contractHash = `sha256:${identityHash(protocolIdentity)}`;
  const serviceAssembly = serviceAssemblyPointer(indexValue.serviceAssembly);
  const serviceUnit = serviceUnitPointer(indexValue.serviceUnit);
  const buildId = serviceBuildIdFromAssemblyIdentity(serviceAssembly.assemblyIdentity);
  return {
    mode: 'dev',
    serviceId: service.serviceId,
    profile: service.profile,
    buildId,
    contractHash,
    protocolIdentity,
    serviceAssembly: {
      assemblyIdentity: serviceAssembly.assemblyIdentity,
      assemblyPath: serviceAssembly.assemblyPath,
    },
    ...(serviceUnit === undefined ? {} : { serviceUnit }),
  };
}

function serviceUnitPointer(value) {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('serviceUnit must be an object when present');
  }
  const unitPath = stringValue(value.unitPath)
    ?? stringValue(value.artifactPath)
    ?? stringValue(value.path)
    ?? stringValue(value.serviceUnitPath);
  if (unitPath === undefined) {
    throw new Error('serviceUnit requires unitPath/artifactPath/path');
  }
  return {
    ...(stringValue(value.schemaVersion) === undefined ? {} : { schemaVersion: stringValue(value.schemaVersion) }),
    ...(stringValue(value.unitIdentity) === undefined ? {} : { unitIdentity: stringValue(value.unitIdentity) }),
    ...(stringValue(value.unitHash) === undefined ? {} : { unitHash: stringValue(value.unitHash) }),
    unitPath,
  };
}

async function removeStaleServiceIndexFiles(targetRoot, serviceId, keepArtifactPaths) {
  const indexDir = join(targetRoot, 'indexes');
  const indexPaths = await listJsonFiles(indexDir).catch((error) => {
    if (error?.code === 'ENOENT') {
      return [];
    }
    throw error;
  });

  await Promise.all(indexPaths.map(async (indexPath) => {
    const artifactPath = toArtifactPath(relative(targetRoot, indexPath));
    if (keepArtifactPaths.has(artifactPath)) {
      return;
    }
    const value = JSON.parse(await readFile(indexPath, 'utf8'));
    if (stringValue(value?.serviceId) === serviceId) {
      await rm(indexPath, { force: true });
    }
  }));
}

async function reloadRouter(reloadUrl) {
  if (reloadUrl === undefined) {
    return;
  }
  try {
    const response = await postReload(reloadUrl);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      console.warn(`[skiff-dev-sync] warning: router reload returned HTTP ${response.statusCode}${response.body ? `: ${response.body}` : ''}`);
      return;
    }
    console.log(`[skiff-dev-sync] requested router reload at ${reloadUrl}`);
  } catch (error) {
    console.warn(`[skiff-dev-sync] warning: router reload unavailable at ${reloadUrl}: ${formatError(error)}`);
  }
}

function postReload(reloadUrl) {
  return new Promise((resolvePromise, reject) => {
    const url = new URL(reloadUrl);
    const transport = url.protocol === 'https:' ? https : http;
    const request = transport.request(url, {
      method: 'POST',
      headers: {
        'content-length': '0',
      },
    }, (response) => {
      let body = '';
      response.setEncoding('utf8');
      response.on('data', (chunk) => {
        body += chunk;
      });
      response.on('end', () => {
        resolvePromise({
          statusCode: response.statusCode ?? 0,
          body: body.slice(0, 500),
        });
      });
    });
    request.setTimeout(2_000, () => {
      request.destroy(new Error('router reload request timed out'));
    });
    request.on('error', reject);
    request.end();
  });
}

async function assertGeneratedArtifactRoot(root) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    if (!generatedArtifactRootEntries.has(entry.name) || !entry.isDirectory()) {
      throw new Error(`artifact root ${root} contains unsupported top-level entry ${entry.name}`);
    }
  }
}

async function serviceIndexFiles(root, serviceId) {
  const indexDir = join(root, 'indexes');
  const matches = [];
  for (const indexPath of await listJsonFiles(indexDir)) {
    const value = JSON.parse(await readFile(indexPath, 'utf8'));
    if (stringValue(value?.serviceId) === serviceId) {
      matches.push(parseCanonicalArtifactIndex(value, root, indexPath, serviceId));
    }
  }
  matches.sort((left, right) => left.path.localeCompare(right.path));
  if (matches.length !== 1) {
    throw new Error(`artifact root must contain exactly one index pointer for ${serviceId}; found ${matches.length}`);
  }
  return matches;
}

async function listJsonFiles(root) {
  const entries = await readdir(root, { withFileTypes: true });
  const paths = [];
  for (const entry of entries) {
    const entryPath = join(root, entry.name);
    if (entry.isDirectory()) {
      paths.push(...await listJsonFiles(entryPath));
    } else if (entry.isFile() && entry.name.endsWith('.json')) {
      paths.push(entryPath);
    }
  }
  return paths.sort((left, right) => left.localeCompare(right));
}

async function seedSyncCheckRoot(root, services) {
  await mkdir(join(root, 'indexes', 'services'), { recursive: true });
  await mkdir(join(root, 'assemblies', 'services'), { recursive: true });
  await mkdir(join(root, 'bundles'), { recursive: true });
  await mkdir(join(root, 'contracts'), { recursive: true });
  await mkdir(join(root, 'files'), { recursive: true });
  await mkdir(join(root, 'units'), { recursive: true });
  await mkdir(join(root, 'dev', 'services'), { recursive: true });

  const otherServiceId = 'skiff.run/retained';
  const otherServicePath = publicationStorageSegment(otherServiceId);
  const otherProtocolHash = '1'.repeat(64);
  const otherAssemblyHash = '2'.repeat(64);
  const otherProtocolIdentity = `skiff-protocol-v1:sha256:${otherProtocolHash}`;
  const otherAssemblyIdentity = `skiff-service-assembly-v1:sha256:${otherAssemblyHash}`;
  await writeJson(join(root, 'indexes', 'services', ...serviceIdPathSegments(otherServiceId), `${otherProtocolHash}.json`), {
    schemaVersion: 'skiff-artifact-index-v1',
    serviceId: otherServiceId,
    contractIdentity: otherProtocolIdentity,
    service: {
      id: otherServiceId,
      revisionId: '9'.repeat(64),
      protocolIdentity: otherProtocolIdentity,
    },
    serviceAssembly: {
      assemblyIdentity: otherAssemblyIdentity,
      assemblyPath: `assemblies/services/${otherServicePath}/${otherAssemblyHash}.json`,
    },
    files: [
      {
        artifactPath: 'files/other-retained.json',
      },
    ],
  });
  await writeJson(join(root, 'assemblies', 'services', ...serviceIdPathSegments(otherServiceId), `${otherAssemblyHash}.json`), {
    schemaVersion: 'skiff-assembly-v1',
    kind: 'service',
    service: {
      id: otherServiceId,
      assemblyIdentity: otherAssemblyIdentity,
      protocolIdentity: otherProtocolIdentity,
    },
  });
  await writeJson(join(root, 'contracts', `${otherProtocolHash}.json`), {
    schemaVersion: 'skiff-contract-schema-v1',
    serviceId: otherServiceId,
    protocolIdentity: otherProtocolIdentity,
  });
  await writeJson(join(root, 'files', 'other-retained.json'), {
    serviceId: otherServiceId,
    retained: true,
  });
  await writeJson(serviceIdJsonPath(root, ['dev', 'services'], otherServiceId), {
    mode: 'dev',
    serviceId: otherServiceId,
    profile: 'dev',
    buildId: `skiff-service-build-v1:sha256:${otherAssemblyHash}`,
    contractHash: `sha256:${otherProtocolHash}`,
    protocolIdentity: otherProtocolIdentity,
    serviceAssembly: {
      assemblyIdentity: otherAssemblyIdentity,
      assemblyPath: `assemblies/services/${otherServicePath}/${otherAssemblyHash}.json`,
    },
  });

  for (const service of services) {
    for (const spec of defaultConfigSourceSpecs(service.profile)) {
      await writeFile(join(root, spec.path), `stale ${service.serviceId} ${spec.path}\n`);
    }
    const staleIndexPath = serviceScopedHashJsonPath(root, ['indexes', 'services'], service.serviceId, 'stale');
    await writeJson(staleIndexPath, {
      serviceId: service.serviceId,
      stale: true,
    });
  }
}

async function seedMissingDevReloadPointer(root, services) {
  const [service] = services;
  if (service === undefined) {
    return;
  }
  await rm(serviceIdJsonPath(root, ['dev', 'services'], service.serviceId), { force: true });
}

async function seedMissingDevReloadPointerReference(root, services) {
  const [service] = services;
  if (service === undefined) {
    return;
  }
  const pointerPath = serviceIdJsonPath(root, ['dev', 'services'], service.serviceId);
  const pointer = JSON.parse(await readFile(pointerPath, 'utf8'));
  pointer.serviceAssembly = {
    ...pointer.serviceAssembly,
    assemblyPath: `assemblies/services/${publicationStorageSegment(service.serviceId)}/missing-dev-sync-check.json`,
  };
  await writeJson(pointerPath, pointer);
}

async function assertSyncCheckRoot(root, services) {
  for (const service of services) {
    await assertSyncedService(root, service);
    await assertMissing(serviceScopedHashJsonPath(root, ['indexes', 'services'], service.serviceId, 'stale'));
  }

  await assertExists(join(root, 'indexes', 'services', 'skiff~run~~retained', `${'1'.repeat(64)}.json`));
  await assertExists(join(root, 'assemblies', 'services', 'skiff~run~~retained', `${'2'.repeat(64)}.json`));
  await assertExists(join(root, 'contracts', `${'1'.repeat(64)}.json`));
  await assertExists(join(root, 'files', 'other-retained.json'));
  await assertExists(join(root, 'dev', 'services', 'skiff~run~~retained.json'));
}

async function assertConfiguredServiceOutputs(root, services) {
  for (const service of services) {
    const pointer = await readDevReloadPointer(root, service);
    await assertDevReloadPointerContract(root, service, pointer);
    await assertSyncedService(root, service, pointer);
  }
}

async function assertBrokenConfiguredServiceOutput(config, expectedMessage) {
  try {
    await assertConfiguredServiceOutputs(config.artifactRoot, config.services);
  } catch (error) {
    if (formatError(error).includes(expectedMessage)) {
      return;
    }
    throw error;
  }
  throw new Error(`expected configured service output contract check to fail with ${expectedMessage}`);
}

async function assertSyncedService(root, service, pointer = undefined) {
  const [index] = await serviceIndexFiles(root, service.serviceId);
  const indexPath = index.path;
  const pointerPath = serviceIdJsonPath(root, ['dev', 'services'], service.serviceId);
  pointer ??= await readDevReloadPointer(root, service);
  const expectedPointer = devReloadPointerFromIndex(service, index);
  assertDeepEqual(pointer, expectedPointer, `${pointerPath} dev reload pointer`);
  assertBuildId(pointer.buildId, `${pointerPath} buildId`);

  for (const artifactPath of artifactReferencePaths(index)) {
    await assertExists(join(root, artifactPath));
  }

  const contractSchemaPath = stringValue(index.contract?.schemaPath);
  if (contractSchemaPath !== undefined) {
    const contract = JSON.parse(await readFile(join(root, contractSchemaPath), 'utf8'));
    if (contract.schemaVersion !== 'skiff-contract-schema-v1') {
      throw new Error(`${contractSchemaPath} schemaVersion must be skiff-contract-schema-v1`);
    }
    if (contract.protocolIdentity !== expectedPointer.protocolIdentity) {
      throw new Error(`${contractSchemaPath} protocolIdentity must match dev reload pointer`);
    }
  }

  await assertExists(join(root, expectedPointer.serviceAssembly.assemblyPath));
  await assertSyncedConfigSources(root, service);
}

async function assertSyncedConfigSources(root, service) {
  for (const spec of defaultConfigSourceSpecs(service.profile)) {
    const sourcePath = join(service.root, spec.path);
    const targetPath = serviceConfigSourcePath(root, service.serviceId, spec.path);
    await assertMissing(join(root, spec.path));
    const sourceInfo = await stat(sourcePath).catch((error) => {
      if (error?.code === 'ENOENT') {
        return undefined;
      }
      throw error;
    });
    if (!sourceInfo?.isFile()) {
      await assertMissing(targetPath);
      continue;
    }
    await assertExists(targetPath);
    const sourceBytes = await readFile(sourcePath);
    const targetBytes = await readFile(targetPath);
    if (Buffer.compare(sourceBytes, targetBytes) !== 0) {
      throw new Error(`expected ${targetPath} to match service config source ${spec.path}`);
    }
  }
}

function parseCanonicalArtifactIndex(value, root, indexPath, expectedServiceId) {
  if (!isRecord(value)) {
    throw new Error(`${indexPath} artifact index must be an object`);
  }
  if (value.schemaVersion !== 'skiff-artifact-index-v1') {
    throw new Error(`${indexPath} schemaVersion must be skiff-artifact-index-v1`);
  }
  const serviceId = requiredString(value.serviceId, `${indexPath} serviceId`);
  if (serviceId !== expectedServiceId) {
    throw new Error(`${indexPath} serviceId must be ${expectedServiceId}`);
  }
  const contractIdentity = requiredString(value.contractIdentity, `${indexPath} contractIdentity`);
  const protocolHash = identityHash(contractIdentity);
  const artifactPath = toArtifactPath(relative(root, indexPath));
  const expectedArtifactPath = toArtifactPath(relative(root, serviceScopedHashJsonPath(root, ['indexes', 'services'], serviceId, protocolHash)));
  if (artifactPath !== expectedArtifactPath) {
    throw new Error(`${indexPath} must use canonical artifact index path ${expectedArtifactPath}`);
  }
  const serviceAssembly = serviceAssemblyPointer(
    value.serviceAssembly,
    `${indexPath} serviceAssembly`,
  );
  const index = {
    ...value,
    serviceAssembly,
  };
  Object.defineProperty(index, 'path', {
    enumerable: false,
    value: indexPath,
  });
  Object.defineProperty(index, 'artifactPath', {
    enumerable: false,
    value: artifactPath,
  });
  return index;
}

function artifactReferencePaths(value) {
  const result = new Set();
  collectArtifactReferencePaths(value, result, undefined);
  return [...result].sort();
}

function collectArtifactReferencePaths(value, result, key) {
  if (typeof value === 'string') {
    const normalized = value.replaceAll('\\', '/');
    if (artifactPathKeys.has(key) && normalized.endsWith('.json')) {
      result.add(normalized);
    }
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      collectArtifactReferencePaths(item, result, undefined);
    }
    return;
  }
  if (isRecord(value)) {
    for (const [nestedKey, item] of Object.entries(value)) {
      collectArtifactReferencePaths(item, result, nestedKey);
    }
  }
}

async function inputFingerprint(config) {
  const hash = createHash('sha256');
  const watchInputs = await discoverWatchInputs(config);
  for (const inputPath of watchInputs) {
    hash.update(inputPath);
    hash.update('\0');
    let info;
    try {
      info = await stat(inputPath);
    } catch (error) {
      if (error?.code === 'ENOENT') {
        hash.update('missing');
        hash.update('\0');
        continue;
      }
      throw error;
    }
    hash.update(String(info.mtimeMs));
    hash.update('\0');
    hash.update(String(info.size));
    hash.update('\0');
  }
  return hash.digest('hex');
}

async function discoverWatchInputs(config) {
  const inputs = config.configPath === undefined ? [] : [config.configPath];
  for (const service of config.services) {
    inputs.push(...await listFilesIfDirectory(service.root, isServiceWatchInput));
    for (const projectConfigPath of service.projectConfigPaths ?? []) {
      inputs.push(projectConfigPath);
    }
    for (const packageDir of effectivePackageDirs(config, service)) {
      inputs.push(...await listFilesIfDirectory(packageDir, isSharedWatchInput));
    }
  }
  for (const sharedInputRoot of config.sharedInputs) {
    inputs.push(...await listFiles(sharedInputRoot, isSharedWatchInput));
  }
  return [...new Set(inputs)].sort((left, right) => left.localeCompare(right));
}

async function listFilesIfDirectory(root, includeFile) {
  if (!await isDirectory(root)) {
    return [];
  }
  return listFiles(root, includeFile);
}

async function listFiles(root, includeFile) {
  const files = [];
  const visitedDirectories = new Set();

  async function visit(directory) {
    const directoryKey = await realpath(directory).catch(() => resolve(directory));
    if (visitedDirectories.has(directoryKey)) {
      return;
    }
    visitedDirectories.add(directoryKey);
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      const entryPath = join(directory, entry.name);
      let entryInfo;
      if (entry.isSymbolicLink()) {
        try {
          entryInfo = await stat(entryPath);
        } catch (error) {
          if (error?.code === 'ENOENT') {
            continue;
          }
          throw error;
        }
      }
      const isDirectory = entry.isDirectory() || entryInfo?.isDirectory();
      const isFile = entry.isFile() || entryInfo?.isFile();
      if (isDirectory) {
        if (shouldSkipDirectory(entry.name)) {
          continue;
        }
        await visit(entryPath);
      } else if (isFile && includeFile(entry.name, entryPath)) {
        files.push(entryPath);
      }
    }
  }

  await visit(root);
  return files;
}

function isServiceWatchInput(name) {
  return (name.startsWith('service') && name.endsWith('.yml'))
    || (name.startsWith('config') && name.endsWith('.yml'))
    || name.endsWith('.skiff');
}

function isSharedWatchInput(name) {
  return name.endsWith('.skiff')
    || name.endsWith('.yml')
    || name.endsWith('.yaml')
    || name.endsWith('.json');
}

function shouldSkipDirectory(name) {
  // Build output now lives under the dev home, not the service tree. `build`
  // and `build.lock` remain skipped so stale in-tree directories left by older
  // builds don't re-enter the watch fingerprint and trigger rebuild loops.
  return name === 'node_modules'
    || name === 'build'
    || name === 'build.lock'
    || name === '.skiff-build'
    || name === '.git'
    || name === 'target';
}

async function loadConfig(cli) {
  const configPath = resolveDevConfigPath(cli.config);
  const raw = configPath === undefined
    ? { __exists: false }
    : await readOptionalJsonConfig(configPath, !cli.watch);
  const configDir = configPath === undefined ? process.cwd() : dirname(configPath);
  const configLabel = configPath ?? 'dev config';
  const profile = cli.profile ?? optionalString(raw.profile, `${configLabel} profile`) ?? 'dev';
  const services = await readConfiguredServices({
    cli,
    configPath,
    configDir,
    configLabel,
    profile,
    raw,
  });
  const artifactRootValue =
    cli.artifactRoot ??
    process.env.SKIFF_ARTIFACT_ROOT ??
    optionalString(raw.artifactRoot, `${configLabel} artifactRoot`) ??
    defaultArtifactRoot;
  const artifactRoot = artifactRootValue === undefined
    ? undefined
    : resolveConfigPath(cli.artifactRoot !== undefined || process.env.SKIFF_ARTIFACT_ROOT !== undefined ? process.cwd() : configDir, artifactRootValue);
  const compilerManifestValue =
    cli.compilerManifest ??
    optionalString(raw.compilerManifest, `${configLabel} compilerManifest`) ??
    defaultCompilerManifest;
  const sharedInputs = [
    ...defaultSharedInputs,
    ...readSharedInputs(raw, configLabel, configDir),
  ];
  const cliPackageDirs = cli.packageDirs.map((path) => resolve(process.cwd(), path));
  const configPackageDirs = readPackageDirs(raw, configLabel, configDir);
  const packageDirSource = cliPackageDirs.length > 0
    ? 'cli'
    : configPackageDirs.length > 0
      ? 'config'
      : 'project';
  const packageDirs = uniquePaths(packageDirSource === 'cli'
    ? cliPackageDirs
    : packageDirSource === 'config'
      ? configPackageDirs
      : await defaultProjectPackageDirsForServices(services));
  const cliServiceArtifactRoots = cli.serviceArtifactRoots.map((path) => resolve(process.cwd(), path));
  const configServiceArtifactRoots = readServiceArtifactRoots(raw, configLabel, configDir);
  const serviceArtifactRoots = uniquePaths(cliServiceArtifactRoots.length > 0
    ? cliServiceArtifactRoots
    : configServiceArtifactRoots);
  return {
    artifactRoot,
    compilerManifest: resolveConfigPath(cli.compilerManifest !== undefined ? process.cwd() : configDir, compilerManifestValue),
    configPath,
    packageDirSource,
    packageDirs,
    reloadUrl: cli.reloadUrl ?? process.env.SKIFF_DEV_RELOAD_URL ?? optionalString(raw.reloadUrl, `${configLabel} reloadUrl`) ?? defaultReloadUrl,
    services,
    serviceArtifactRoots,
    sharedInputs,
  };
}

async function readConfiguredServices(input) {
  if (input.cli.root !== undefined || input.cli.serviceId !== undefined) {
    return [await readSingleCliService(input)];
  }

  if (input.configPath !== undefined && input.raw.__exists === false) {
    return [];
  }

  if (input.raw.services !== undefined) {
    return readConfigServices(input);
  }

  return [await readSingleCliService(input)];
}

async function readSingleCliService({ cli, profile }) {
  const root = resolve(process.cwd(), cli.root ?? '.');
  const serviceId = cli.serviceId ?? await readServiceId(root);
  assertServiceId(serviceId, `service id ${serviceId}`);
  return {
    serviceId,
    root,
    profile,
  };
}

async function readConfigServices({ raw, configDir, configLabel, profile }) {
  if (!Array.isArray(raw.services)) {
    throw new Error(`${configLabel} services must be an array`);
  }
  const services = [];
  const seenServiceIds = new Set();
  for (const [index, value] of raw.services.entries()) {
    const label = `${configLabel} services[${index}]`;
    if (!isRecord(value)) {
      throw new Error(`${label} must be an object`);
    }
    const rootValue = requiredString(value.root, `${label}.root`);
    const root = resolveConfigPath(configDir, rootValue);
    const serviceId = optionalString(value.serviceId, `${label}.serviceId`) ?? await readServiceId(root);
    assertServiceId(serviceId, `${label}.serviceId`);
    if (seenServiceIds.has(serviceId)) {
      throw new Error(`${configLabel} services contains duplicate serviceId ${serviceId}`);
    }
    seenServiceIds.add(serviceId);
    services.push({
      serviceId,
      root,
      profile: optionalString(value.profile, `${label}.profile`) ?? profile,
      packageDirs: value.packageDirs === undefined
        ? undefined
        : readPathList(value.packageDirs, `${label}.packageDirs`, configDir),
    });
  }
  services.sort((left, right) =>
    left.serviceId.localeCompare(right.serviceId)
    || left.root.localeCompare(right.root)
    || left.profile.localeCompare(right.profile));
  return services;
}

function assertServiceId(serviceId, label) {
  if (!isPublicationId(serviceId)) {
    throw new Error(`${label} must be a publication id`);
  }
}

async function readOptionalJsonConfig(path, required) {
  try {
    const value = JSON.parse(await readFile(path, 'utf8'));
    if (!isRecord(value)) {
      throw new Error(`${path} must be a JSON object`);
    }
    value.__exists = true;
    return value;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      if (required) {
        throw new Error(`failed to read dev config ${path}`);
      }
      return { __exists: false };
    }
    throw error;
  }
}

function readSharedInputs(raw, configPath, configDir) {
  return readPathList(raw.sharedInputs, `${configPath} sharedInputs`, configDir);
}

function readPackageDirs(raw, configPath, configDir) {
  return readPathList(raw.packageDirs, `${configPath} packageDirs`, configDir);
}

function readServiceArtifactRoots(raw, configPath, configDir) {
  return readPathList(raw.serviceArtifactRoots, `${configPath} serviceArtifactRoots`, configDir);
}

function readPathList(value, label, configDir) {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw new Error(`${label} must be an array`);
  }
  return value.map((item, index) =>
    resolveConfigPath(configDir, requiredString(item, `${label}[${index}]`)));
}

async function defaultProjectPackageDirsForServices(services) {
  if (services.length === 0) {
    return (await readProjectPackageDirs(process.cwd())).packageDirs;
  }
  const packageDirs = [];
  for (const service of services) {
    const project = await readProjectPackageDirs(service.root);
    service.projectConfigPaths = project.configPaths;
    service.projectPackageDirs = project.packageDirs;
    packageDirs.push(...project.packageDirs);
  }
  return packageDirs;
}

async function isDirectory(path) {
  try {
    return (await stat(path)).isDirectory();
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function uniquePaths(paths) {
  return [...new Set(paths.map((path) => resolve(path)))];
}

function appendPackagesDirArgs(targetArgs, packageDirs) {
  for (const packageDir of packageDirs) {
    targetArgs.push('--packages-dir', packageDir);
  }
}

async function readServiceId(root) {
  const serviceConfigPath = await serviceConfigPathFor(root);
  const source = await readFile(serviceConfigPath, 'utf8');
  const match = source.match(/^id:\s*([a-z0-9_./-]+)\s*$/m);
  if (!match) {
    throw new Error(`${serviceConfigPath} must declare top-level id`);
  }
  return match[1];
}

async function serviceConfigPathFor(root) {
  const defaultPath = join(root, 'service.yml');
  if (await isFile(defaultPath)) {
    return defaultPath;
  }
  throw new Error(`${root} must contain service.yml`);
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function resolveConfigPath(configDir, path) {
  return isAbsolute(path) ? path : resolve(configDir, path);
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

function resolveDevConfigPath(cliConfig) {
  const config = cliConfig ?? process.env.SKIFF_DEV_CONFIG ?? process.env.SKIFF_DEV_SYNC_CONFIG;
  return config === undefined ? undefined : resolve(process.cwd(), config);
}

function parseCli(args) {
  const result = {
    check: false,
    checkSync: false,
    config: undefined,
    artifactRoot: undefined,
    compilerManifest: undefined,
    noReload: false,
    packageDirs: [],
    profile: undefined,
    pollIntervalMs: defaultPollIntervalMs,
    reloadUrl: undefined,
    root: undefined,
    serviceArtifactRoots: [],
    serviceId: undefined,
    watch: false,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--watch') {
      result.watch = true;
    } else if (arg === '--check') {
      result.check = true;
    } else if (arg === '--check-sync') {
      result.checkSync = true;
    } else if (arg === '--no-reload') {
      result.noReload = true;
    } else if (arg === '--config') {
      result.config = requireNextArg(args, index, '--config');
      index += 1;
    } else if (arg.startsWith('--config=')) {
      result.config = arg.slice('--config='.length);
    } else if (arg === '--root') {
      result.root = requireNextArg(args, index, '--root');
      index += 1;
    } else if (arg.startsWith('--root=')) {
      result.root = arg.slice('--root='.length);
    } else if (arg === '--profile') {
      result.profile = requireNextArg(args, index, '--profile');
      index += 1;
    } else if (arg.startsWith('--profile=')) {
      result.profile = arg.slice('--profile='.length);
    } else if (arg === '--service' || arg === '--service-id') {
      result.serviceId = requireNextArg(args, index, arg);
      index += 1;
    } else if (arg.startsWith('--service=')) {
      result.serviceId = arg.slice('--service='.length);
    } else if (arg.startsWith('--service-id=')) {
      result.serviceId = arg.slice('--service-id='.length);
    } else if (arg === '--artifact-root') {
      result.artifactRoot = requireNextArg(args, index, '--artifact-root');
      index += 1;
    } else if (arg.startsWith('--artifact-root=')) {
      result.artifactRoot = arg.slice('--artifact-root='.length);
    } else if (arg === '--reload-url') {
      result.reloadUrl = requireNextArg(args, index, '--reload-url');
      index += 1;
    } else if (arg.startsWith('--reload-url=')) {
      result.reloadUrl = arg.slice('--reload-url='.length);
    } else if (arg === '--compiler-manifest') {
      result.compilerManifest = requireNextArg(args, index, '--compiler-manifest');
      index += 1;
    } else if (arg.startsWith('--compiler-manifest=')) {
      result.compilerManifest = arg.slice('--compiler-manifest='.length);
    } else if (arg === '--packages-dir') {
      result.packageDirs.push(requireNextArg(args, index, '--packages-dir'));
      index += 1;
    } else if (arg.startsWith('--packages-dir=')) {
      result.packageDirs.push(arg.slice('--packages-dir='.length));
    } else if (arg === '--service-artifact-root') {
      result.serviceArtifactRoots.push(requireNextArg(args, index, '--service-artifact-root'));
      index += 1;
    } else if (arg.startsWith('--service-artifact-root=')) {
      result.serviceArtifactRoots.push(arg.slice('--service-artifact-root='.length));
    } else if (arg === '--poll-interval-ms') {
      result.pollIntervalMs = parsePositiveInteger(requireNextArg(args, index, '--poll-interval-ms'), '--poll-interval-ms');
      index += 1;
    } else if (arg.startsWith('--poll-interval-ms=')) {
      result.pollIntervalMs = parsePositiveInteger(arg.slice('--poll-interval-ms='.length), '--poll-interval-ms');
    } else if (arg === '-h' || arg === '--help') {
      printUsage();
      process.exit(0);
    } else if (!arg.startsWith('-')) {
      if (result.root !== undefined) {
        throw new Error(`unexpected argument ${arg}`);
      }
      result.root = arg;
    } else {
      throw new Error(`unknown option ${arg}`);
    }
  }

  return result;
}

function requireNextArg(args, index, optionName) {
  const value = args[index + 1];
  if (value === undefined || value.startsWith('-')) {
    throw new Error(`${optionName} requires a value`);
  }
  return value;
}

function parsePositiveInteger(value, label) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${label} must be a positive integer`);
  }
  return parsed;
}

function printUsage() {
  console.log('usage: node skiff-dev-sync.mjs [root] [--watch|--check|--check-sync] [--root <service-dir>] [--profile <name>] [--artifact-root <dir>] [--service-artifact-root <dir>]... [--reload-url <url>] [--no-reload] [--config <path>] [--packages-dir <dir>]... [--poll-interval-ms <ms>]');
}

function run(command, runArgs, cwd, env = process.env) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, runArgs, {
      cwd,
      env,
      stdio: 'inherit',
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

async function writeStagedFile(targetPath, contents) {
  const tempPath = join(dirname(targetPath), `.${basename(targetPath)}.${process.pid}.${Date.now()}.${randomUUID()}.next`);
  try {
    await writeFile(tempPath, contents);
    return tempPath;
  } catch (error) {
    await rm(tempPath, { force: true });
    throw error;
  }
}

async function writeJson(path, value) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

async function assertExists(path) {
  try {
    await stat(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`expected ${path} to exist`);
    }
    throw error;
  }
}

async function assertMissing(path) {
  try {
    await stat(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  throw new Error(`expected ${path} to be removed`);
}

function assertServiceAssembly(value, serviceId) {
  if (value?.schemaVersion !== 'skiff-assembly-v1') {
    throw new Error('service assembly schemaVersion must be skiff-assembly-v1');
  }
  if (value.kind !== 'service') {
    throw new Error('service assembly kind must be service');
  }
  if (value.service?.id !== serviceId) {
    throw new Error(`service assembly service.id must be ${serviceId}`);
  }
  if (typeof value.service?.assemblyIdentity !== 'string') {
    throw new Error('service assembly is missing service.assemblyIdentity');
  }
  if (typeof value.service?.protocolIdentity !== 'string') {
    throw new Error('service assembly is missing service.protocolIdentity');
  }
}

function assertDeepEqual(actual, expected, label) {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`${label} mismatch\nexpected: ${expectedJson}\nactual: ${actualJson}`);
  }
}

function identityHash(identity) {
  const marker = ':sha256:';
  const index = identity.lastIndexOf(marker);
  if (index === -1) {
    throw new Error(`identity must include ${marker}`);
  }
  const hash = identity.slice(index + marker.length);
  if (hash.length === 0) {
    throw new Error('identity sha256 hash must not be empty');
  }
  return hash;
}

function serviceBuildIdFromAssemblyIdentity(assemblyIdentity) {
  const hash = identityHash(assemblyIdentity);
  assertSha256Hex(hash, 'service assembly identity sha256 hash');
  return `skiff-service-build-v1:sha256:${hash}`;
}

function assertBuildId(value, label) {
  if (typeof value !== 'string') {
    throw new Error(`${label} must be a string`);
  }
  const prefix = 'skiff-service-build-v1:sha256:';
  if (!value.startsWith(prefix)) {
    throw new Error(`${label} must start with ${prefix}`);
  }
  assertSha256Hex(value.slice(prefix.length), label);
}

function assertSha256Hex(value, label) {
  if (!/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`${label} must be 64 lowercase hex characters`);
  }
}

function serviceAssemblyPointer(value, label = 'serviceAssembly') {
  if (!isRecord(value)) {
    throw new Error(`${label} must be an object`);
  }
  const keys = Object.keys(value).sort();
  const expectedKeys = ['assemblyIdentity', 'assemblyPath'];
  if (keys.length !== expectedKeys.length || keys.some((key, index) => key !== expectedKeys[index])) {
    throw new Error(`${label} must contain only assemblyIdentity and assemblyPath`);
  }
  return {
    assemblyIdentity: requiredString(value.assemblyIdentity, `${label}.assemblyIdentity`),
    assemblyPath: requiredString(value.assemblyPath, `${label}.assemblyPath`),
  };
}

function requiredString(value, label) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function optionalString(value, label) {
  if (value === undefined) {
    return undefined;
  }
  return requiredString(value, label);
}

function stringValue(value) {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function sleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}
