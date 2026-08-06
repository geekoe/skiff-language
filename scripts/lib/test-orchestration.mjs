import { createHash } from 'node:crypto';
import { mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join, relative, resolve, sep as pathSeparator } from 'node:path';

import { cargoBuildEnv } from './cargo-target-dir.mjs';
import { captureAttachedCommand } from './command-execution.mjs';

const EXCLUDED_DIGEST_DIRECTORIES = new Set([
  'node_modules',
  '.git',
  'target',
  'dist',
  'build',
  '.stack',
  '.skiff-package-store',
  '.vscode',
]);

const BOOTSTRAP_PROFILE = 'skiff-test';

export async function discoverTestFiles(roots) {
  const found = new Set();
  for (const root of roots) {
    const resolved = resolve(root);
    let info;
    try {
      info = await stat(resolved);
    } catch (error) {
      throw new Error(`cannot inspect test root ${resolved}: ${error.message}`);
    }
    if (info.isFile()) {
      found.add(resolved);
      continue;
    }
    if (!info.isDirectory()) {
      throw new Error(`test root ${resolved} must be a file or directory`);
    }
    const pending = [resolved];
    while (pending.length > 0) {
      const directory = pending.pop();
      let entries;
      try {
        entries = await readdir(directory, { withFileTypes: true });
      } catch (error) {
        throw new Error(`failed to read test directory ${directory}: ${error.message}`);
      }
      for (const entry of entries) {
        if (entry.name === 'node_modules' || entry.name === '.git' || entry.name.startsWith('.')) {
          continue;
        }
        const entryPath = join(directory, entry.name);
        if (entry.isDirectory()) {
          pending.push(entryPath);
          continue;
        }
        if (entry.isFile() && entry.name.endsWith('.test.skiff')) {
          found.add(entryPath);
        }
      }
    }
  }
  return [...found].sort();
}

