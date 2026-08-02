import {
  assertRouterProcessSpec,
  routerProcessInvocation,
} from './dev-runtime-paths.mjs';
import { join, resolve } from 'node:path';

export const ROUTER_ROLLBACK_MANIFEST_SCHEMA = 'skiff-router-rollback-unit-v1';
export const ROUTER_ROLLBACK_UNIT_SCHEMA = 'skiff-router-rollback-ts-unit-v1';
export const ROUTER_ROLLBACK_SWITCH_SCHEMA = 'skiff-router-rollback-switch-v1';

export const ROLLBACK_UNIT_NODE_BIN = 'node-runtime/bin/node';
export const ROLLBACK_UNIT_TSX_CLI = 'router/node_modules/tsx/dist/cli.mjs';
export const ROLLBACK_UNIT_SERVER_ENTRY = 'router/src/router/server.ts';

export function buildRouterRollbackManifest(spec) {
  assertRouterProcessSpec(spec);
  const process = routerProcessInvocation(spec);
  return Object.freeze({
    schemaVersion: ROUTER_ROLLBACK_MANIFEST_SCHEMA,
    implementation: spec.implementation,
    config_path: spec.config_path,
    ...(spec.implementation === 'ts'
      ? { ts_source_root: spec.ts_source_root }
      : { rust_binary_path: spec.rust_binary_path }),
    process: Object.freeze({
      command: process.command,
      args: Object.freeze([...process.args]),
    }),
  });
}

export function assertRouterRollbackManifest(manifest) {
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
    throw new Error('router rollback manifest must be an object');
  }
  if (manifest.schemaVersion !== ROUTER_ROLLBACK_MANIFEST_SCHEMA) {
    throw new Error(
      `router rollback manifest schema must be ${ROUTER_ROLLBACK_MANIFEST_SCHEMA}`,
    );
  }
  const spec = {
    implementation: manifest.implementation,
    config_path: manifest.config_path,
  };
  if (spec.implementation === 'ts') {
    spec.ts_source_root = manifest.ts_source_root;
  } else {
    spec.rust_binary_path = manifest.rust_binary_path;
  }
  assertRouterProcessSpec(spec);
  const expectedKeys = spec.implementation === 'ts'
    ? ['schemaVersion', 'implementation', 'config_path', 'ts_source_root', 'process']
    : ['schemaVersion', 'implementation', 'config_path', 'rust_binary_path', 'process'];
  const actualKeys = Object.keys(manifest).sort();
  if (actualKeys.join(',') !== [...expectedKeys].sort().join(',')) {
    throw new Error(
      `router rollback manifest must contain exactly ${expectedKeys.join(', ')}`,
    );
  }
  const process = routerProcessInvocation(spec);
  if (
    !manifest.process
    || typeof manifest.process !== 'object'
    || Array.isArray(manifest.process)
    || manifest.process.command !== process.command
    || !Array.isArray(manifest.process.args)
    || manifest.process.args.join('\0') !== process.args.join('\0')
  ) {
    throw new Error(
      'router rollback manifest process command must match the RouterProcessSpec invocation',
    );
  }
  return manifest;
}

// Immutable TS rollback unit (§11.2 final form): pinned Node runtime + last
// TS source + materialized Router dependencies + package/lockfile + process
// spec + all file/source identity. The unit is relocatable: every unit file
// path in `process` is relative to the unit root and `files` is keyed by
// relative paths, so a copied unit re-verifies against the same manifest.
export function routerRollbackUnitProcessRelative({ configPath }) {
  if (
    typeof configPath !== 'string'
    || configPath.trim().length === 0
    || resolve(configPath) !== configPath
  ) {
    throw new Error('router rollback unit process requires an absolute config_path');
  }
  return Object.freeze({
    command: ROLLBACK_UNIT_NODE_BIN,
    args: Object.freeze([
      ROLLBACK_UNIT_TSX_CLI,
      ROLLBACK_UNIT_SERVER_ENTRY,
      '--config',
      configPath,
    ]),
  });
}

