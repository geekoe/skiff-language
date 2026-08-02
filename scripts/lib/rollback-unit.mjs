// Immutable TS rollback unit builder / verifier (§11.2 final form).
//
// The unit is built in a fresh temporary directory and never modified after
// the manifest is written: pinned self-contained Node runtime + the last TS
// Router source + materialized Router dependencies + package/lockfile +
// process spec + a full file/source identity map (every payload file
// relative path -> sha256 plus deterministic aggregate tree digest). The
// unit contains only relative symlinks (every link target is verified to
// stay inside the unit) and every process path is relative to the unit
// root, so a byte-copied unit verifies identically from a new directory
// (relocatable immutable artifact). Dependencies are installed inside the
// unit with a frozen lockfile (offline from the local pnpm store when
// possible), never copied from the workspace `router/node_modules`; startup
// never touches the network.

import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  access,
  chmod,
  copyFile,
  mkdir,
  open,
  readFile,
  readlink,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { homedir } from 'node:os';
import {
  dirname,
  isAbsolute,
  join,
  relative as relativePath,
  resolve,
} from 'node:path';
import { promisify } from 'node:util';

import { resolveRouterProcessSpec } from './dev-runtime-paths.mjs';
import {
  assertTsRollbackUnitManifest,
  buildRouterRollbackSwitchPlan,
  buildTsRollbackUnitManifest,
  routerRollbackUnitProcessRelative,
} from './rollback-manifest.mjs';

const execFileAsync = promisify(execFile);

export const ROLLBACK_UNIT_MANIFEST_FILE = 'rollback-unit.json';
export const DEFAULT_PINNED_NODE_VERSION = '22.17.0';

const ROUTER_SOURCE_ITEMS = [
  'package.json',
  'pnpm-lock.yaml',
  'pnpm-workspace.yaml',
  'tsconfig.json',
  'src',
];
const NODE_RUNTIME_DIR_IN_UNIT = 'node-runtime';
const ROUTER_DIR_IN_UNIT = 'router';

export async function discoverPinnedNodeRuntimeDir({ env = process.env } = {}) {
  const explicit = env.SKIFF_ROLLBACK_NODE_RUNTIME_DIR;
  if (explicit !== undefined && explicit.trim().length > 0) {
    const resolved = resolve(explicit.trim());
    await assertNodeRuntimeDir(resolved);
    return resolved;
  }
  const version = env.SKIFF_ROLLBACK_NODE_VERSION ?? DEFAULT_PINNED_NODE_VERSION;
  const candidate = join(homedir(), '.nvm', 'versions', 'node', `v${version}`);
  try {
    await assertNodeRuntimeDir(candidate);
    return candidate;
  } catch {
    throw new Error(
      'cannot discover a self-contained pinned Node runtime; set '
      + 'SKIFF_ROLLBACK_NODE_RUNTIME_DIR to an official Node distribution '
      + `directory with bin/node (tried ${candidate})`,
    );
  }
}