export async function countTestCases(files) {
  const counts = [];
  for (const file of files) {
    let source;
    try {
      source = await readFile(file, 'utf8');
    } catch (error) {
      throw new Error(`cannot read test file ${file}: ${error.message}`);
    }
    counts.push((source.match(/^\s*test\s+"/gm) ?? []).length);
  }
  return counts;
}

export function partitionTestFiles(files, counts, shardCount) {
  if (files.length === 0) {
    return [];
  }
  const capped = Math.min(shardCount, files.length);
  const shards = Array.from({ length: capped }, () => ({ files: [], cases: 0 }));
  const order = files
    .map((file, index) => ({ file, cases: counts[index] }))
    .sort((a, b) => b.cases - a.cases);
  for (const entry of order) {
    const target = shards.reduce(
      (least, shard) => (shard.cases < least.cases ? shard : least),
      shards[0],
    );
    target.files.push(entry.file);
    target.cases += entry.cases;
  }
  return shards
    .filter((shard) => shard.files.length > 0)
    .map((shard, index) => ({ index, files: shard.files, cases: shard.cases }));
}

export async function sourceTreeDigest(root) {
  const resolved = resolve(root);
  const files = [];
  const pending = [resolved];
  while (pending.length > 0) {
    const directory = pending.pop();
    let entries;
    try {
      entries = await readdir(directory, { withFileTypes: true });
    } catch (error) {
      throw new Error(`failed to read source tree ${directory}: ${error.message}`);
    }
    for (const entry of entries) {
      const entryPath = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!EXCLUDED_DIGEST_DIRECTORIES.has(entry.name)) {
          pending.push(entryPath);
        }
        continue;
      }
      if (!entry.isFile()) {
        continue;
      }
      const content = await readFile(entryPath);
      files.push({
        path: relative(resolved, entryPath).split(pathSeparator).join('/'),
        sha256: createHash('sha256').update(content).digest('hex'),
      });
    }
  }
  files.sort((a, b) => a.path.localeCompare(b.path));
  const digest = createHash('sha256')
    .update(files.map((file) => `${file.path}:${file.sha256}\n`).join(''))
    .digest('hex');
  return { digest, files };
}

export async function readSourceManifest(path) {
  let raw;
  try {
    raw = JSON.parse(await readFile(path, 'utf8'));
  } catch (error) {
    throw new Error(`cannot read source manifest ${path}: ${error.message}`);
  }
  if (!isPlainObject(raw)) {
    throw new Error(`source manifest ${path} must be a JSON object`);
  }
  const normalized = { packages: [], services: [], testServices: [] };
  for (const key of ['packages', 'services', 'testServices']) {
    const entries = raw[key];
    if (!Array.isArray(entries)) {
      throw new Error(`source manifest ${path} must declare array ${key}`);
    }
    for (const entry of entries) {
      normalized[key].push(normalizeManifestEntry(entry, key, path));
    }
  }
  return normalized;
}

export function enumerateManifestSources(manifest) {
  return [
    ...manifest.packages.map((entry) => ({ ...entry, kind: 'package' })),
    ...manifest.services.map((entry) => ({ ...entry, kind: 'service' })),
  ];
}

export async function readStoreSidecar(store) {
  try {
    const raw = JSON.parse(await readFile(join(store, 'skiff-test-sources.json'), 'utf8'));
    if (isPlainObject(raw) && Array.isArray(raw.sources)) {
      return { createdAt: raw.createdAt ?? null, sources: raw.sources };
    }
  } catch {
    // A missing or unreadable sidecar falls back to the empty store state.
  }
  return { createdAt: null, sources: [] };
}

export async function writeStoreSidecar(store, sidecar) {
  await mkdir(store, { recursive: true });
  const document = {
    createdAt: new Date().toISOString(),
    sources: sidecar.sources,
  };
  await writeFile(join(store, 'skiff-test-sources.json'), `${JSON.stringify(document, null, 2)}\n`);
  return document;
}

export async function planSourcePublish({ manifest, store, fresh }) {
  const sidecar = await readStoreSidecar(store);
  const sources = manifest === undefined ? [] : enumerateManifestSources(manifest);
  const full = fresh || (await inspectStoreState(store, sidecar)) !== 'present';
  const entries = [];
  for (const source of sources) {
    const previous = sidecar.sources.find((entry) => entry.coordinate === source.coordinate);
    const digest = await sourceTreeDigest(source.root);
    const firstPublish = previous === undefined;
    let action;
    let changedFiles = null;
    if (firstPublish) {
      action = 'publish';
    } else if (fresh || previous.digest !== digest.digest) {
      action = 'publish';
      changedFiles = countChangedFiles(previous, digest);
    } else {
      action = 'reuse';
    }
    entries.push({
      coordinate: source.coordinate,
      kind: source.kind,
      root: source.root,
      version: source.version,
      bootstrap: source.bootstrap === true,
      action,
      changedFiles,
      totalFiles: digest.files.length,
      firstPublish,
    });
  }
  return {
    mode: full ? 'hermetic full rebuild' : 'incremental reuse',
    entries,
  };
}

export async function publishSources({
  skiffRoot,
  store,
  manifest,
  entries,
  env,
  log = () => {},
}) {
  const sidecar = await readStoreSidecar(store);
  const planByCoordinate = new Map(entries.map((entry) => [entry.coordinate, entry]));
  for (const source of enumerateManifestSources(manifest)) {
    if (planByCoordinate.get(source.coordinate)?.action !== 'publish') {
      continue;
    }
    const invocation = source.bootstrap === true
      ? bootstrapSeedInvocation({ skiffRoot, store })
      : packagePublishInvocation({ skiffRoot, store, root: source.root });
    if (source.bootstrap === true) {
      // The bootstrap initializes the profile activation state with an
      // expected-absent CAS, so a reused store must reset that state first.
      await resetBootstrapProfileState(store, BOOTSTRAP_PROFILE);
    }
    const outcome = await captureAttachedCommand(invocation.command, invocation.args, {
      cwd: invocation.cwd,
      env,
    });
    if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
      const detail = outcome.stderr.trim() || outcome.stdout.trim()
        || outcome.error?.message || `exit ${outcome.signal ?? outcome.code}`;
      throw new Error(`source publish failed for ${source.coordinate}: ${detail}`);
    }
    const digest = await sourceTreeDigest(source.root);
    upsertSidecarSource(sidecar, {
      coordinate: source.coordinate,
      kind: source.kind,
      root: source.root,
      version: source.version,
      digest: digest.digest,
      files: digest.files,
    });
    log(`published ${source.coordinate}`);
  }
  return writeStoreSidecar(store, sidecar);
}