export function resolveRouterRollbackUnitProcess(processSpec, unitRoot) {
  const resolvedUnitRoot = resolve(unitRoot);
  assertProcessInvocation(processSpec, 'router rollback unit process');
  if (processSpec.args.length !== 4 || processSpec.args[2] !== '--config') {
    throw new Error(
      'router rollback unit process.args must be [tsx cli, server entry, --config, config_path]',
    );
  }
  const [tsxCli, serverEntry, flag, configPath] = processSpec.args;
  for (const arg of [processSpec.command, tsxCli, serverEntry]) {
    assertRollbackUnitRelativePath(arg, 'router rollback unit process path');
  }
  if (!isAbsolutePath(configPath)) {
    throw new Error('router rollback unit config_path must be an absolute path');
  }
  return Object.freeze({
    command: join(resolvedUnitRoot, processSpec.command),
    args: Object.freeze([
      join(resolvedUnitRoot, tsxCli),
      join(resolvedUnitRoot, serverEntry),
      flag,
      configPath,
    ]),
  });
}

export function buildRouterRollbackSwitchPlan({
  tsSpec,
  rustSpec,
  tsUnitProcess,
}) {
  assertRouterProcessSpec(tsSpec);
  assertRouterProcessSpec(rustSpec);
  if (tsUnitProcess !== undefined) {
    assertProcessInvocation(tsUnitProcess, 'tsUnitProcess');
  }
  const tsToRust = buildRollbackTransition({
    from: 'ts',
    to: 'rust',
    start: routerProcessInvocation(rustSpec),
  });
  const rustToTs = buildRollbackTransition({
    from: 'rust',
    to: 'ts',
    start: tsUnitProcess ?? routerProcessInvocation(tsSpec),
  });
  return Object.freeze({
    schemaVersion: ROUTER_ROLLBACK_SWITCH_SCHEMA,
    phases: Object.freeze(['ts', 'rust', 'ts']),
    transitions: Object.freeze({
      'ts->rust': tsToRust,
      'rust->ts': rustToTs,
    }),
  });
}

export function assertRouterRollbackSwitchPlan(plan) {
  if (!plan || typeof plan !== 'object' || Array.isArray(plan)) {
    throw new Error('router rollback switch plan must be an object');
  }
  if (plan.schemaVersion !== ROUTER_ROLLBACK_SWITCH_SCHEMA) {
    throw new Error(
      `router rollback switch plan schema must be ${ROUTER_ROLLBACK_SWITCH_SCHEMA}`,
    );
  }
  const expectedKeys = ['schemaVersion', 'phases', 'transitions'];
  assertExactKeys(plan, expectedKeys, 'router rollback switch plan');
  assertEqual(
    plan.phases,
    ['ts', 'rust', 'ts'],
    'router rollback switch plan phases',
  );
  assertExactKeys(
    plan.transitions,
    ['ts->rust', 'rust->ts'],
    'router rollback switch plan transitions',
  );
  assertRollbackTransition(plan.transitions['ts->rust'], 'ts', 'rust');
  assertRollbackTransition(plan.transitions['rust->ts'], 'rust', 'ts');
  return plan;
}