export async function buildImmutableTsRollbackUnit({
  unitRoot,
  repoRoot,
  nodeRuntimeDir,
  configPath,
  sourceCommit,
  pnpmCommand,
  offlineFirst = true,
}) {
  const resolvedUnitRoot = resolve(unitRoot);
  const resolvedRepoRoot = resolve(repoRoot);
  const resolvedConfigPath = resolve(configPath);
  await mkdir(resolvedUnitRoot, { recursive: true });
  const existing = await readdir(resolvedUnitRoot);
  if (existing.length > 0) {
    throw new Error(
      `immutable rollback unit root must be empty, found: ${existing.join(', ')}`,
    );
  }
  if (typeof sourceCommit !== 'string' || sourceCommit.trim().length === 0) {
    throw new Error('immutable rollback unit requires a non-empty sourceCommit');
  }

  const resolvedNodeRuntimeDir = resolve(nodeRuntimeDir);
  await assertNodeRuntimeDir(resolvedNodeRuntimeDir);
  const nodeInfo = await materializeNodeRuntime({
    unitRoot: resolvedUnitRoot,
    nodeRuntimeDir: resolvedNodeRuntimeDir,
  });
  await materializeRouterSource({
    unitRoot: resolvedUnitRoot,
    repoRoot: resolvedRepoRoot,
  });
  const installOffline = await installRouterDependencies({
    unitRoot: resolvedUnitRoot,
    pnpmCommand: pnpmCommand ?? 'pnpm',
    offlineFirst,
  });

  const payload = await hashUnitPayload(resolvedUnitRoot);
  const {
    files,
    symlinks,
    fileCount,
    symlinkCount,
    sha256Tree,
  } = payload;
  const devHome = dirname(resolvedConfigPath);
  const tsSpec = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot: resolvedRepoRoot,
  });
  const rustSpec = resolveRouterProcessSpec({
    devHome,
    implementation: 'rust',
    repoRoot: resolvedRepoRoot,
  });
  const unitProcess = routerRollbackUnitProcessRelative({
    configPath: resolvedConfigPath,
  });
  const manifest = buildTsRollbackUnitManifest({
    sourceCommit,
    configPath: resolvedConfigPath,
    pinnedNode: {
      version: nodeInfo.version,
      platform: nodeInfo.platform,
      arch: nodeInfo.arch,
      bin_path: 'node-runtime/bin/node',
      sha256: files['node-runtime/bin/node'],
    },
    routerSource: {
      root: ROUTER_DIR_IN_UNIT,
      file_count: countPrefixExclusive(files, 'router/', 'router/node_modules/'),
      sha256_tree: prefixTreeDigestExclusive(
        files,
        'router/',
        'router/node_modules/',
      ),
    },
    dependencies: {
      mode: 'materialized',
      root: 'router/node_modules',
      install_command: ['pnpm', '--dir', 'router', 'install', '--frozen-lockfile'],
      install_offline: installOffline,
      file_count: countPrefix(files, 'router/node_modules/'),
      sha256_tree: prefixTreeDigest(files, 'router/node_modules/'),
      symlink_count: countPrefix(symlinks, 'router/node_modules/'),
    },
    lockfiles: {
      'router/package.json': files['router/package.json'],
      'router/pnpm-lock.yaml': files['router/pnpm-lock.yaml'],
      'router/pnpm-workspace.yaml': files['router/pnpm-workspace.yaml'],
    },
    files,
    symlinks,
    fileCount,
    symlinkCount,
    sha256Tree,
    process: unitProcess,
    switchCommands: buildRouterRollbackSwitchPlan({
      tsSpec,
      rustSpec,
      tsUnitProcess: unitProcess,
    }),
  });
  const manifestPath = join(resolvedUnitRoot, ROLLBACK_UNIT_MANIFEST_FILE);
  await writeFile(
    manifestPath,
    `${JSON.stringify(manifest, null, 2)}\n`,
    { encoding: 'utf8', mode: 0o644 },
  );
  await assertImmutableTsRollbackUnit(resolvedUnitRoot);
  return { unitRoot: resolvedUnitRoot, manifestPath, manifest };
}

export async function assertImmutableTsRollbackUnit(unitRoot) {
  const resolvedUnitRoot = resolve(unitRoot);
  const manifestPath = join(resolvedUnitRoot, ROLLBACK_UNIT_MANIFEST_FILE);
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(
      `immutable rollback unit manifest is unreadable at ${manifestPath}`,
      { cause: error },
    );
  }
  assertTsRollbackUnitManifest(manifest);

  const payload = await hashUnitPayload(resolvedUnitRoot);
  assertPayloadMatchesManifest(payload, manifest, 'rollback unit');

  const pinnedNode = join(
    resolvedUnitRoot,
    manifest.pinned_node.bin_path,
  );
  const version = (await execFileAsync(pinnedNode, ['-v'])).stdout.trim();
  if (version !== manifest.pinned_node.version) {
    throw new Error(
      `rollback unit pinned node version drift: manifest ${manifest.pinned_node.version}, `
      + `binary reports ${version}`,
    );
  }
  const processCommand = resolveUnitRelative(resolvedUnitRoot, manifest.process.command);
  const [tsxCli, serverEntry] = manifest.process.args;
  for (const relative of [manifest.process.command, tsxCli, serverEntry]) {
    await access(resolveUnitRelative(resolvedUnitRoot, relative));
  }
  return manifest;
}