export function deriveBaseServices(manifest, testRoots) {
  const resolvedTestRoots = testRoots.map((root) => resolve(root));
  const testService = manifest.testServices.find((entry) =>
    resolvedTestRoots.some((root) => (
      root === entry.root || pathIsInside(entry.root, root)
    )),
  );
  if (testService === undefined) {
    throw new Error(
      'no --sources test root matches a manifest.testServices entry; provide --base-assembly and --base-config-snapshot together, or run a root declared in testServices',
    );
  }
  const subjectCoordinate = testService.subjectCoordinate;
  const baseServices = manifest.services.filter(
    (entry) => entry.coordinate !== subjectCoordinate,
  );
  if (baseServices.length === 0) {
    throw new Error(`no base services remain after excluding subject ${subjectCoordinate}`);
  }
  return { subjectCoordinate, testService, baseServices };
}

export async function resolveBasePair({ skiffRoot, manifest, store, testRoots, env }) {
  const { baseServices } = deriveBaseServices(manifest, testRoots);
  const cargoEnv = cargoBuildEnv(skiffRoot, env);
  const bases = [];
  for (const service of baseServices) {
    const pointerPath = serviceDeploymentPointerPath(store, service.coordinate, service.version);
    let document;
    try {
      document = JSON.parse(await readFile(pointerPath, 'utf8'));
    } catch (error) {
      throw new Error(
        `missing service deployment pointer for ${service.coordinate} at ${pointerPath}: ${error.message}; finish publishing sources first or use --fresh`,
      );
    }
    const deployment = document?.pointer?.deployment ?? document?.deployment;
    if (!isPlainObject(deployment)) {
      throw new Error(`service deployment pointer ${pointerPath} has no deployment object`);
    }
    bases.push({ coordinate: service.coordinate, root: service.root, deployment });
  }

  const assemblyArgs = [
    join(skiffRoot, 'scripts', 'skiff.mjs'),
    'assembly',
    'build',
    '--artifact-root',
    store,
    '--profile',
    'skiff-test',
  ];
  for (const base of bases) {
    assemblyArgs.push('--root-deployment', JSON.stringify(base.deployment));
  }
  assemblyArgs.push('--json');
  const assemblyOutcome = await captureAttachedCommand('node', assemblyArgs, {
    cwd: skiffRoot,
    env: cargoEnv,
  });
  if (assemblyOutcome.error !== null || assemblyOutcome.signal !== null || assemblyOutcome.code !== 0) {
    throw new Error(`assembly build failed: ${commandFailureDetail(assemblyOutcome)}`);
  }
  const assemblyReceipt = parseJsonOutput(assemblyOutcome.stdout, 'assembly build');
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly?.assemblyIdentity;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  if (typeof assembly !== 'string' || typeof recordPath !== 'string' || recordPath.length === 0) {
    throw new Error('assembly build did not return runtimeAssemblyReceipt.assembly.assemblyIdentity and recordPath');
  }

  const snapshotArgs = [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'config-snapshot-tooling', 'Cargo.toml'),
    '--',
    '--artifact-root',
    store,
    '--assembly-record',
    recordPath,
    '--profile',
    'skiff-test',
  ];
  for (const base of bases) {
    snapshotArgs.push('--source', JSON.stringify({ root: base.root, deployment: base.deployment }));
  }
  const snapshotOutcome = await captureAttachedCommand('cargo', snapshotArgs, {
    cwd: skiffRoot,
    env: cargoEnv,
  });
  if (snapshotOutcome.error !== null || snapshotOutcome.signal !== null || snapshotOutcome.code !== 0) {
    throw new Error(`config snapshot production failed: ${commandFailureDetail(snapshotOutcome)}`);
  }
  const snapshotReceipt = parseJsonOutput(snapshotOutcome.stdout, 'config snapshot production');
  const baseConfigSnapshot = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof baseConfigSnapshot !== 'string') {
    throw new Error('config snapshot production did not return runtimeConfigSnapshotReceipt.snapshot.snapshotId');
  }
  return {
    baseAssembly: assembly,
    baseConfigSnapshot,
    baseServices: bases.map((base) => base.coordinate),
  };
}

