import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { deployStack, renderEcosystemConfig } from '../lib/stack-deploy.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('deploy copies the three YAML files verbatim from configDir', async (t) => {
  const { configDir, buildRoot, shell, calls } = await deployFixture(t);
  const result = await deployStack({ configDir, skiffRoot, shell });

  const configRsyncs = calls.filter((call) => (
    call.op === 'rsync'
    && call.destination.startsWith('deploy.test:/srv/skiff/config/')
    && call.destination.endsWith('.yml')
  ));
  assert.equal(configRsyncs.length, 3);
  for (const file of ['router.yml', 'runtime.yml', 'telemetry.yml']) {
    const call = calls.find((entry) => (
      entry.op === 'rsync'
      && entry.destination === `deploy.test:/srv/skiff/config/${file}`
    ));
    assert.ok(call, `missing rsync for ${file}`);
    const copied = await readFile(call.source, 'utf8');
    assert.equal(copied, await readFile(join(configDir, file), 'utf8'));
    assert.ok(!copied.includes('deploy.test'), `${file} must be copied verbatim`);
  }
  assert.equal(result.config.length, 3);
});

test('deploy uploads manifest binaries, installs telemetry, and PM2 deletes before startOrReload', async (t) => {
  const { configDir, shell, calls, buildRoot } = await deployFixture(t);
  await deployStack({ configDir, skiffRoot, shell });

  for (const [unit, target] of [
    ['router', 'skiff-router'],
    ['runtime', 'skiff-runtime'],
    ['compiler', 'skiff-compiler'],
  ]) {
    const upload = calls.find((call) => (
      call.op === 'rsync'
      && call.destination === `deploy.test:/srv/skiff/bin/${target}`
    ));
    assert.ok(upload, `missing binary upload for ${unit}`);
    assert.equal(upload.source, join(buildRoot, 'bin', target));
    const chmod = calls.find((call) => (
      call.op === 'ssh'
      && call.command === `chmod +x /srv/skiff/bin/${target}`
    ));
    assert.ok(chmod, `missing chmod for ${target}`);
  }

  assert.ok(calls.some((call) => (
    call.op === 'ssh'
    && call.command === 'cd /srv/skiff/telemetry && PATH=/opt/node/bin:$PATH pnpm install --prod=false --ignore-scripts'
  )), 'telemetry dependencies must be installed');

  const deletes = calls
    .filter((call) => call.op === 'ssh' && call.command.includes('pm2 delete'))
    .map((call) => call.command);
  const reloads = calls
    .filter((call) => call.op === 'ssh' && call.command.includes('startOrReload'))
    .map((call) => call.command);
  assert.equal(deletes.length, 3);
  assert.equal(reloads.length, 3);
  for (const app of ['skiff-router', 'skiff-runtime', 'skiff-telemetry']) {
    const deleteIndex = calls.findIndex((call) => (
      call.op === 'ssh' && call.command.includes(`pm2 delete ${app} || true`)
    ));
    const reloadIndex = calls.findIndex((call) => (
      call.op === 'ssh' && call.command.includes(`startOrReload ecosystem.config.cjs --only ${app}`)
    ));
    assert.ok(deleteIndex !== -1 && reloadIndex !== -1, app);
    assert.ok(deleteIndex < reloadIndex, `${app} must be deleted before startOrReload`);
  }
  assert.ok(calls.some((call) => (
    call.op === 'ssh' && call.command === 'PATH=/opt/node/bin:$PATH pm2 save'
  )), 'pm2 save must run');
});

test('deploy fails closed on profile mismatch before any remote command', async (t) => {
  const { configDir, shell, calls } = await deployFixture(t, {
    router: 'profile: other\nhost: 127.0.0.1\n',
  });
  await assert.rejects(
    deployStack({ configDir, skiffRoot, shell }),
    /stack profile mismatch/,
  );
  assert.equal(calls.length, 0);
});

test('deploy fails closed when the router or runtime build unit is missing', async (t) => {
  const { configDir, shell, calls, buildRoot } = await deployFixture(t);
  const manifestPath = join(buildRoot, 'manifest.json');
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  delete manifest.units.router;
  await writeFile(manifestPath, JSON.stringify(manifest));

  await assert.rejects(
    deployStack({ configDir, skiffRoot, shell }),
    /router is missing from .*manifest\.json/,
  );
  assert.equal(calls.length, 0);
});

