import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  KNOWN_BUILD_UNITS,
  assertProfileToken,
  loadStackConfig,
  parseStackConfigDirArg,
} from '../lib/stack-config.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');
const fixtureConfigDir = join(skiffRoot, 'scripts', 'fixtures', 'stack-config');

const VALID_CONFIG_YML = [
  'profile: prod',
  'remote:',
  '  host: root@example.test',
  '  remoteSkiff: /srv/skiff',
  '  nodeBin: /opt/node/bin',
  'verify:',
  '  httpPort: 4000',
  '  controlPort: 4001',
  '  telemetryPort: 4002',
  '  healthPath: /__router/health',
  '',
].join('\n');

const VALID_ROUTER_YML = [
  'profile: prod',
  'host: 127.0.0.1',
  'artifactsPath: /srv/skiff/artifacts',
  'serviceDb:',
  '  mongoUrl: mongodb://127.0.0.1:27017',
  '',
].join('\n');

test('loadStackConfig parses and validates the full fixture configDir', async (t) => {
  const stack = await loadStackConfig(fixtureConfigDir, { skiffRoot });
  t.after(() => {});
  assert.equal(stack.config.profile, 'prod');
  assert.equal(stack.router.profile, 'prod');
  assert.equal(stack.config.remote.host, 'root@skiff.hanzhe.com');
  assert.equal(stack.config.verify.healthPath, '/__router/health');
  assert.deepEqual(stack.build.units, ['runtime', 'router', 'telemetry']);
  assert.equal(stack.paths.buildRoot, join(skiffRoot, 'build', 'runtime-stack'));
  assert.equal(stack.paths.cargoTargetDir, join(skiffRoot, 'build', 'cargo-target'));
});

test('loadStackConfig fails closed when config.yml.profile != router.yml.profile', async (t) => {
  const root = await writeStackConfigDir(t, {
    router: 'profile: other\n',
  });
  await assert.rejects(
    loadStackConfig(root, { skiffRoot }),
    /stack profile mismatch: config\.yml\.profile="prod" but router\.yml\.profile="other"/,
  );
});

test('profile token rejects "." ".." and non-canonical characters', () => {
  assertProfileToken('prod', 'config.yml profile');
  assertProfileToken('a.b-c_9', 'config.yml profile');
  for (const token of ['.', '..', 'bad!', 'a/b', '', 'x'.repeat(201)]) {
    assert.throws(
      () => assertProfileToken(token, 'config.yml profile'),
      /canonical ASCII profile token/,
      token,
    );
  }
});

test('loadStackConfig requires complete remote and verify fields', async (t) => {
  const withoutHost = await writeStackConfigDir(t, {
    config: VALID_CONFIG_YML.replace('  host: root@example.test\n', ''),
  });
  await assert.rejects(
    loadStackConfig(withoutHost, { skiffRoot }),
    /config\.yml remote host is required/,
  );

  const withoutHealthPath = await writeStackConfigDir(t, {
    config: VALID_CONFIG_YML.replace('  healthPath: /__router/health\n', ''),
  });
  await assert.rejects(
    loadStackConfig(withoutHealthPath, { skiffRoot }),
    /config\.yml verify healthPath is required/,
  );
});

test('loadStackConfig rejects relative remote paths and invalid ports', async (t) => {
  const relativeSkiff = await writeStackConfigDir(t, {
    config: VALID_CONFIG_YML.replace('  remoteSkiff: /srv/skiff\n', '  remoteSkiff: srv/skiff\n'),
  });
  await assert.rejects(
    loadStackConfig(relativeSkiff, { skiffRoot }),
    /remote\.remoteSkiff must be an absolute path/,
  );

  const invalidPort = await writeStackConfigDir(t, {
    config: VALID_CONFIG_YML.replace('  controlPort: 4001\n', '  controlPort: 70000\n'),
  });
  await assert.rejects(
    loadStackConfig(invalidPort, { skiffRoot }),
    /config\.yml verify controlPort must be a TCP port/,
  );
});

test('loadStackConfig fails closed on unparseable YAML and missing files', async (t) => {
  const broken = await writeStackConfigDir(t, {
    router: 'profile: prod\n  nested: [unclosed\n',
  });
  await assert.rejects(
    loadStackConfig(broken, { skiffRoot }),
    /router\.yml YAML parse error/,
  );

  const missing = await writeStackConfigDir(t, { drop: ['telemetry.yml'] });
  await assert.rejects(
    loadStackConfig(missing, { skiffRoot }),
    /telemetry\.yml is required at/,
  );
});

