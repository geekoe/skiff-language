import assert from 'node:assert/strict';
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { loadStackConfig } from '../lib/stack-config.mjs';
import { parseStackYaml } from '../lib/stack-config.mjs';
import {
  generateLocalInstanceSpec,
  localInstanceSpecFrom,
} from '../lib/stack-instance-spec.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('local instance spec derives processes, ports, and shared facts from configDir + manifest', async (t) => {
  const fixture = await buildFixture(t);
  const stack = await loadStackConfig(fixture.configDir, {
    skiffRoot,
    files: ['build.yml', 'config.yml', 'router.yml', 'runtime.yml'],
  });
  const manifest = {
    profile: 'debug',
    units: {
      router: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-router' }] },
      runtime: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-runtime' }] },
      compiler: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-compiler' }] },
    },
  };
  const spec = localInstanceSpecFrom({ stack, skiffRoot, manifest });

  assert.equal(spec.schemaVersion, 'skiff-instance-v1');
  assert.equal(spec.profile, 'dev');
  assert.equal(spec.compilerBinary, resolve(skiffRoot, 'build/runtime-stack/bin/skiff-compiler'));
  assert.deepEqual(
    spec.processes.map((process) => process.name),
    ['mongo', 'router', 'runtime'],
  );
  const mongo = spec.processes.find((process) => process.name === 'mongo');
  assert.equal(mongo.args[mongo.args.indexOf('--port') + 1], '27017');
  const router = spec.processes.find((process) => process.name === 'router');
  assert.deepEqual(router.ports, [4100, 4101]);
  assert.equal(router.healthUrl, 'http://127.0.0.1:4101/__router/health');
});

test('local instance spec supervises watch when process.watch is managed', async (t) => {
  const fixture = await buildFixture(t, { watch: true });
  const stack = await loadStackConfig(fixture.configDir, {
    skiffRoot,
    files: ['build.yml', 'config.yml', 'router.yml', 'runtime.yml'],
  });
  const manifest = {
    profile: 'debug',
    units: {
      router: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-router' }] },
      runtime: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-runtime' }] },
      compiler: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-compiler' }] },
    },
  };
  const spec = localInstanceSpecFrom({ stack, skiffRoot, manifest });

  const watch = spec.processes.find((process) => process.name === 'watch');
  assert.ok(watch, 'watch process is present');
  assert.equal(watch.command, 'node');
  assert.ok(
    watch.args.some((arg) => arg.endsWith('skiff-watch.mjs')),
    'watch args reference skiff-watch.mjs',
  );
  assert.ok(
    watch.args.includes('--config') && watch.args.includes(join(stack.configDir, 'watch')),
    'watch args point at the configDir watch directory',
  );
  assert.deepEqual(watch.ports, []);
  assert.equal(watch.healthUrl, null);
});

test('generateLocalInstanceSpec writes instance.yml and devHome directories', async (t) => {
  const fixture = await buildFixture(t);
  const stack = await loadStackConfig(fixture.configDir, {
    skiffRoot,
    files: ['build.yml', 'config.yml', 'router.yml', 'runtime.yml'],
  });
  const manifestPath = join(stack.paths.buildRoot, 'manifest.json');
  await writeFile(manifestPath, JSON.stringify({
    profile: 'debug',
    units: {
      router: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-router' }] },
      runtime: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-runtime' }] },
      compiler: { artifacts: [{ kind: 'binary', path: 'build/runtime-stack/bin/skiff-compiler' }] },
    },
  }));
  const spec = await generateLocalInstanceSpec({ stack, skiffRoot });
  const written = parseStackYaml(
    await readFile(join(stack.paths.buildRoot, 'instance.yml'), 'utf8'),
    'instance.yml',
  );
  assert.equal(written.schemaVersion, 'skiff-instance-v1');
  for (const name of ['artifacts', 'runtime-home', 'secrets', 'pids', 'logs', 'mongo-data']) {
    await access(join(spec.devHome, name));
  }
});

async function buildFixture(t, { watch = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-spec-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const configDir = join(root, 'configDir');
  const buildRoot = join(root, 'build', 'runtime-stack');
  await mkdir(configDir, { recursive: true });
  await mkdir(join(buildRoot, 'bin'), { recursive: true });
  await writeFile(join(configDir, 'build.yml'), [
    'target: aarch64-apple-darwin',
    'zigDir: /cache/zig',
    `buildRoot: ${buildRoot}`,
    'cargoTargetDir: build/cargo-target',
    'profile: debug',
    'process:',
    '  mongo: managed',
    '  telemetry: disabled',
    `  watch: ${watch ? 'managed' : 'disabled'}`,
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'config.yml'), [
    'profile: dev',
    'remote:',
    '  host: root@example.test',
    '  remoteSkiff: /srv/skiff',
    '  nodeBin: /opt/node/bin',
    'verify:',
    '  httpPort: 4100',
    '  controlPort: 4101',
    '  telemetryPort: 4102',
    '  healthPath: /__router/health',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'router.yml'), [
    'profile: dev',
    'host: 127.0.0.1',
    'http:',
    '  port: 4100',
    'runtime:',
    '  port: 4101',
    'serviceDb:',
    '  mongoUrl: mongodb://127.0.0.1:27017/?directConnection=true',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'runtime.yml'), [
    'router: ws://127.0.0.1:4101/runtime',
    'runtime-home: /tmp/runtime-home',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'telemetry.yml'), [
    'telemetry:',
    '  host: 127.0.0.1',
    '  port: 4102',
    '  path: /telemetry',
    '  memory: true',
    '',
  ].join('\n'));
  return { configDir, root };
}
