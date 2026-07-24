import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import {
  renderRouterConfig,
  renderRuntimeConfig,
} from '../lib/runtime-stack-config.mjs';

const execFileAsync = promisify(execFile);

const routerConfig = {
  profile: 'dev',
  host: '127.0.0.1',
  environment: 'f04-host-test',
  artifactsPath: '/tmp/skiff/artifacts',
  ecosystemStoreCliPath: '/tmp/skiff/bin/skiff-compiler',
  identityCliPath: '/tmp/skiff/bin/artifact-identity',
  devReload: true,
  releaseMode: false,
  httpPort: 4100,
  runtimePort: 4101,
  serviceDbMongoUrl: 'mongodb://127.0.0.1:27017/skiff',
};

const runtimeConfig = {
  routerUrl: 'ws://127.0.0.1:4101/runtime',
  runtimeHome: '/tmp/skiff/runtime-home',
  environment: 'f10-runtime-test',
};

test('router config renders an explicit environment', () => {
  const rendered = renderRouterConfig(routerConfig);

  assert.match(rendered, /^environment: "f04-host-test"$/m);
  assert.match(
    rendered,
    /^ecosystemStoreCliPath: "\/tmp\/skiff\/bin\/skiff-compiler"$/m,
  );
  assert.equal(rendered.match(/^environment:/gm)?.length, 1);
  assert.match(rendered, /^artifactsPath: "\/tmp\/skiff\/artifacts"$/m);
  assert.match(rendered, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017\/skiff"$/m);
  assert.doesNotMatch(rendered, /^artifactRoots?:/m);
});

test('router config fails closed when environment is omitted or empty', () => {
  const { environment: _environment, ...withoutEnvironment } = routerConfig;
  assert.throws(
    () => renderRouterConfig(withoutEnvironment),
    /router environment is required/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, environment: '' }),
    /router environment is required/,
  );
});

test('router config fails closed when ecosystemStoreCliPath is omitted or empty', () => {
  const { ecosystemStoreCliPath: _ecosystemStoreCliPath, ...withoutStoreCli } = routerConfig;
  assert.throws(
    () => renderRouterConfig(withoutStoreCli),
    /router ecosystemStoreCliPath is required/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, ecosystemStoreCliPath: '   ' }),
    /router ecosystemStoreCliPath is required/,
  );
});

test('router config fails closed when artifact path or Mongo URL is omitted', () => {
  const { artifactsPath: _artifactsPath, ...withoutArtifactsPath } = routerConfig;
  const { serviceDbMongoUrl: _serviceDbMongoUrl, ...withoutMongoUrl } = routerConfig;
  assert.throws(
    () => renderRouterConfig(withoutArtifactsPath),
    /router artifactsPath must be an absolute path/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, artifactsPath: '   ' }),
    /router artifactsPath must be an absolute path/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, artifactsPath: 'relative/artifacts' }),
    /router artifactsPath must be an absolute path/,
  );
  assert.throws(
    () => renderRouterConfig(withoutMongoUrl),
    /router serviceDb\.mongoUrl is required/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, serviceDbMongoUrl: '' }),
    /router serviceDb\.mongoUrl is required/,
  );
});

test('runtime config renders one exact environment without deployment bootstrap ownership', () => {
  const rendered = renderRuntimeConfig(runtimeConfig);

  assert.match(rendered, /^environment: "f10-runtime-test"$/m);
  assert.equal(rendered.match(/^environment:/gm)?.length, 1);
  assert.doesNotMatch(rendered, /^artifactRoots?:/m);
  assert.doesNotMatch(rendered, /mongoUrl/);
});

test('runtime config fails closed on missing or empty environment', () => {
  const { environment: _environment, ...withoutEnvironment } = runtimeConfig;
  assert.throws(
    () => renderRuntimeConfig(withoutEnvironment),
    /runtime environment is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, environment: '' }),
    /runtime environment is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, environment: null }),
    /runtime environment is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, environment: '   ' }),
    /runtime environment is required/,
  );
});

test('local dev config writes bootstrap ownership only to router', async () => {
  const devHome = await mkdtemp(join(tmpdir(), 'skiff-f10-dev-config-'));
  try {
    await execFileAsync(process.execPath, [
      fileURLToPath(new URL('../skiff.mjs', import.meta.url)),
      'dev',
      'init',
      '--dev-home',
      devHome,
      '--no-bin',
    ]);
    const rendered = await readFile(join(devHome, 'runtime.yml'), 'utf8');
    const router = await readFile(join(devHome, 'router.yml'), 'utf8');

    assert.match(rendered, /^environment: "dev"$/m);
    assert.doesNotMatch(rendered, /^artifactRoots?:/m);
    assert.doesNotMatch(rendered, /mongoUrl/);
    assert.match(router, new RegExp(`^artifactsPath: ${JSON.stringify(join(devHome, 'artifacts'))}$`, 'm'));
    assert.match(router, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017\//m);
    assert.doesNotMatch(router, /^artifactRoots?:/m);
    assert.match(
      router,
      new RegExp(`^ecosystemStoreCliPath: ${JSON.stringify(join(devHome, 'bin', process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler'))}$`, 'm'),
    );
    assert.doesNotMatch(router, /^rewrite:/m);
  } finally {
    await rm(devHome, { recursive: true, force: true });
  }
});
