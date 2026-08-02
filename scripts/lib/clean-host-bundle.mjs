// Clean-host deployment bundle for the Router Rust rollback rehearsal
// (plan §8 `router-clean-host-live` / §11.1 binary lifecycle).
//
// The bundle contains only the production payload a Linux/PM2 clean host
// needs: router + runtime binaries, YAML configs and the compiler artifact
// root, plus `sh` start scripts. It deliberately contains no Node runtime,
// no pnpm/tsx and no node_modules; the local rehearsal runs it with a PATH
// that cannot resolve pnpm/tsx. Runtime home stays outside the bundle
// (stateful path, same topology as deploy-runtime-stack). Real Linux/PM2
// clean-host runs belong to CI; this module only prepares and verifies the
// bundle shape and the local equivalent rehearsal.

import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  access,
  chmod,
  copyFile,
  cp,
  mkdir,
  open,
  readFile,
  readdir,
  writeFile,
} from 'node:fs/promises';
import { join, resolve } from 'node:path';

function execFileAsync(command, args, options) {
  return new Promise((resolve, reject) => {
    // child-process-owner: clean-host-exec-file
    execFile(command, args, options, (error, stdout, stderr) => {
      if (error !== null) {
        error.stdout = stdout;
        error.stderr = stderr;
        reject(error);
        return;
      }
      resolve({ stdout, stderr });
    });
  });
}

export const CLEAN_HOST_BUNDLE_SCHEMA = 'skiff-router-clean-host-bundle-v1';
export const CLEAN_HOST_BUNDLE_MANIFEST_FILE = 'bundle-manifest.json';
export const CLEAN_HOST_PATH = '/usr/bin:/bin:/usr/sbin:/sbin';

export const START_ROUTER_SCRIPT = `#!/bin/sh
set -eu
BUNDLE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec "$BUNDLE_ROOT/bin/skiff-router" "$BUNDLE_ROOT/config/router.yml"
`;

export const START_RUNTIME_SCRIPT = `#!/bin/sh
set -eu
BUNDLE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec "$BUNDLE_ROOT/bin/skiff-runtime" "$BUNDLE_ROOT/config/runtime.yml"
`;

export function cleanHostEnv(env = process.env, { home } = {}) {
  const result = {
    ...env,
    PATH: CLEAN_HOST_PATH,
    npm_config_offline: 'true',
  };
  if (home !== undefined) {
    const resolvedHome = resolve(home);
    result.HOME = resolvedHome;
    result.XDG_CONFIG_HOME = join(resolvedHome, '.config');
    result.XDG_CACHE_HOME = join(resolvedHome, '.cache');
    result.npm_config_cache = join(resolvedHome, '.npm-cache');
  }
  for (const key of [
    'PNPM_HOME',
    'COREPACK_HOME',
    'NVM_DIR',
    'NODE_PATH',
    'NODE_OPTIONS',
    'HTTP_PROXY',
    'HTTPS_PROXY',
    'ALL_PROXY',
    'http_proxy',
    'https_proxy',
    'all_proxy',
  ]) {
    delete result[key];
  }
  return result;
}

export async function assertNoPnpmOrTsxOnPath({ env, label = 'clean-host PATH' }) {
  const probe = await execFileAsync(
    '/bin/sh',
    [
      '-c',
      'if command -v pnpm >/dev/null 2>&1 || command -v tsx >/dev/null 2>&1; '
      + 'then echo PRESENT; else echo ABSENT; fi',
    ],
    { env },
  );
  if (probe.stdout.trim() !== 'ABSENT') {
    throw new Error(`${label} must not expose pnpm/tsx, got ${probe.stdout.trim()}`);
  }
  return true;
}