export async function runShardedTests({
  skiffRoot,
  shards,
  store,
  baseAssembly,
  baseConfigSnapshot,
  maxCases,
  env,
  cwd,
  log = console.log,
}) {
  const startedAt = Date.now();
  const shardEnv = shardedTestEnvironment(env, maxCases);
  const outcomes = await Promise.all(shards.map((shard) =>
    runOneShard({
      shard,
      skiffRoot,
      store,
      baseAssembly,
      baseConfigSnapshot,
      env: shardEnv,
      cwd,
      log,
    })));
  log(`[sharded] total ${Date.now() - startedAt}ms`);
  return outcomes.every((outcome) => outcome.passed);
}

export function renderPlan({ mode, store, sourceEntries, testLabel, baseLabel }) {
  const modeLine = mode === 'hermetic full rebuild'
    ? 'hermetic full rebuild（store 不存在 / --fresh）'
    : 'incremental reuse（store 已存在，sidecar 匹配）';
  const lines = [`mode:    ${modeLine}`, `store:   ${store}`, 'sources:'];
  const coordinateWidth = Math.max(0, ...sourceEntries.map((entry) => entry.coordinate.length));
  for (const entry of sourceEntries) {
    let note;
    if (entry.action === 'reuse') {
      note = 'unchanged';
    } else if (entry.firstPublish) {
      note = '首次发布';
    } else {
      note = `${entry.changedFiles} 个文件变更`;
    }
    lines.push(`  ${entry.action.padEnd(9)}${entry.coordinate.padEnd(coordinateWidth)}（${note}）`);
  }
  lines.push(`tests:    ${testLabel}`);
  lines.push(`base:     ${baseLabel}`);
  return lines.join('\n');
}

function bootstrapProfileStatePath(store, profile) {
  return join(store, 'profiles', profile, 'activation.json');
}

async function resetBootstrapProfileState(store, profile) {
  try {
    await rm(bootstrapProfileStatePath(store, profile), { force: true });
  } catch (error) {
    throw new Error(`failed to reset bootstrap profile state for ${profile}: ${error.message}`);
  }
}

