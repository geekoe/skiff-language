#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { access, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { publicationStorageSegment } from './lib/publication-id.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffCli = join(scriptDir, 'skiff.mjs');
const devSyncCli = join(scriptDir, 'skiff-dev-sync.mjs');
const rustEnv = {
  CARGO_HOME: process.env.CARGO_HOME ?? join(homedir(), '.cargo'),
  RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(homedir(), '.rustup'),
};

const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-package-store-discovery-'));

try {
  await checkExplicitPackagesDir();
  await checkProjectPackageDirs();
  await checkLocalProjectPackageDirsOverride();
  await checkProjectPackageDirsFallback();
  await checkProjectPackageDirsShadowLowerPriority();
  await checkNoImplicitDevHomePackageDir();
  await checkExplicitPackagesDirOverridesLocalProject();
  await checkPackagesDirSymlink();
  await checkPackagesDirIsNotRecursive();
  await checkPackagePullDefaultTargetUsesProjectPackageDir();
  await checkPackagePullFailsWithoutProjectPackageDir();
  await checkDevWatchConfigUsesProjectPackageDirs();
  console.log('Package store discovery check passed.');
} finally {
  await rm(tempRoot, { force: true, recursive: true });
}

async function checkExplicitPackagesDir() {
  await writePackageWithDependency(join(tempRoot, 'explicit', 'pkg'));
  await writeCloudDependency(packageStorePath(join(tempRoot, 'explicit-store'), 'google.com/cloud', '1.0.0'));
  await runSkiff(['test', 'explicit/pkg', '--packages-dir', 'explicit-store']);
}

async function checkProjectPackageDirs() {
  const project = join(tempRoot, 'project-config');
  await writeProjectConfig(project, ['.skiff-package-store']);
  await writePackageWithDependency(join(project, 'pkg'));
  await writeCloudDependency(packageStorePath(join(project, '.skiff-package-store'), 'google.com/cloud', '1.0.0'));
  await runSkiff(['test', 'project-config/pkg']);
}

async function checkLocalProjectPackageDirsOverride() {
  const project = join(tempRoot, 'project-local-config');
  await writeProjectConfig(project, ['base-store']);
  await writeLocalProjectConfig(project, ['local-store']);
  await writePackageWithDependency(join(project, 'pkg'));
  await writeCloudDependency(packageStorePath(join(project, 'base-store'), 'google.com/cloud', '1.0.0'), 'bad');
  await writeCloudDependency(packageStorePath(join(project, 'local-store'), 'google.com/cloud', '1.0.0'), 'ok');
  await runSkiff(['test', 'project-local-config/pkg']);
}

async function checkProjectPackageDirsFallback() {
  const project = join(tempRoot, 'project-fallback');
  await writeProjectConfig(project, ['empty-store', 'fallback-store']);
  await writePackageWithDependency(join(project, 'pkg'));
  await writeCloudDependency(packageStorePath(join(project, 'fallback-store'), 'google.com/cloud', '1.0.0'));
  await runSkiff(['test', 'project-fallback/pkg']);
}

async function checkProjectPackageDirsShadowLowerPriority() {
  const project = join(tempRoot, 'project-shadow');
  await writeProjectConfig(project, ['override-store', 'base-store']);
  await writePackageWithDependency(join(project, 'pkg'));
  await writeCloudDependency(packageStorePath(join(project, 'base-store'), 'google.com/cloud', '1.0.0'), 'bad');
  await writeCloudDependency(packageStorePath(join(project, 'override-store'), 'google.com/cloud', '1.0.0'), 'ok');
  await runSkiff(['test', 'project-shadow/pkg']);
}

async function checkNoImplicitDevHomePackageDir() {
  const home = join(tempRoot, 'home-ignored');
  await writePackageWithDependency(join(tempRoot, 'no-project', 'pkg'));
  await writeCloudDependency(packageStorePath(join(home, '.skiff', 'dev', 'packages'), 'google.com/cloud', '1.0.0'));
  await runSkiffExpectFailure(['test', 'no-project/pkg'], { HOME: home, USERPROFILE: home });
}