export async function copyImmutableTsRollbackUnit(srcRoot, destRoot) {
  const resolvedSrc = resolve(srcRoot);
  const resolvedDest = resolve(destRoot);
  await mkdir(resolvedDest, { recursive: true });
  const existing = await readdir(resolvedDest);
  if (existing.length > 0) {
    throw new Error(
      `immutable rollback unit destination must be empty, found: ${existing.join(', ')}`,
    );
  }
  await copyTree(resolvedSrc, resolvedDest);
  const manifest = await assertImmutableTsRollbackUnit(resolvedDest);
  return { unitRoot: resolvedDest, manifest };
}

async function copyTree(src, dest) {
  const sourceInfo = await stat(src);
  if (sourceInfo.isFile()) {
    await copyFile(src, dest);
    return;
  }
  await mkdir(dest, { recursive: true });
  const entries = await readdir(src, { withFileTypes: true });
  for (const entry of entries) {
    const from = join(src, entry.name);
    const to = join(dest, entry.name);
    if (entry.isSymbolicLink()) {
      // Recreate the link verbatim: `fs.cp` resolves relative links to
      // absolute targets on macOS, which would break unit relocatability.
      await symlink(await readlink(from), to);
    } else if (entry.isDirectory()) {
      await copyTree(from, to);
    } else if (entry.isFile()) {
      await copyFile(from, to);
    }
  }
}

async function materializeNodeRuntime({ unitRoot, nodeRuntimeDir }) {
  const sourceBin = join(nodeRuntimeDir, 'bin', 'node');
  const runtimeDir = join(unitRoot, NODE_RUNTIME_DIR_IN_UNIT);
  const runtimeBinDir = join(runtimeDir, 'bin');
  await mkdir(runtimeBinDir, { recursive: true });
  const destBin = join(runtimeBinDir, 'node');
  await copyFile(sourceBin, destBin);
  await chmod(destBin, 0o755);
  for (const name of ['LICENSE', 'README.md', 'CHANGELOG.md']) {
    const source = join(nodeRuntimeDir, name);
    try {
      await access(source);
      await copyFile(source, join(runtimeDir, name));
    } catch {
      // Optional provenance files are not required for a self-contained runtime.
    }
  }
  const version = (await execFileAsync(destBin, ['-v'])).stdout.trim();
  if (!/^v\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`pinned Node runtime reports unexpected version ${version}`);
  }
  const infoText = (await execFileAsync(
    destBin,
    ['-e', 'process.stdout.write(JSON.stringify({ platform: process.platform, arch: process.arch }))'],
  )).stdout;
  const { platform, arch } = JSON.parse(infoText);
  if (typeof platform !== 'string' || typeof arch !== 'string') {
    throw new Error('pinned Node runtime did not report platform/arch');
  }
  return { version, platform, arch, binPath: destBin };
}

async function materializeRouterSource({ unitRoot, repoRoot }) {
  const routerRoot = join(repoRoot, 'router');
  const unitRouterRoot = join(unitRoot, ROUTER_DIR_IN_UNIT);
  await mkdir(unitRouterRoot, { recursive: true });
  for (const item of ROUTER_SOURCE_ITEMS) {
    const source = join(routerRoot, item);
    try {
      await access(source);
    } catch {
      throw new Error(`router source item missing at ${source}`);
    }
    await copyTree(source, join(unitRouterRoot, item));
  }
}