export function buildTsRollbackUnitManifest({
  sourceCommit,
  configPath,
  pinnedNode,
  routerSource,
  dependencies,
  lockfiles,
  files,
  symlinks,
  fileCount,
  symlinkCount,
  sha256Tree,
  process: processSpec,
  switchCommands,
}) {
  const resolvedConfigPath = resolve(configPath);
  const unitProcess = processSpec ?? routerRollbackUnitProcessRelative({
    configPath: resolvedConfigPath,
  });
  assertProcessInvocation(unitProcess, 'router rollback unit process');
  const manifest = {
    schemaVersion: ROUTER_ROLLBACK_UNIT_SCHEMA,
    implementation: 'ts',
    source_commit: sourceCommit,
    config_path: resolvedConfigPath,
    pinned_node: deepFreeze({ ...pinnedNode }),
    router_source: deepFreeze({ ...routerSource }),
    dependencies: deepFreeze({ ...dependencies }),
    lockfiles: deepFreeze({ ...lockfiles }),
    files: deepFreeze({ ...files }),
    symlinks: deepFreeze({ ...symlinks }),
    file_count: fileCount,
    symlink_count: symlinkCount,
    sha256_tree: sha256Tree,
    process: deepFreeze({
      command: unitProcess.command,
      args: [...unitProcess.args],
    }),
    switch_commands: switchCommands,
  };
  return Object.freeze(manifest);
}