async function checkExplicitPackagesDirOverridesLocalProject() {
  const project = join(tempRoot, 'override');
  await writeProjectConfig(project, ['.skiff-package-store']);
  await writeLocalProjectConfig(project, ['local-store']);
  await writePackageWithDependency(join(project, 'pkg'));
  await writeCloudDependency(packageStorePath(join(project, '.skiff-package-store'), 'google.com/cloud', '1.0.0'), 'bad');
  await writeCloudDependency(packageStorePath(join(project, 'local-store'), 'google.com/cloud', '1.0.0'), 'bad');
  await writeCloudDependency(packageStorePath(join(tempRoot, 'override-store'), 'google.com/cloud', '1.0.0'), 'ok');
  await runSkiff(['test', 'override/pkg', '--packages-dir', 'override-store']);
}

async function checkPackagesDirSymlink() {
  await writePackageWithDependency(join(tempRoot, 'symlink', 'pkg'));
  const target = join(tempRoot, 'symlink-target');
  await writeCloudDependency(target);
  const link = packageStorePath(join(tempRoot, 'symlink-store'), 'google.com/cloud', '1.0.0');
  await mkdir(dirname(link), { recursive: true });
  await symlink(target, link, 'dir');
  await runSkiff(['test', 'symlink/pkg', '--packages-dir', 'symlink-store']);
}

async function checkPackagesDirIsNotRecursive() {
  await writePackageWithDependency(join(tempRoot, 'nested', 'pkg'));
  await writeCloudDependency(join(tempRoot, 'nested-store', 'nested', publicationStorageSegment('google.com/cloud'), '1.0.0'));
  await runSkiffExpectFailure(['test', 'nested/pkg', '--packages-dir', 'nested-store']);
}

async function checkPackagePullDefaultTargetUsesProjectPackageDir() {
  const project = join(tempRoot, 'pull-project');
  await writeProjectConfig(project, ['base-store']);
  await writeLocalProjectConfig(project, ['.skiff-package-store']);
  await withPackageRemote(async (extraEnv) => {
    await runSkiffCommand(['package', 'pull', 'skiff.run/llm@1.0.0'], {
      cwd: project,
      extraEnv,
    });
  });
  await assertFileExists(join(
    project,
    '.skiff-package-store',
    publicationStorageSegment('skiff.run/llm'),
    '1.0.0',
    'package.yml',
  ));
}

async function checkPackagePullFailsWithoutProjectPackageDir() {
  const project = join(tempRoot, 'pull-no-project');
  await mkdir(project, { recursive: true });
  await withPackageRemote(async (extraEnv) => {
    await runSkiffCommandExpectFailure(['package', 'pull', 'skiff.run/llm@1.0.0'], {
      cwd: project,
      extraEnv,
      expectedOutput: 'skiff package pull without --out requires a skiff.yml or skiff.local.yml',
    });
  });
}

async function checkDevWatchConfigUsesProjectPackageDirs() {
  const project = join(tempRoot, 'watch-project');
  const service = join(project, 'service');
  const registryPath = join(tempRoot, 'watch.json');
  await writeProjectConfig(project, ['base-store']);
  await writeLocalProjectConfig(project, ['.skiff-package-store', '../shared-store']);
  await writeServiceWithDependency(service, 'example.com/watch');
  await writeCloudDependency(packageStorePath(join(project, '.skiff-package-store'), 'google.com/cloud', '1.0.0'));
  await writeCloudDependency(packageStorePath(join(project, 'base-store'), 'google.com/cloud', '1.0.0'), 'bad');

  await runSkiffCommand(['service', 'dev', 'registry', 'add', 'watch-project/service', '--config', registryPath]);
  const registry = JSON.parse(await readFile(registryPath, 'utf8'));
  if (Object.hasOwn(registry.services?.[0] ?? {}, 'packageDirs')) {
    throw new Error(`watch registry should not snapshot packageDirs: ${JSON.stringify(registry.services[0].packageDirs)}`);
  }
  await runCommand(process.execPath, [devSyncCli, '--check', '--config', registryPath], {
    extraEnv: {
      SKIFF_DEV_HOME: join(tempRoot, 'dev-sync-home'),
    },
  }).then(({ code, signal, stdout, stderr }) => {
    if (code === 0) {
      return;
    }
    throw new Error([
      `skiff-dev-sync --check --config failed with ${signal ?? code}`,
      stderr.trim(),
      stdout.trim(),
    ].filter(Boolean).join('\n'));
  });
}