export async function buildCleanHostBundle({
  bundleRoot,
  routerBinary,
  runtimeBinary,
  routerConfigText,
  runtimeConfigText,
  artifactRoot,
  platform = process.platform,
}) {
  const resolvedBundleRoot = resolve(bundleRoot);
  await mkdir(resolvedBundleRoot, { recursive: true });
  const existing = await readdir(resolvedBundleRoot);
  if (existing.length > 0) {
    throw new Error(
      `clean-host bundle root must be empty, found: ${existing.join(', ')}`,
    );
  }
  const binDir = join(resolvedBundleRoot, 'bin');
  const configDir = join(resolvedBundleRoot, 'config');
  const scriptsDir = join(resolvedBundleRoot, 'scripts');
  const artifactsDir = join(resolvedBundleRoot, 'artifacts');
  for (const dir of [binDir, configDir, scriptsDir, artifactsDir]) {
    await mkdir(dir, { recursive: true });
  }

  await copyExecutable(routerBinary, join(binDir, 'skiff-router'));
  await copyExecutable(runtimeBinary, join(binDir, 'skiff-runtime'));
  await writeExclusive(join(configDir, 'router.yml'), routerConfigText);
  await writeExclusive(join(configDir, 'runtime.yml'), runtimeConfigText);
  await cp(resolve(artifactRoot), artifactsDir, { recursive: true });
  await writeExecutable(join(scriptsDir, 'start-router.sh'), START_ROUTER_SCRIPT);
  await writeExecutable(join(scriptsDir, 'start-runtime.sh'), START_RUNTIME_SCRIPT);

  const payload = await hashBundlePayload(resolvedBundleRoot);
  const { files, fileCount, sha256Tree } = payload;
  const manifest = {
    schemaVersion: CLEAN_HOST_BUNDLE_SCHEMA,
    platform,
    files,
    file_count: fileCount,
    sha256_tree: sha256Tree,
    process_commands: {
      router: {
        script: 'scripts/start-router.sh',
        exec: {
          command: 'bin/skiff-router',
          args: ['config/router.yml'],
        },
      },
      runtime: {
        script: 'scripts/start-runtime.sh',
        exec: {
          command: 'bin/skiff-runtime',
          args: ['config/runtime.yml'],
        },
      },
    },
  };
  const manifestPath = join(resolvedBundleRoot, CLEAN_HOST_BUNDLE_MANIFEST_FILE);
  await writeFile(
    manifestPath,
    `${JSON.stringify(manifest, null, 2)}\n`,
    { encoding: 'utf8', mode: 0o644 },
  );
  await assertCleanHostBundle(resolvedBundleRoot);
  return { bundleRoot: resolvedBundleRoot, manifestPath, manifest };
}

export async function assertCleanHostBundle(bundleRoot) {
  const resolvedBundleRoot = resolve(bundleRoot);
  const manifestPath = join(resolvedBundleRoot, CLEAN_HOST_BUNDLE_MANIFEST_FILE);
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(`clean-host bundle manifest is unreadable at ${manifestPath}`, {
      cause: error,
    });
  }
  assertCleanHostBundleManifest(manifest);
  const payload = await hashBundlePayload(resolvedBundleRoot);
  const { files, fileCount, sha256Tree } = payload;
  if (
    fileCount !== manifest.file_count
    || sha256Tree !== manifest.sha256_tree
    || Object.keys(files).sort().join('\n')
      !== Object.keys(manifest.files).sort().join('\n')
  ) {
    throw new Error('clean-host bundle payload identity drift');
  }
  for (const [relative, digest] of Object.entries(manifest.files)) {
    if (files[relative] !== digest) {
      throw new Error(`clean-host bundle file identity drift at ${relative}`);
    }
  }
  for (const key of ['router', 'runtime']) {
    const scriptPath = join(
      resolvedBundleRoot,
      manifest.process_commands[key].script,
    );
    const script = await readFile(scriptPath, 'utf8');
    if (!script.startsWith('#!/bin/sh\n')) {
      throw new Error(`clean-host ${key} start script must be POSIX sh`);
    }
    if (/\bpnpm\b|\btsx\b|node_modules/.test(script)) {
      throw new Error(`clean-host ${key} start script must not reference pnpm/tsx/node_modules`);
    }
    await access(join(resolvedBundleRoot, manifest.process_commands[key].exec.command));
  }
  return manifest;
}

export function assertCleanHostBundleManifest(manifest) {
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
    throw new Error('clean-host bundle manifest must be an object');
  }
  if (manifest.schemaVersion !== CLEAN_HOST_BUNDLE_SCHEMA) {
    throw new Error(
      `clean-host bundle manifest schema must be ${CLEAN_HOST_BUNDLE_SCHEMA}`,
    );
  }
  const expectedKeys = [
    'schemaVersion',
    'platform',
    'files',
    'file_count',
    'sha256_tree',
    'process_commands',
  ];
  const actualKeys = Object.keys(manifest).sort();
  if (actualKeys.join(',') !== [...expectedKeys].sort().join(',')) {
    throw new Error(
      `clean-host bundle manifest must contain exactly ${expectedKeys.join(', ')}`,
    );
  }
  if (typeof manifest.platform !== 'string' || manifest.platform.trim().length === 0) {
    throw new Error('clean-host bundle manifest platform must be a non-empty string');
  }
  if (!manifest.files || typeof manifest.files !== 'object' || Array.isArray(manifest.files)) {
    throw new Error('clean-host bundle manifest files must be an object');
  }
  const fileKeys = Object.keys(manifest.files);
  for (const relative of fileKeys) {
    if (typeof manifest.files[relative] !== 'string'
      || !/^[0-9a-f]{64}$/.test(manifest.files[relative])) {
      throw new Error(`clean-host bundle manifest files.${relative} must be a sha256 hex digest`);
    }
  }
  if (!Number.isSafeInteger(manifest.file_count) || manifest.file_count !== fileKeys.length) {
    throw new Error(
      'clean-host bundle manifest file_count must equal the files map size',
    );
  }
  if (typeof manifest.sha256_tree !== 'string'
    || !/^[0-9a-f]{64}$/.test(manifest.sha256_tree)) {
    throw new Error('clean-host bundle manifest sha256_tree must be a sha256 hex digest');
  }
  assertProcessCommand(manifest.process_commands?.router, 'router');
  assertProcessCommand(manifest.process_commands?.runtime, 'runtime');
  return manifest;
}

