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
  artifactRoots: ['/tmp/skiff/artifacts'],
  identityCliPath: '/tmp/skiff/bin/artifact-identity',
  devReload: true,
  releaseMode: false,
  httpPort: 4100,
  runtimePort: 4101,
};

const runtimeConfig = {
  routerUrl: 'ws://127.0.0.1:4101/runtime',
  runtimeHome: '/tmp/skiff/runtime-home',
  environment: 'f10-runtime-test',
  artifactRoot: '/tmp/skiff/artifacts',
};

test('router config renders an explicit environment', () => {
  const rendered = renderRouterConfig(routerConfig);

  assert.match(rendered, /^environment: "f04-host-test"$/m);
  assert.equal(rendered.match(/^environment:/gm)?.length, 1);
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

test('runtime config renders one exact environment and singular artifactRoot', () => {
  const rendered = renderRuntimeConfig(runtimeConfig);

  assert.match(rendered, /^environment: "f10-runtime-test"$/m);
  assert.match(rendered, /^artifactRoot: "\/tmp\/skiff\/artifacts"$/m);
  assert.equal(rendered.match(/^environment:/gm)?.length, 1);
  assert.equal(rendered.match(/^artifactRoot:/gm)?.length, 1);
  assert.doesNotMatch(rendered, /^artifactRoots:/m);
});

test('runtime config fails closed on missing or empty bootstrap fields', () => {
  const { environment: _environment, ...withoutEnvironment } = runtimeConfig;
  const { artifactRoot: _artifactRoot, ...withoutArtifactRoot } = runtimeConfig;
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
  assert.throws(
    () => renderRuntimeConfig(withoutArtifactRoot),
    /runtime artifactRoot is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, artifactRoot: '' }),
    /runtime artifactRoot is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, artifactRoot: null }),
    /runtime artifactRoot is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({ ...runtimeConfig, artifactRoot: '   ' }),
    /runtime artifactRoot is required/,
  );
  assert.throws(
    () => renderRuntimeConfig({
      ...withoutArtifactRoot,
      artifactRoots: ['/tmp/a', '/tmp/b'],
    }),
    /runtime artifactRoot is required/,
  );
});

test('local dev config caller writes dev and one canonical artifact root', async () => {
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

    assert.match(rendered, /^environment: "dev"$/m);
    assert.match(rendered, new RegExp(`^artifactRoot: ${JSON.stringify(join(devHome, 'artifacts'))}$`, 'm'));
    assert.doesNotMatch(rendered, /^artifactRoots:/m);
  } finally {
    await rm(devHome, { recursive: true, force: true });
  }
});