function bootstrapSeedInvocation({ skiffRoot, store }) {
  return {
    command: 'cargo',
    cwd: skiffRoot,
    args: [
      'run',
      '--locked',
      '--quiet',
      '--manifest-path',
      join(skiffRoot, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root',
      store,
      '--profile',
      'skiff-test',
      '--platform-source-root',
      skiffRoot,
    ],
  };
}

function packagePublishInvocation({ skiffRoot, store, root }) {
  return {
    command: 'node',
    cwd: skiffRoot,
    args: [
      join(skiffRoot, 'scripts', 'skiff.mjs'),
      'package',
      'publish',
      root,
      '--artifact-root',
      store,
      '--json',
    ],
  };
}

function serviceDeploymentPointerPath(store, coordinate, version) {
  return join(
    store,
    'pointers',
    'service-deployments',
    coordinate.replaceAll('.', '~d').replaceAll('/', '~s'),
    `${version}.json`,
  );
}

function shardedTestEnvironment(env, maxCases) {
  const shardEnv = { ...env };
  if (maxCases !== undefined && maxCases !== null) {
    shardEnv.SKIFF_TEST_MAX_CASES_PER_ACTIVATION = String(maxCases);
  }
  for (const key of ['CARGO_TARGET_DIR', 'SKIFF_TEST_TRUSTED_SOURCE_ROOT']) {
    if (!(key in env)) {
      delete shardEnv[key];
    }
  }
  return shardEnv;
}

async function runOneShard({
  shard,
  skiffRoot,
  store,
  baseAssembly,
  baseConfigSnapshot,
  env,
  cwd,
  log,
}) {
  const args = [
    join(skiffRoot, 'scripts', 'skiff.mjs'),
    'test',
    ...shard.files,
    '--artifact-root',
    store,
  ];
  if (baseAssembly !== undefined) {
    args.push('--base-assembly', baseAssembly);
    args.push('--base-config-snapshot', baseConfigSnapshot);
  }
  args.push('--deny-skips', '--require-tests');
  const startedAt = Date.now();
  const outcome = await captureAttachedCommand('node', args, { cwd, env });
  const durationMs = Date.now() - startedAt;
  if (outcome.error === null && outcome.signal === null && outcome.code === 0) {
    log(`[shard ${shard.index}] PASS ${durationMs}ms (${shard.files.length} files, ${shard.cases} cases)`);
    return { passed: true };
  }
  log(`[shard ${shard.index}] FAIL ${durationMs}ms (${shard.files.length} files, ${shard.cases} cases)`);
  const combined = `${outcome.stdout ?? ''}\n${outcome.stderr ?? ''}`;
  const logDirectory = join(tmpdir(), 'skiff-sharded-test-logs');
  await mkdir(logDirectory, { recursive: true });
  const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
  const logPath = join(logDirectory, `shard-${shard.index}-${timestamp}.log`);
  await writeFile(logPath, `${combined.trimEnd()}\n`);
  log(`shard ${shard.index} full log: ${logPath}`);
  const tail = combined.trimEnd().split('\n').slice(-25);
  for (const line of tail) {
    log(line);
  }
  return { passed: false };
}

async function inspectStoreState(store, sidecar) {
  let entries;
  try {
    entries = await readdir(store);
  } catch {
    return 'missing';
  }
  if (entries.length === 0 || sidecar.sources.length === 0) {
    return 'empty';
  }
  return 'present';
}

function countChangedFiles(previous, digest) {
  const previousByPath = new Map(
    (previous.files ?? []).map((file) => [file.path, file.sha256]),
  );
  const currentByPath = new Map(digest.files.map((file) => [file.path, file.sha256]));
  let changed = 0;
  for (const [path, sha256] of currentByPath) {
    if (previousByPath.get(path) !== sha256) {
      changed += 1;
    }
  }
  for (const path of previousByPath.keys()) {
    if (!currentByPath.has(path)) {
      changed += 1;
    }
  }
  return changed;
}

function upsertSidecarSource(sidecar, source) {
  const index = sidecar.sources.findIndex((entry) => entry.coordinate === source.coordinate);
  if (index === -1) {
    sidecar.sources.push(source);
  } else {
    sidecar.sources[index] = source;
  }
  return sidecar;
}

function normalizeManifestEntry(entry, kind, path) {
  if (!isPlainObject(entry)) {
    throw new Error(`source manifest ${path} ${kind} entries must be objects`);
  }
  if (typeof entry.coordinate !== 'string' || entry.coordinate.length === 0) {
    throw new Error(`source manifest ${path} ${kind} entry requires a coordinate`);
  }
  if (typeof entry.root !== 'string' || entry.root.length === 0) {
    throw new Error(`source manifest ${path} ${kind} entry ${entry.coordinate} requires a root`);
  }
  if (typeof entry.version !== 'string' || entry.version.length === 0) {
    throw new Error(`source manifest ${path} ${kind} entry ${entry.coordinate} requires a version`);
  }
  if (
    kind === 'testServices'
    && (typeof entry.subjectCoordinate !== 'string' || entry.subjectCoordinate.length === 0)
  ) {
    throw new Error(`source manifest ${path} testServices entry ${entry.coordinate} requires a subjectCoordinate`);
  }
  const normalized = {
    coordinate: entry.coordinate,
    root: resolve(entry.root),
    version: entry.version,
  };
  if (kind === 'packages') {
    normalized.bootstrap = entry.bootstrap === true;
  }
  if (kind === 'testServices') {
    normalized.subjectCoordinate = entry.subjectCoordinate;
  }
  return normalized;
}

function pathIsInside(parent, child) {
  const path = relative(resolve(parent), resolve(child));
  return path === '' || (!path.startsWith('..') && !isAbsolute(path));
}

function parseJsonOutput(stdout, label) {
  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`${label} returned invalid JSON: ${error.message}`);
  }
}

function commandFailureDetail(outcome) {
  return outcome.stderr.trim() || outcome.stdout.trim()
    || outcome.error?.message || `exit ${outcome.signal ?? outcome.code}`;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