async function copyExecutable(source, dest) {
  await copyFile(source, dest);
  await chmod(dest, 0o755);
}

async function writeExclusive(path, text) {
  const handle = await open(path, 'wx', 0o600);
  try {
    await handle.writeFile(text, 'utf8');
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function writeExecutable(path, text) {
  const handle = await open(path, 'wx', 0o755);
  try {
    await handle.writeFile(text, 'utf8');
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function hashBundlePayload(bundleRoot) {
  const files = {};
  for (const relative of await walkFiles(bundleRoot)) {
    if (relative === CLEAN_HOST_BUNDLE_MANIFEST_FILE) {
      continue;
    }
    files[relative] = await sha256File(join(bundleRoot, relative));
  }
  const entries = Object.entries(files);
  return {
    files,
    fileCount: entries.length,
    sha256Tree: treeDigest(entries),
  };
}

async function walkFiles(root, relative = '') {
  const results = [];
  const entries = await readdir(root, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    const entryRelative = relative === '' ? entry.name : `${relative}/${entry.name}`;
    if (entry.isSymbolicLink()) {
      throw new Error(
        `clean-host bundle must contain no symlinks, found ${entryRelative} in ${root}`,
      );
    }
    if (entry.isDirectory()) {
      results.push(...await walkFiles(join(root, entry.name), entryRelative));
    } else if (entry.isFile()) {
      results.push(entryRelative);
    }
  }
  return results;
}

async function sha256File(path) {
  const hash = createHash('sha256');
  const handle = await open(path, 'r');
  try {
    const buffer = Buffer.alloc(1024 * 1024);
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length);
      if (bytesRead === 0) {
        break;
      }
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    await handle.close();
  }
  return hash.digest('hex');
}

function treeDigest(entries) {
  const hash = createHash('sha256');
  for (const [relative, digest] of [...entries].sort(([a], [b]) => a.localeCompare(b))) {
    hash.update(`${relative}\0${digest}\n`);
  }
  return hash.digest('hex');
}

function assertProcessCommand(processCommand, label) {
  if (!processCommand || typeof processCommand !== 'object' || Array.isArray(processCommand)) {
    throw new Error(`clean-host bundle manifest process_commands.${label} must be an object`);
  }
  const actualKeys = Object.keys(processCommand).sort();
  if (actualKeys.join(',') !== ['exec', 'script'].join(',')) {
    throw new Error(
      `clean-host bundle manifest process_commands.${label} must contain exactly script, exec`,
    );
  }
  if (typeof processCommand.script !== 'string' || processCommand.script.trim().length === 0) {
    throw new Error(`clean-host bundle manifest process_commands.${label}.script must be a string`);
  }
  const execCommand = processCommand.exec;
  if (!execCommand || typeof execCommand !== 'object' || Array.isArray(execCommand)) {
    throw new Error(`clean-host bundle manifest process_commands.${label}.exec must be an object`);
  }
  if (Object.keys(execCommand).sort().join(',') !== ['args', 'command'].join(',')) {
    throw new Error(
      `clean-host bundle manifest process_commands.${label}.exec must contain exactly command, args`,
    );
  }
  if (typeof execCommand.command !== 'string' || execCommand.command.trim().length === 0) {
    throw new Error(`clean-host bundle manifest process_commands.${label}.exec.command must be a string`);
  }
  if (!Array.isArray(execCommand.args)
    || execCommand.args.some((arg) => typeof arg !== 'string')) {
    throw new Error(`clean-host bundle manifest process_commands.${label}.exec.args must be an array of strings`);
  }
}
