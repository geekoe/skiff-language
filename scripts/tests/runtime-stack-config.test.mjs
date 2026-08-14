import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import {
  DEFAULT_GENERATED_ROUTER_RUNTIME_MAX_CONCURRENCY,
  renderRouterConfig,
  renderRuntimeConfig,
} from '../lib/runtime-stack-config.mjs';

const execFileAsync = promisify(execFile);

const routerConfig = {
  profile: 'dev',
  host: '127.0.0.1',
  artifactsPath: '/tmp/skiff/artifacts',
  devReload: true,
  releaseMode: false,
  httpPort: 4100,
  httpMaxRequestBytes: 67108864,
  httpMaxResponseBytes: 8388608,
  runtimePort: 4101,
  runtimeMaxConcurrency: 17,
  serviceDbMongoUrl: 'mongodb://127.0.0.1:27017/skiff',
};

const runtimeConfig = {
  routerUrl: 'ws://127.0.0.1:4101/runtime',
  runtimeHome: '/tmp/skiff/runtime-home',
};

test('router config renders an explicit profile and no environment', () => {
  const rendered = renderRouterConfig(routerConfig);

  assert.match(rendered, /^profile: dev$/m);
  assert.equal(rendered.match(/^profile:/gm)?.length, 1);
  assert.doesNotMatch(rendered, /^environment:/m);
  assert.doesNotMatch(rendered, /^ecosystemStoreCliPath:/m);
  assert.match(rendered, /^artifactsPath: "\/tmp\/skiff\/artifacts"$/m);
  assert.match(rendered, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017\/skiff"$/m);
  assert.match(rendered, /^  maxRequestBytes: 67108864$/m);
  assert.match(rendered, /^  maxResponseBytes: 8388608$/m);
  assert.doesNotMatch(rendered, /^activation:/m);
  assert.match(rendered, /^runtime:\n  port: 4101\n  path: \/runtime\n  maxConcurrency: 17$/m);
  assert.doesNotMatch(rendered, /idleTimeoutMs/);
  assert.doesNotMatch(rendered, /bodyLimitBytes/);
  assert.doesNotMatch(rendered, /^artifactRoots?:/m);
});

test('router config generator owns one default runtime concurrency and always emits it', () => {
  const { runtimeMaxConcurrency: _value, ...withoutConcurrency } = routerConfig;
  const rendered = renderRouterConfig(withoutConcurrency);

  assert.equal(DEFAULT_GENERATED_ROUTER_RUNTIME_MAX_CONCURRENCY, 128);
  assert.match(
    rendered,
    new RegExp(`^  maxConcurrency: ${DEFAULT_GENERATED_ROUTER_RUNTIME_MAX_CONCURRENCY}$`, 'm'),
  );
});

test('router config rejects invalid explicit runtime concurrency', () => {
  for (const value of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '128']) {
    assert.throws(
      () => renderRouterConfig({ ...routerConfig, runtimeMaxConcurrency: value }),
      /router runtime\.maxConcurrency must be a positive safe integer/,
    );
  }
});

test('router config requires explicit positive safe HTTP byte ceilings', () => {
  for (const key of ['httpMaxRequestBytes', 'httpMaxResponseBytes']) {
    const { [key]: _value, ...missing } = routerConfig;
    assert.throws(
      () => renderRouterConfig(missing),
      new RegExp(`router http\\.${key === 'httpMaxRequestBytes' ? 'maxRequestBytes' : 'maxResponseBytes'} must be a positive safe integer`),
    );
    for (const value of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '8388608']) {
      assert.throws(
        () => renderRouterConfig({ ...routerConfig, [key]: value }),
        /must be a positive safe integer/,
      );
    }
  }
});

test('router config fails closed when profile is omitted or empty', () => {
  const { profile: _profile, ...withoutProfile } = routerConfig;
  assert.throws(
    () => renderRouterConfig(withoutProfile),
    /router profile is required/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, profile: '' }),
    /router profile is required/,
  );
});

test('router config fails closed when the retired ecosystemStoreCliPath is still passed', () => {
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, ecosystemStoreCliPath: '/tmp/skiff/bin/skiff-compiler' }),
    /router config ecosystemStoreCliPath is not supported/,
  );
});

test('router config rejects invalid profile, host, ports, and runtime path', () => {
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, profile: '..' }),
    /router profile must be a canonical ASCII token/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, host: '   ' }),
    /router host must be a non-empty string/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, httpPort: 70000 }),
    /router http\.port must be a TCP port/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, runtimePort: 0 }),
    /router runtime\.port must be a TCP port/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, runtimePath: 'runtime' }),
    /router runtime\.path must start with \//,
  );
});

test('router config rejects invalid profile tokens', () => {
  for (const profile of ['bad env!', '..', 'a'.repeat(201)]) {
    assert.throws(
      () => renderRouterConfig({ ...routerConfig, profile }),
      /router profile must be a canonical ASCII token/,
    );
  }
});

test('router config validates request timeout, booleans, and telemetry endpoint', () => {
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, requestTimeoutMs: 0 }),
    /router requestTimeoutMs must be a positive safe integer/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, devReload: 'true' }),
    /router devReload must be a boolean/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, releaseMode: 1 }),
    /router releaseMode must be a boolean/,
  );
  for (const telemetryEndpoint of [undefined, '', '   ']) {
    const rendered = renderRouterConfig({ ...routerConfig, telemetryEndpoint });
    assert.doesNotMatch(rendered, /^telemetry:/m);
  }
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, telemetryEndpoint: 42 }),
    /router telemetry\.endpoint must be a non-empty string/,
  );
});