async function installRouterDependencies({ unitRoot, pnpmCommand, offlineFirst }) {
  const routerRoot = join(unitRoot, ROUTER_DIR_IN_UNIT);
  const baseArgs = ['--dir', routerRoot, 'install', '--frozen-lockfile'];
  let offline = true;
  try {
    await execFileAsync(pnpmCommand, [...baseArgs, '--offline'], {
      cwd: unitRoot,
      maxBuffer: 16 * 1024 * 1024,
    });
  } catch (error) {
    if (!offlineFirst) {
      throw error;
    }
    offline = false;
    console.warn(
      'rollback unit: offline frozen install missed packages; '
      + 'falling back to the frozen install (build-time network)',
    );
    await execFileAsync(pnpmCommand, baseArgs, {
      cwd: unitRoot,
      maxBuffer: 16 * 1024 * 1024,
    });
  }
  const installedAt = join(routerRoot, 'node_modules');
  await access(installedAt);
  // pnpm runtime metadata embeds the build machine's store path; it is not
  // part of the Router dependency graph and would break relocatability.
  for (const metadataFile of [
    '.modules.yaml',
    '.pnpm-workspace-state-v1.json',
  ]) {
    await rm(join(installedAt, metadataFile), { force: true });
  }
  await relativizeSymlinks(unitRoot);
  return offline;
}

async function relativizeSymlinks(unitRoot) {
  const pending = [];
  async function walk(dir, relative) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const absolute = join(dir, entry.name);
      const entryRelative = relative === '' ? entry.name : `${relative}/${entry.name}`;
      if (entry.isSymbolicLink()) {
        pending.push({
          absolute,
          relative: entryRelative,
          target: await readlink(absolute),
        });
      } else if (entry.isDirectory()) {
        await walk(absolute, entryRelative);
      }
    }
  }
  await walk(unitRoot, '');
  for (const { absolute, relative, target } of pending) {
    const linkDir = dirname(absolute);
    const targetAbsolute = isAbsolute(target) ? target : resolve(linkDir, target);
    const fromUnit = relativePath(unitRoot, targetAbsolute);
    if (fromUnit.startsWith('..') || isAbsolute(fromUnit)) {
      throw new Error(
        `rollback unit symlink ${relative} escapes the unit (target ${target})`,
      );
    }
    const newTarget = relativePath(linkDir, targetAbsolute);
    await rm(absolute, { force: true });
    await symlink(newTarget, absolute);
  }
}

async function hashUnitPayload(unitRoot) {
  const files = {};
  const symlinks = {};
  for (const entry of await walkFiles(unitRoot)) {
    const { path: relative } = entry;
    if (relative === ROLLBACK_UNIT_MANIFEST_FILE) {
      continue;
    }
    if (entry.symlinkTarget !== undefined) {
      symlinks[relative] = entry.symlinkTarget;
      files[relative] = sha256Text(entry.symlinkTarget);
      await access(join(unitRoot, entry.resolvedTarget));
    } else {
      files[relative] = await sha256File(join(unitRoot, relative));
    }
  }
  const entries = Object.entries(files);
  return {
    files,
    symlinks,
    fileCount: entries.length,
    symlinkCount: Object.keys(symlinks).length,
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
      const target = await readlink(join(root, entry.name));
      const resolvedTarget = assertSelfContainedSymlink(entryRelative, target);
      results.push({
        path: entryRelative,
        symlinkTarget: target,
        resolvedTarget,
      });
    } else if (entry.isDirectory()) {
      results.push(...await walkFiles(join(root, entry.name), entryRelative));
    } else if (entry.isFile()) {
      results.push({ path: entryRelative });
    }
  }
  return results;
}

function assertSelfContainedSymlink(relative, target) {
  if (target.startsWith('/')) {
    throw new Error(
      `rollback unit symlink ${relative} must be relative, got ${target}`,
    );
  }
  const linkDir = relative.includes('/')
    ? relative.slice(0, relative.lastIndexOf('/'))
    : '';
  const resolved = normalizeRelative(join(linkDir, target));
  if (resolved === null || resolved.startsWith('..')) {
    throw new Error(
      `rollback unit symlink ${relative} escapes the unit (target ${target})`,
    );
  }
  return resolved;
}