export function assertTsRollbackUnitManifest(manifest) {
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
    throw new Error('router rollback unit manifest must be an object');
  }
  if (manifest.schemaVersion !== ROUTER_ROLLBACK_UNIT_SCHEMA) {
    throw new Error(
      `router rollback unit manifest schema must be ${ROUTER_ROLLBACK_UNIT_SCHEMA}`,
    );
  }
  const expectedKeys = [
    'schemaVersion',
    'implementation',
    'source_commit',
    'config_path',
    'pinned_node',
    'router_source',
    'dependencies',
    'lockfiles',
    'files',
    'symlinks',
    'file_count',
    'symlink_count',
    'sha256_tree',
    'process',
    'switch_commands',
  ];
  assertExactKeys(manifest, expectedKeys, 'router rollback unit manifest');
  if (manifest.implementation !== 'ts') {
    throw new Error('router rollback unit manifest implementation must be "ts"');
  }
  if (
    typeof manifest.source_commit !== 'string'
    || manifest.source_commit.trim().length === 0
  ) {
    throw new Error('router rollback unit manifest source_commit must be a non-empty string');
  }
  if (!isAbsolutePath(manifest.config_path)) {
    throw new Error('router rollback unit manifest config_path must be an absolute path');
  }

  const pinnedNode = manifest.pinned_node;
  assertExactKeys(
    pinnedNode,
    ['version', 'platform', 'arch', 'bin_path', 'sha256'],
    'router rollback unit pinned_node',
  );
  for (const key of ['version', 'platform', 'arch']) {
    if (typeof pinnedNode[key] !== 'string' || pinnedNode[key].trim().length === 0) {
      throw new Error(`router rollback unit pinned_node.${key} must be a non-empty string`);
    }
  }
  assertRollbackUnitRelativePath(pinnedNode.bin_path, 'pinned_node.bin_path');
  assertSha256(pinnedNode.sha256, 'pinned_node.sha256');

  assertExactKeys(
    manifest.router_source,
    ['root', 'file_count', 'sha256_tree'],
    'router rollback unit router_source',
  );
  assertSourceIdentity(manifest.router_source, 'router_source');
  if (manifest.router_source.root !== 'router') {
    throw new Error('router rollback unit router_source.root must be "router"');
  }
  assertExactKeys(
    manifest.dependencies,
    [
      'mode',
      'root',
      'install_command',
      'install_offline',
      'file_count',
      'sha256_tree',
      'symlink_count',
    ],
    'router rollback unit dependencies',
  );
  assertSourceIdentity(manifest.dependencies, 'dependencies');
  if (manifest.dependencies.mode !== 'materialized') {
    throw new Error('router rollback unit dependencies.mode must be "materialized"');
  }
  if (manifest.dependencies.root !== 'router/node_modules') {
    throw new Error(
      'router rollback unit dependencies.root must be "router/node_modules"',
    );
  }
  if (
    !Array.isArray(manifest.dependencies.install_command)
    || manifest.dependencies.install_command.length === 0
    || manifest.dependencies.install_command[0] !== 'pnpm'
    || manifest.dependencies.install_command.some(
      (part) => typeof part !== 'string',
    )
  ) {
    throw new Error(
      'router rollback unit dependencies.install_command must start with pnpm',
    );
  }
  if (typeof manifest.dependencies.install_offline !== 'boolean') {
    throw new Error(
      'router rollback unit dependencies.install_offline must be a boolean',
    );
  }
  if (
    !Number.isSafeInteger(manifest.dependencies.symlink_count)
    || manifest.dependencies.symlink_count < 0
  ) {
    throw new Error(
      'router rollback unit dependencies.symlink_count must be a safe integer',
    );
  }

  const expectedLockfiles = [
    'router/package.json',
    'router/pnpm-lock.yaml',
    'router/pnpm-workspace.yaml',
  ];
  assertExactKeys(manifest.lockfiles, expectedLockfiles, 'router rollback unit lockfiles');
  for (const key of expectedLockfiles) {
    assertSha256(manifest.lockfiles[key], `lockfiles.${key}`);
  }

  if (!isPlainObject(manifest.files)) {
    throw new Error('router rollback unit manifest files must be an object');
  }
  if (!isPlainObject(manifest.symlinks)) {
    throw new Error('router rollback unit manifest symlinks must be an object');
  }
  const fileKeys = Object.keys(manifest.files).sort();
  for (const key of fileKeys) {
    assertRollbackUnitRelativePath(key, `files key`);
    assertSha256(manifest.files[key], `files.${key}`);
  }
  const symlinkKeys = Object.keys(manifest.symlinks).sort();
  for (const key of symlinkKeys) {
    assertRollbackUnitRelativePath(key, 'symlinks key');
    const target = manifest.symlinks[key];
    if (
      typeof target !== 'string'
      || target.trim().length === 0
      || target.startsWith('/')
    ) {
      throw new Error(
        `router rollback unit manifest symlinks.${key} must be a relative target`,
      );
    }
  }
  if (!Number.isSafeInteger(manifest.file_count) || manifest.file_count !== fileKeys.length) {
    throw new Error(
      `router rollback unit manifest file_count must equal files map size (${fileKeys.length})`,
    );
  }
  if (
    !Number.isSafeInteger(manifest.symlink_count)
    || manifest.symlink_count !== symlinkKeys.length
  ) {
    throw new Error(
      `router rollback unit manifest symlink_count must equal symlinks map size (${symlinkKeys.length})`,
    );
  }
  if (!symlinkKeys.every((key) => fileKeys.includes(key))) {
    throw new Error(
      'router rollback unit manifest symlinks must also appear in files',
    );
  }
  if (
    manifest.router_source.file_count + manifest.dependencies.file_count
      >= manifest.file_count
  ) {
    throw new Error(
      'router rollback unit manifest subsets must not cover the whole unit '
      + '(pinned Node runtime files are outside router source and dependencies)',
    );
  }
  assertSha256(manifest.sha256_tree, 'sha256_tree');

  const processSpec = manifest.process;
  assertProcessInvocation(processSpec, 'router rollback unit process');
  assertRollbackUnitRelativePath(processSpec.command, 'process.command');
  if (processSpec.command !== ROLLBACK_UNIT_NODE_BIN) {
    throw new Error(
      `router rollback unit process.command must be ${ROLLBACK_UNIT_NODE_BIN}`,
    );
  }
  const expectedProcessArgs = [
    ROLLBACK_UNIT_TSX_CLI,
    ROLLBACK_UNIT_SERVER_ENTRY,
    '--config',
    manifest.config_path,
  ];
  if (processSpec.args.join('\0') !== expectedProcessArgs.join('\0')) {
    throw new Error('router rollback unit process.args must match the canonical unit invocation');
  }
  assertRouterRollbackSwitchPlan(manifest.switch_commands);
  const rustToTsStart = manifest.switch_commands.transitions['rust->ts'].start;
  if (
    rustToTsStart.command !== processSpec.command
    || rustToTsStart.args.join('\0') !== processSpec.args.join('\0')
  ) {
    throw new Error(
      'router rollback unit rust->ts switch start must match the unit process',
    );
  }
  return manifest;
}