test('deploy requires a build manifest and rejects missing binaries before remote commands', async (t) => {
  const { configDir, shell, calls, buildRoot } = await deployFixture(t);
  await rm(join(buildRoot, 'bin', 'skiff-runtime'), { force: true });

  await assert.rejects(
    deployStack({ configDir, skiffRoot, shell }),
    /runtime binary does not exist/,
  );
  assert.equal(calls.length, 0);
});

test('ecosystem template targets the copied config files and node bin', () => {
  const text = renderEcosystemConfig({
    remoteSkiff: '/srv/skiff',
    nodeBin: '/opt/node/bin',
  });
  assert.match(text, /script: '\/srv\/skiff\/bin\/skiff-router'/);
  assert.match(text, /args: '\/srv\/skiff\/config\/router\.yml'/);
  assert.match(text, /args: '--config \/srv\/skiff\/config\/telemetry\.yml'/);
  assert.match(text, /interpreter: NODE_BIN \+ '\/node'/);
  assert.doesNotMatch(text, /mongoUrl|httpMaxRequestBytes|prepareTimeoutMs/);
});

async function deployFixture(t, {
  router = undefined,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-deploy-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const configDir = join(root, 'configDir');
  const buildRoot = join(root, 'build');
  await mkdir(join(buildRoot, 'bin'), { recursive: true });
  await mkdir(configDir, { recursive: true });

  await writeFile(join(configDir, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    `buildRoot: ${buildRoot}`,
    `cargoTargetDir: ${join(root, 'cargo-target')}`,
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'config.yml'), [
    'profile: prod',
    'remote:',
    '  host: deploy.test',
    '  remoteSkiff: /srv/skiff',
    '  nodeBin: /opt/node/bin',
    'verify:',
    '  httpPort: 4000',
    '  controlPort: 4001',
    '  telemetryPort: 4002',
    '  healthPath: /__router/health',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'router.yml'), router ?? [
    'profile: prod',
    'host: 127.0.0.1',
    'artifactsPath: /srv/skiff/artifacts',
    'serviceDb:',
    '  mongoUrl: mongodb://127.0.0.1:27017',
    '',
  ].join('\n'));
  await writeFile(
    join(configDir, 'runtime.yml'),
    'router: ws://127.0.0.1:4001/runtime\nruntime-home: /srv/skiff/runtime-home\n',
  );
  await writeFile(
    join(configDir, 'telemetry.yml'),
    'telemetry:\n  host: 127.0.0.1\n  port: 4002\n  path: /telemetry\n',
  );

  for (const name of ['skiff-router', 'skiff-runtime', 'skiff-compiler']) {
    await writeFile(join(buildRoot, 'bin', name), `fake-${name}\n`);
  }
  await writeFile(join(buildRoot, 'manifest.json'), JSON.stringify({
    schemaVersion: 'skiff-runtime-stack-build-v1',
    target: 'x86_64-unknown-linux-gnu',
    commit: 'test-commit',
    units: {
      router: binaryUnit(join(buildRoot, 'bin', 'skiff-router')),
      runtime: binaryUnit(join(buildRoot, 'bin', 'skiff-runtime')),
      compiler: binaryUnit(join(buildRoot, 'bin', 'skiff-compiler')),
      telemetry: {
        kind: 'ts',
        commit: 'test-commit',
        sourceKey: 'test-telemetry',
        artifacts: [],
      },
    },
  }, null, 2));

  const calls = [];
  const shell = {
    remoteRun: async (host, command) => {
      calls.push({ op: 'ssh', host, command });
    },
    rsync: async (source, destination, extra = []) => {
      calls.push({ op: 'rsync', source, destination, extra });
    },
  };
  return { configDir, buildRoot, shell, calls, root };
}

function binaryUnit(pathValue) {
  return {
    kind: 'rs',
    commit: 'test-commit',
    sourceKey: 'test-source',
    artifacts: [{ kind: 'binary', path: pathValue }],
  };
}