function normalizeRelative(value) {
  const parts = [];
  for (const part of value.split('/')) {
    if (part === '' || part === '.') {
      continue;
    }
    if (part === '..') {
      if (parts.length === 0) {
        return null;
      }
      parts.pop();
      continue;
    }
    parts.push(part);
  }
  return parts.join('/');
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

function sha256Text(text) {
  return createHash('sha256').update(text, 'utf8').digest('hex');
}

function treeDigest(entries) {
  const hash = createHash('sha256');
  for (const [relative, digest] of [...entries].sort(([a], [b]) => a.localeCompare(b))) {
    hash.update(`${relative}\0${digest}\n`);
  }
  return hash.digest('hex');
}

function prefixTreeDigest(files, prefix) {
  return treeDigest(
    Object.entries(files).filter(([relative]) => relative.startsWith(prefix)),
  );
}

function countPrefix(files, prefix) {
  return Object.keys(files).filter((relative) => relative.startsWith(prefix)).length;
}

function countPrefixExclusive(files, prefix, excludedPrefix) {
  return Object.keys(files).filter(
    (relative) => relative.startsWith(prefix) && !relative.startsWith(excludedPrefix),
  ).length;
}

function prefixTreeDigestExclusive(files, prefix, excludedPrefix) {
  return treeDigest(
    Object.entries(files).filter(
      ([relative]) => relative.startsWith(prefix) && !relative.startsWith(excludedPrefix),
    ),
  );
}

function assertPayloadMatchesManifest(payload, manifest, label) {
  const {
    files,
    symlinks,
    fileCount,
    symlinkCount,
    sha256Tree,
  } = payload;
  if (fileCount !== manifest.file_count) {
    throw new Error(
      `${label} file count drift: manifest ${manifest.file_count}, actual ${fileCount}`,
    );
  }
  if (sha256Tree !== manifest.sha256_tree) {
    throw new Error(`${label} sha256 tree drift: manifest and unit payload differ`);
  }
  if (symlinkCount !== manifest.symlink_count) {
    throw new Error(`${label} symlink count drift`);
  }
  if (JSON.stringify(Object.entries(symlinks).sort()) !== JSON.stringify(Object.entries(manifest.symlinks).sort())) {
    throw new Error(`${label} symlink identity drift`);
  }
  const manifestFileKeys = Object.keys(manifest.files).sort();
  const actualFileKeys = Object.keys(files).sort();
  if (manifestFileKeys.join('\n') !== actualFileKeys.join('\n')) {
    throw new Error(`${label} file set drift (manifest vs unit payload)`);
  }
  for (const relative of manifestFileKeys) {
    if (files[relative] !== manifest.files[relative]) {
      throw new Error(`${label} file identity drift at ${relative}`);
    }
  }
  const routerPrefix = 'router/';
  const dependenciesPrefix = 'router/node_modules/';
  if (
    countPrefixExclusive(files, routerPrefix, dependenciesPrefix)
      !== manifest.router_source.file_count
    || prefixTreeDigestExclusive(files, routerPrefix, dependenciesPrefix)
      !== manifest.router_source.sha256_tree
  ) {
    throw new Error(`${label} router source identity drift`);
  }
  if (
    countPrefix(files, dependenciesPrefix) !== manifest.dependencies.file_count
    || prefixTreeDigest(files, dependenciesPrefix) !== manifest.dependencies.sha256_tree
    || countPrefix(symlinks, dependenciesPrefix) !== manifest.dependencies.symlink_count
  ) {
    throw new Error(`${label} dependencies identity drift`);
  }
}

async function assertNodeRuntimeDir(nodeRuntimeDir) {
  const binary = join(nodeRuntimeDir, 'bin', 'node');
  try {
    const info = await stat(binary);
    if (!info.isFile() || (info.mode & 0o111) === 0) {
      throw new Error(`${binary} must be an executable file`);
    }
  } catch (error) {
    throw new Error(`pinned Node runtime missing executable at ${binary}`, {
      cause: error,
    });
  }
}

function resolveUnitRelative(unitRoot, relative) {
  if (relative.startsWith('/')) {
    return relative;
  }
  return join(unitRoot, relative);
}