function buildRollbackTransition({ from, to, start }) {
  assertProcessInvocation(start, `${from}->${to} start`);
  return Object.freeze({
    from,
    to,
    stop: Object.freeze({
      signal: 'SIGTERM',
      expect_exit_code: 0,
      verify_listeners_closed: true,
    }),
    start: Object.freeze({
      command: start.command,
      args: Object.freeze([...start.args]),
    }),
  });
}

function assertRollbackTransition(transition, from, to) {
  if (!transition || typeof transition !== 'object' || Array.isArray(transition)) {
    throw new Error(`router rollback switch transition ${from}->${to} must be an object`);
  }
  assertExactKeys(transition, ['from', 'to', 'stop', 'start'], `transition ${from}->${to}`);
  if (transition.from !== from || transition.to !== to) {
    throw new Error(`router rollback switch transition must be ${from}->${to}`);
  }
  assertExactKeys(
    transition.stop,
    ['signal', 'expect_exit_code', 'verify_listeners_closed'],
    `transition ${from}->${to} stop`,
  );
  if (transition.stop.signal !== 'SIGTERM') {
    throw new Error(`transition ${from}->${to} stop.signal must be SIGTERM`);
  }
  if (transition.stop.expect_exit_code !== 0) {
    throw new Error(`transition ${from}->${to} stop.expect_exit_code must be 0`);
  }
  if (transition.stop.verify_listeners_closed !== true) {
    throw new Error(
      `transition ${from}->${to} stop.verify_listeners_closed must be true`,
    );
  }
  assertProcessInvocation(transition.start, `transition ${from}->${to} start`);
}

function assertProcessInvocation(invocation, label) {
  if (!invocation || typeof invocation !== 'object' || Array.isArray(invocation)) {
    throw new Error(`${label} must be an object`);
  }
  assertExactKeys(invocation, ['command', 'args'], label);
  if (
    typeof invocation.command !== 'string'
    || invocation.command.trim().length === 0
  ) {
    throw new Error(`${label}.command must be a non-empty string`);
  }
  if (
    !Array.isArray(invocation.args)
    || invocation.args.some((arg) => typeof arg !== 'string')
  ) {
    throw new Error(`${label}.args must be an array of strings`);
  }
}

function assertSourceIdentity(identity, label) {
  if (!identity || typeof identity !== 'object' || Array.isArray(identity)) {
    throw new Error(`router rollback unit ${label} must be an object`);
  }
  if (typeof identity.root !== 'string' || identity.root.trim().length === 0) {
    throw new Error(`router rollback unit ${label}.root must be a non-empty string`);
  }
  if (!Number.isSafeInteger(identity.file_count) || identity.file_count < 0) {
    throw new Error(`router rollback unit ${label}.file_count must be a safe integer`);
  }
  assertSha256(identity.sha256_tree, `${label}.sha256_tree`);
}

function assertRollbackUnitRelativePath(value, label) {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  if (isAbsolutePath(value)) {
    throw new Error(`${label} must be relative to the unit root`);
  }
  const parts = value.split('/');
  if (parts.some((part) => part === '..' || part === '')) {
    throw new Error(`${label} must be a clean relative path`);
  }
}

function assertSha256(value, label) {
  if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`${label} must be a sha256 hex digest`);
  }
}

function assertExactKeys(value, expectedKeys, label) {
  if (!isPlainObject(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actualKeys = Object.keys(value).sort();
  if (actualKeys.join(',') !== [...expectedKeys].sort().join(',')) {
    throw new Error(
      `${label} must contain exactly ${expectedKeys.join(', ')}`,
    );
  }
}

function assertEqual(actual, expected, label) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function isAbsolutePath(value) {
  return typeof value === 'string' && value.trim().length > 0 && resolve(value) === value;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