test('build-only load reads build.yml without requiring the other four files', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-build-only-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    'buildRoot: build/runtime-stack',
    'cargoTargetDir: /absolute/cargo-target',
    'units:',
    '  - router',
    '  - router',
    '',
  ].join('\n'));
  const stack = await loadStackConfig(root, { skiffRoot, files: ['build.yml'] });
  assert.equal(stack.paths.buildRoot, join(skiffRoot, 'build', 'runtime-stack'));
  assert.equal(stack.paths.cargoTargetDir, '/absolute/cargo-target');
  assert.deepEqual(stack.build.units, ['router']);
});

test('build.yml units must be known build units', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-build-units-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    'buildRoot: build/runtime-stack',
    'cargoTargetDir: build/cargo-target',
    'units:',
    '  - not-a-unit',
    '',
  ].join('\n'));
  await assert.rejects(
    loadStackConfig(root, { skiffRoot, files: ['build.yml'] }),
    /build\.yml units must be one of/,
  );
  assert.equal(KNOWN_BUILD_UNITS.has('router'), true);
});

test('build.yml rejects process.telemetry managed (standalone repo owns the process)', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-telemetry-flag-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    'buildRoot: build/runtime-stack',
    'cargoTargetDir: build/cargo-target',
    'process:',
    '  telemetry: managed',
    '',
  ].join('\n'));
  await assert.rejects(
    loadStackConfig(root, { skiffRoot, files: ['build.yml'] }),
    /process\.telemetry must be "disabled"/,
  );
});

test('parseStackConfigDirArg accepts both option forms and rejects ambiguity', () => {
  assert.equal(
    parseStackConfigDirArg(['--configDir', '/tmp/stack']).configDir,
    '/tmp/stack',
  );
  assert.equal(
    parseStackConfigDirArg(['--configDir=/tmp/stack']).configDir,
    '/tmp/stack',
  );
  assert.throws(
    () => parseStackConfigDirArg([]),
    /requires --configDir/,
  );
  assert.throws(
    () => parseStackConfigDirArg(['--configDir']),
    /--configDir requires a directory path/,
  );
  assert.throws(
    () => parseStackConfigDirArg(['--configDir', '/a', '--configDir', '/b']),
    /provided more than once/,
  );
  assert.throws(
    () => parseStackConfigDirArg(['--remote', 'host']),
    /unknown stack option/,
  );
});

test('stack validate CLI exits zero on the fixture and non-zero on a broken configDir', async () => {
  const validateScript = join(skiffRoot, 'scripts', 'skiff-stack-validate.mjs');
  const ok = await spawnCapture(process.execPath, [
    validateScript,
    '--configDir',
    fixtureConfigDir,
  ]);
  assert.equal(ok.code, 0, ok.stderr);
  const summary = JSON.parse(ok.stdout);
  assert.equal(summary.ok, true);
  assert.equal(summary.profile, 'prod');

  const brokenRoot = await writeStackConfigDir(null, {
    router: 'profile: other\n',
  });
  try {
    const broken = await spawnCapture(process.execPath, [
      validateScript,
      '--configDir',
      brokenRoot,
    ]);
    assert.notEqual(broken.code, 0);
    assert.match(broken.stderr + broken.stdout, /stack profile mismatch/);
  } finally {
    await rm(brokenRoot, { recursive: true, force: true });
  }
});

async function writeStackConfigDir(t, {
  config = VALID_CONFIG_YML,
  router = VALID_ROUTER_YML,
  drop = [],
} = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-config-test-'));
  if (t !== null) {
    t.after(() => rm(root, { recursive: true, force: true }));
  }
  const files = {
    'build.yml': [
      'target: x86_64-unknown-linux-gnu',
      'zigDir: /cache/zig',
      'buildRoot: build/runtime-stack',
      'cargoTargetDir: build/cargo-target',
      '',
    ].join('\n'),
    'config.yml': config,
    'router.yml': router,
    'runtime.yml': 'router: ws://127.0.0.1:4001/runtime\nruntime-home: /srv/skiff/runtime-home\n',
    'telemetry.yml': 'telemetry:\n  host: 127.0.0.1\n  port: 4002\n',
  };
  for (const file of drop) {
    delete files[file];
  }
  for (const [file, contents] of Object.entries(files)) {
    await writeFile(join(root, file), contents);
  }
  return root;
}

function spawnCapture(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: skiffRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}