function packageStorePath(root, packageId, version) {
  return join(root, publicationStorageSegment(packageId), version);
}

async function writeProjectConfig(projectRoot, packageDirs) {
  await writePackageDirsConfig(projectRoot, 'skiff.yml', packageDirs);
}

async function writeLocalProjectConfig(projectRoot, packageDirs) {
  await writePackageDirsConfig(projectRoot, 'skiff.local.yml', packageDirs);
}

async function writePackageDirsConfig(projectRoot, fileName, packageDirs) {
  await mkdir(projectRoot, { recursive: true });
  await writeFile(
    join(projectRoot, fileName),
    [
      'packageDirs:',
      ...packageDirs.map((packageDir) => `  - ${packageDir}`),
      '',
    ].join('\n'),
  );
}

async function writePackageWithDependency(packageRoot) {
  await mkdir(packageRoot, { recursive: true });
  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/facade',
      'version: 1.0.0',
      'packages:',
      '  - id: google.com/cloud',
      '    version: 1.0.0',
      '    alias: gcloud',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(packageRoot, 'api.yml'),
    [
      'facade:',
      '  facade: facade.facade',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(packageRoot, 'facade.skiff'),
    [
      'import gcloud',
      '',
      'function facade() -> string {',
      '  return gcloud.storage.upload()',
      '}',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(packageRoot, 'facade.test.skiff'),
    [
      'test "package dependency alias call works" {',
      '  assert root.facade.facade() == "ok"',
      '}',
      '',
    ].join('\n'),
  );
}

async function writeServiceWithDependency(serviceRoot, serviceId) {
  await mkdir(join(serviceRoot, 'api'), { recursive: true });
  await mkdir(join(serviceRoot, 'internal'), { recursive: true });
  await writeFile(
    join(serviceRoot, 'service.yml'),
    [
      `id: ${serviceId}`,
      'version: 0.1.0',
      'packages:',
      '  - id: google.com/cloud',
      '    version: 1.0.0',
      '    alias: gcloud',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(serviceRoot, 'api.yml'),
    [
      'ExampleService: internal.example.ExampleService',
      'api:',
      '  example:',
      '    Input: api.example.Input',
      '    Output: api.example.Output',
      '    ExampleService: api.example.ExampleService',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(serviceRoot, 'api', 'example.skiff'),
    [
      'type Input {}',
      'type Output {}',
      'interface ExampleService {',
      '  function run(input: Input) -> Output',
      '}',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(serviceRoot, 'internal', 'example.skiff'),
    [
      'type ExampleService {}',
      '',
      'impl ExampleService {',
      '  function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {',
      '    return root.api.example.Output {}',
      '  }',
      '}',
      '',
    ].join('\n'),
  );
}

async function writeCloudDependency(packageRoot, returnValue = 'ok') {
  await mkdir(join(packageRoot, 'cloud'), { recursive: true });
  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: google.com/cloud',
      'version: 1.0.0',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(packageRoot, 'api.yml'),
    [
      'storage:',
      '  upload: cloud.storage.upload',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(packageRoot, 'cloud', 'storage.skiff'),
    [
      'function upload() -> string {',
      `  return "${returnValue}"`,
      '}',
      '',
    ].join('\n'),
  );
}

async function withPackageRemote(action) {
  const archiveBytes = await createPackageSourceArchive();
  const server = createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    if (request.method === 'POST' && url.pathname === '/packages/download') {
      response.writeHead(200, { 'content-type': 'application/json; charset=utf-8' });
      response.end(JSON.stringify({
        revision: {
          packageId: 'skiff.run/llm',
          version: '1.0.0',
        },
        sourceArchiveBase64: archiveBytes.toString('base64'),
        sourceArchiveSize: archiveBytes.length,
      }));
      return;
    }
    response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
    response.end('not found');
  });
  const home = join(tempRoot, 'package-remote-home');
  await mkdir(home, { recursive: true });
  await listen(server);
  const address = server.address();
  const extraEnv = {
    HOME: home,
    USERPROFILE: home,
    SKIFF_PACKAGE_REMOTE_URL: `http://127.0.0.1:${address.port}`,
    SKIFF_PACKAGE_TOKEN: 'test-token',
  };
  try {
    await action(extraEnv);
  } finally {
    await closeServer(server);
  }
}

async function createPackageSourceArchive() {
  const root = await mkdtemp(join(tempRoot, 'pull-source-'));
  const archivePath = join(root, 'source.tgz');
  const source = join(root, 'source');
  await mkdir(source, { recursive: true });
  await writeFile(
    join(source, 'package.yml'),
    [
      'id: skiff.run/llm',
      'version: 1.0.0',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(source, 'api.yml'),
    [
      'main:',
      '  noop: main.noop',
      '',
    ].join('\n'),
  );
  await spawnSuccess('tar', ['-czf', archivePath, '-C', source, 'package.yml', 'api.yml']);
  return readFile(archivePath);
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolve();
    });
  });
}

function closeServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error);
        return;
      }
      resolve();
    });
  });
}

async function assertFileExists(path) {
  try {
    await access(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`expected file to exist: ${path}`);
    }
    throw error;
  }
}

function runSkiff(args, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [skiffCli, ...args], {
      cwd: tempRoot,
      env: { ...process.env, ...rustEnv, ...extraEnv },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0 && stdout.includes('test result: ok. 1 passed; 0 failed')) {
        resolve();
        return;
      }
      reject(new Error([
        `skiff ${args.join(' ')} failed with ${signal ?? code}`,
        stderr.trim(),
        stdout.trim(),
      ].filter(Boolean).join('\n')));
    });
  });
}

function runSkiffCommand(args, options = {}) {
  return runCommand(process.execPath, [skiffCli, ...args], options).then(({ code, signal, stdout, stderr }) => {
    if (code === 0) {
      return;
    }
    throw new Error([
      `skiff ${args.join(' ')} failed with ${signal ?? code}`,
      stderr.trim(),
      stdout.trim(),
    ].filter(Boolean).join('\n'));
  });
}

function runSkiffCommandExpectFailure(args, options = {}) {
  return runCommand(process.execPath, [skiffCli, ...args], options).then(({ code, stdout, stderr }) => {
    const output = `${stderr}\n${stdout}`;
    if (code !== 0 && output.includes(options.expectedOutput)) {
      return;
    }
    throw new Error([
      `skiff ${args.join(' ')} unexpectedly ${code === 0 ? 'succeeded' : `failed without expected output ${JSON.stringify(options.expectedOutput)}`}`,
      stderr.trim(),
      stdout.trim(),
    ].filter(Boolean).join('\n'));
  });
}

function spawnSuccess(command, args) {
  return runCommand(command, args).then(({ code, signal, stdout, stderr }) => {
    if (code === 0) {
      return;
    }
    throw new Error([
      `${command} ${args.join(' ')} failed with ${signal ?? code}`,
      stderr.trim(),
      stdout.trim(),
    ].filter(Boolean).join('\n'));
  });
}

function runCommand(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? tempRoot,
      env: { ...process.env, ...rustEnv, ...(options.extraEnv ?? {}) },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      resolve({ code, signal, stdout, stderr });
    });
  });
}

function runSkiffExpectFailure(args, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [skiffCli, ...args], {
      cwd: tempRoot,
      env: { ...process.env, ...rustEnv, ...extraEnv },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('exit', (code) => {
      const output = `${stderr}\n${stdout}`;
      if (code !== 0 && output.includes('package dependency google.com/cloud version 1.0.0 has no matching package.yml')) {
        resolve();
        return;
      }
      reject(new Error([
        `skiff ${args.join(' ')} unexpectedly ${code === 0 ? 'succeeded' : `failed with ${code}`}`,
        stderr.trim(),
        stdout.trim(),
      ].filter(Boolean).join('\n')));
    });
  });
}