test('router config validates rewrite items against the frozen schema', () => {
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, rewrite: 'no' }),
    /router rewrite must be an array/,
  );
  assert.throws(
    () => renderRouterConfig({
      ...routerConfig,
      rewrite: [{ service: 'skiff.run/account' }],
    }),
    /router rewrite\[0\]\.host must be a non-empty string/,
  );
  assert.throws(
    () => renderRouterConfig({
      ...routerConfig,
      rewrite: [{ host: 'a.localhost', path: 'api', service: 'skiff.run/account' }],
    }),
    /router rewrite\[0\]\.path must start with \//,
  );
});

test('router config renderer emits only the frozen schema keys', async () => {
  const { parseDocument } = await import('yaml');
  const rendered = renderRouterConfig(routerConfig);
  const document = parseDocument(rendered);
  assert.equal(document.errors.length, 0);
  const value = document.toJS();
  assert.deepEqual(
    Object.keys(value).sort(),
    [
      'artifactsPath',
      'devReload',
      'host',
      'http',
      'profile',
      'releaseMode',
      'requestTimeoutMs',
      'runtime',
      'serviceDb',
    ].sort(),
  );
  assert.deepEqual(
    Object.keys(value.http).sort(),
    ['maxRequestBytes', 'maxResponseBytes', 'port'],
  );
  assert.deepEqual(
    Object.keys(value.runtime).sort(),
    ['maxConcurrency', 'path', 'port'],
  );
  assert.deepEqual(Object.keys(value.serviceDb), ['mongoUrl']);
  assert.equal(Object.hasOwn(value, 'ecosystemStoreCliPath'), false);
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

test('runtime config renders local facts without any activation identifier', () => {
  const rendered = renderRuntimeConfig(runtimeConfig);

  assert.doesNotMatch(rendered, /^(?:profile|environment):/m);
  assert.doesNotMatch(rendered, /^artifactRoots?:/m);
  assert.doesNotMatch(rendered, /mongoUrl/);
  assert.doesNotMatch(rendered, /maxConcurrency|idleTimeoutMs/);
});

test('runtime config renders and validates an explicit operator-owned HTTP egress proxy', () => {
  const rendered = renderRuntimeConfig({
    ...runtimeConfig,
    httpEgressProxy: 'http://127.0.0.1:4321',
  });

  assert.match(rendered, /^http:\n  egress:\n    proxy: "http:\/\/127\.0\.0\.1:4321"$/m);
  for (const httpEgressProxy of ['', '   ', '127.0.0.1:4321', 'socks5://127.0.0.1:4321', 4321]) {
    assert.throws(
      () => renderRuntimeConfig({ ...runtimeConfig, httpEgressProxy }),
      /runtime http\.egress\.proxy must be/,
    );
  }
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
      '--http-max-request-bytes',
      '67108864',
      '--http-max-response-bytes',
      '8388608',
      '--no-bin',
    ]);
    const rendered = await readFile(join(devHome, 'runtime.yml'), 'utf8');
    const router = await readFile(join(devHome, 'router.yml'), 'utf8');

    assert.doesNotMatch(rendered, /^(?:profile|environment):/m);
    assert.doesNotMatch(rendered, /^artifactRoots?:/m);
    assert.doesNotMatch(rendered, /mongoUrl/);
    assert.doesNotMatch(rendered, /maxConcurrency|idleTimeoutMs/);
    assert.match(router, /^profile: dev$/m);
    assert.doesNotMatch(router, /^environment:/m);
    assert.match(router, new RegExp(`^artifactsPath: ${JSON.stringify(join(devHome, 'artifacts'))}$`, 'm'));
    assert.match(router, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017\//m);
    assert.match(router, /^  maxRequestBytes: 67108864$/m);
    assert.match(router, /^  maxResponseBytes: 8388608$/m);
    assert.doesNotMatch(router, /^activation:/m);
    assert.match(router, /^runtime:\n  port: 4001\n  path: \/runtime\n  maxConcurrency: 128$/m);
    assert.doesNotMatch(router, /idleTimeoutMs/);
    assert.doesNotMatch(router, /bodyLimitBytes/);
    assert.doesNotMatch(router, /^artifactRoots?:/m);
    assert.doesNotMatch(router, /^ecosystemStoreCliPath:/m);
    assert.doesNotMatch(router, /^rewrite:/m);
  } finally {
    await rm(devHome, { recursive: true, force: true });
  }
});

test('local dev init requires explicit positive HTTP byte ceilings', async () => {
  const devHome = await mkdtemp(join(tmpdir(), 'skiff-f135-dev-config-'));
  try {
    await assert.rejects(
      execFileAsync(process.execPath, [
        fileURLToPath(new URL('../skiff.mjs', import.meta.url)),
        'dev',
        'init',
        '--dev-home',
        devHome,
        '--no-bin',
      ]),
      /--http-max-request-bytes must be a positive safe integer/,
    );
    await assert.rejects(
      execFileAsync(process.execPath, [
        fileURLToPath(new URL('../skiff.mjs', import.meta.url)),
        'dev',
        'init',
        '--dev-home',
        devHome,
        '--http-max-request-bytes',
        '67108864',
        '--http-max-response-bytes',
        '0',
        '--no-bin',
      ]),
      /--http-max-response-bytes must be a positive safe integer/,
    );
  } finally {
    await rm(devHome, { recursive: true, force: true });
  }
});
