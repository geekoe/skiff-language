import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { TELEMETRY_PROTOCOL, TELEMETRY_TOPICS } from '../src/protocol/envelope.js';
import { loadRouterConfig } from '../src/router/config.js';

const tempDirs: string[] = [];
const originalIdentityCliEnv = process.env.SKIFF_ARTIFACT_IDENTITY_CLI;
const originalDevHomeEnv = process.env.SKIFF_DEV_HOME;

beforeEach(() => {
  delete process.env.SKIFF_ARTIFACT_IDENTITY_CLI;
  delete process.env.SKIFF_DEV_HOME;
});

afterEach(async () => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) {
      await rm(dir, { recursive: true, force: true });
    }
  }
  restoreEnv('SKIFF_ARTIFACT_IDENTITY_CLI', originalIdentityCliEnv);
  restoreEnv('SKIFF_DEV_HOME', originalDevHomeEnv);
});

describe('router config', () => {
  it('loads router.yml values and resolves manifest relative to the config file', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'host: 0.0.0.0',
        'artifactRoots:',
        '  - ../var/skiff-artifacts',
        'releaseMode: true',
        'manifest: manifests/router-manifest.json',
        'requestTimeoutMs: 7000',
        'http:',
        '  port: 5010',
        '  bodyLimitBytes: 16777216',
        'runtime:',
        '  port: 5011',
        '  path: /runtime-dev',
        'fileBackend:',
        '  local:',
        '    root: ../var/skiff-file-blobs',
        '  oss:',
        '    endpoint: https://oss-cn-hangzhou.aliyuncs.com',
        '    bucket: skiff-dev-files',
        '    region: cn-hangzhou',
        '    accessKeyIdEnv: SKIFF_OSS_ACCESS_KEY_ID',
        '    accessKeySecretEnv: SKIFF_OSS_ACCESS_KEY_SECRET',
        'websocket:',
        '  path: /socket',
        'rewrite:',
        '  - host: Account.Localhost:4000.',
        '    path: /api',
        '    service: skiff.run/account',
        '    version: 0.1.0',
        '  - host: registry.localhost',
        '    service: skiff.run/registry',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toEqual({
      artifactRoots: [join(dir, '..', 'var/skiff-artifacts')],
      host: '0.0.0.0',
      httpBodyLimitBytes: 16777216,
      httpPort: 5010,
      manifests: [join(dir, 'manifests/router-manifest.json')],
      profile: 'dev',
      releaseMode: true,
      requestTimeoutMs: 7000,
      fileBackend: {
        local: {
          root: join(dir, '..', 'var/skiff-file-blobs'),
        },
        oss: {
          endpoint: 'https://oss-cn-hangzhou.aliyuncs.com',
          bucket: 'skiff-dev-files',
          region: 'cn-hangzhou',
          accessKeyIdEnv: 'SKIFF_OSS_ACCESS_KEY_ID',
          accessKeySecretEnv: 'SKIFF_OSS_ACCESS_KEY_SECRET',
        },
      },
      rewrite: [
        {
          host: 'account.localhost',
          path: '/api',
          service: 'skiff.run/account',
          version: '0.1.0',
        },
        {
          host: 'registry.localhost',
          service: 'skiff.run/registry',
        },
      ],
      runtimePath: '/runtime-dev',
      runtimePort: 5011,
      websocketPath: '/socket',
    });
  });

  it('allows command line overrides on top of router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, ['profile: base', 'manifest: base.json', ''].join('\n'));

    await expect(
      loadRouterConfig(configPath, {
        host: '127.0.0.2',
        artifactRoots: ['artifact-override'],
        httpBodyLimitBytes: '33554432',
        httpPort: '6010',
        manifest: 'override.json',
        requestTimeoutMs: '9000',
        runtimePath: '/override-runtime',
        runtimePort: '6011',
        websocketPath: '/override-ws',
        profile: 'prod',
        releaseMode: true,
      })
    ).resolves.toMatchObject({
      artifactRoots: [join(dir, 'artifact-override')],
      host: '127.0.0.2',
      httpBodyLimitBytes: 33554432,
      httpPort: 6010,
      manifests: [join(dir, 'override.json')],
      profile: 'prod',
      releaseMode: true,
      requestTimeoutMs: 9000,
      runtimePath: '/override-runtime',
      runtimePort: 6011,
      websocketPath: '/override-ws',
    });
  });

  it('loads router profile from top-level profile and allows overrides', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, 'profile: staging\n');

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      profile: 'staging',
    });

    await expect(
      loadRouterConfig(configPath, {
        profile: 'prod',
      })
    ).resolves.toMatchObject({
      profile: 'prod',
    });
  });

  it('loads dev reload with command line overrides', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      ['profile: dev', 'devReload: true', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      devReload: true,
    });

    await expect(
      loadRouterConfig(configPath, {
        devReload: false,
      })
    ).resolves.toMatchObject({
      devReload: false,
    });
  });

  it('resolves identity CLI path from config, override, env, and dev fallback', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      ['profile: dev', 'identityCliPath: bin/skiff-artifact-identity', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      identityCliPath: join(dir, 'bin/skiff-artifact-identity'),
    });

    await expect(
      loadRouterConfig(configPath, {
        identityCliPath: 'override/skiff-artifact-identity',
      })
    ).resolves.toMatchObject({
      identityCliPath: resolve('override/skiff-artifact-identity'),
    });

    await writeFile(configPath, ['profile: dev', ''].join('\n'));
    process.env.SKIFF_ARTIFACT_IDENTITY_CLI = 'env/skiff-artifact-identity';
    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      identityCliPath: resolve('env/skiff-artifact-identity'),
    });

    delete process.env.SKIFF_ARTIFACT_IDENTITY_CLI;
    const devHome = join(dir, 'dev-home');
    process.env.SKIFF_DEV_HOME = devHome;
    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      identityCliPath: join(devHome, 'bin/skiff-artifact-identity'),
    });
  });

  it('does not use local dev identity CLI fallback in release mode', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      ['profile: prod', 'releaseMode: true', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.not.toHaveProperty('identityCliPath');
  });

  it('loads telemetry config with router-owned defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      telemetry: {
        endpoint: 'ws://127.0.0.1:4002/telemetry',
        protocol: TELEMETRY_PROTOCOL,
        topics: [...TELEMETRY_TOPICS],
        queueMaxEvents: 10000,
        batchMaxEvents: 200,
        batchMaxBytes: 262144,
        flushIntervalMs: 1000,
        enabled: true,
      },
    });
  });

  it('loads serviceDb Mongo URL for runtime activation', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'serviceDb:',
        '  mongoUrl: mongodb://127.0.0.1:27017/?directConnection=true',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      serviceDb: {
        mongoUrl: 'mongodb://127.0.0.1:27017/?directConnection=true',
      },
    });
  });

  it('loads OSS file backend credentials from env references or direct values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const envConfig = join(dir, 'file-env.yml');
    await writeFile(
      envConfig,
      [
        'profile: dev',
        'fileBackend:',
        '  oss:',
        '    endpoint: https://oss-cn-hangzhou.aliyuncs.com',
        '    bucket: skiff-files',
        '    accessKeyIdEnv: SKIFF_OSS_ACCESS_KEY_ID',
        '    accessKeySecretEnv: SKIFF_OSS_ACCESS_KEY_SECRET',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(envConfig)).resolves.toMatchObject({
      fileBackend: {
        oss: {
          endpoint: 'https://oss-cn-hangzhou.aliyuncs.com',
          bucket: 'skiff-files',
          accessKeyIdEnv: 'SKIFF_OSS_ACCESS_KEY_ID',
          accessKeySecretEnv: 'SKIFF_OSS_ACCESS_KEY_SECRET',
        },
      },
    });

    const directConfig = join(dir, 'file-direct.yml');
    await writeFile(
      directConfig,
      [
        'profile: dev',
        'fileBackend:',
        '  oss:',
        '    endpoint: https://oss-cn-hangzhou.aliyuncs.com',
        '    bucket: skiff-files',
        '    accessKeyId: local-only-id',
        '    accessKeySecret: local-only-secret',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(directConfig)).resolves.toMatchObject({
      fileBackend: {
        oss: {
          accessKeyId: 'local-only-id',
          accessKeySecret: 'local-only-secret',
        },
      },
    });
  });

  it('rejects incomplete file backend config', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const emptyConfig = join(dir, 'file-empty.yml');
    await writeFile(emptyConfig, ['profile: dev', 'fileBackend: {}', ''].join('\n'));
    await expect(loadRouterConfig(emptyConfig)).rejects.toThrow(
      /router config fileBackend must configure local or oss/
    );

    const missingCredential = join(dir, 'file-missing-credential.yml');
    await writeFile(
      missingCredential,
      [
        'profile: dev',
        'fileBackend:',
        '  oss:',
        '    endpoint: https://oss-cn-hangzhou.aliyuncs.com',
        '    bucket: skiff-files',
        '    accessKeyIdEnv: SKIFF_OSS_ACCESS_KEY_ID',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(missingCredential)).rejects.toThrow(
      /router config fileBackend\.oss requires accessKeySecretEnv or accessKeySecret/
    );
  });

  it('rejects serviceDb storage namespace config values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'serviceDb:',
        '  mongoUrl: mongodb://127.0.0.1:27017/?directConnection=true',
        '  storageNamespace: billing',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config serviceDb\.storageNamespace is no longer supported/
    );
  });

  it('rejects invalid rewrite config values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const missingService = join(dir, 'missing-service.yml');
    await writeFile(
      missingService,
      [
        'profile: dev',
        'rewrite:',
        '  - host: account.localhost',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(missingService)).rejects.toThrow(
      /router config rewrite\[0\]\.service is required/
    );

    const invalidPath = join(dir, 'invalid-path.yml');
    await writeFile(
      invalidPath,
      [
        'profile: dev',
        'rewrite:',
        '  - host: account.localhost',
        '    path: api',
        '    service: skiff.run/account',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidPath)).rejects.toThrow(
      /router config rewrite\[0\]\.path must start with \//
    );

    const invalidService = join(dir, 'invalid-service.yml');
    await writeFile(
      invalidService,
      [
        'profile: dev',
        'rewrite:',
        '  - host: account.localhost',
        '    service: NotAService',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidService)).rejects.toThrow(
      /router config rewrite\[0\]\.service must be a valid publication id/
    );

    const invalidVersion = join(dir, 'invalid-version.yml');
    await writeFile(
      invalidVersion,
      [
        'profile: dev',
        'rewrite:',
        '  - host: account.localhost',
        '    service: skiff.run/account',
        '    version: not valid',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidVersion)).rejects.toThrow(
      /router config rewrite\[0\]\.version must be a valid version/
    );

    const unknownField = join(dir, 'unknown-field.yml');
    await writeFile(
      unknownField,
      [
        'profile: dev',
        'rewrite:',
        '  - host: account.localhost',
        '    service: skiff.run/account',
        '    headers:',
        '      x-test: value',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(unknownField)).rejects.toThrow(
      /router config rewrite\[0\]\.headers is not supported/
    );

    const duplicate = join(dir, 'duplicate.yml');
    await writeFile(
      duplicate,
      [
        'profile: dev',
        'rewrite:',
        '  - host: Account.Localhost:4000',
        '    path: /api',
        '    service: skiff.run/account',
        '  - host: account.localhost',
        '    path: /api',
        '    service: skiff.run/registry',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(duplicate)).rejects.toThrow(
      /duplicate router rewrite rule for host account\.localhost path \/api/
    );
  });

  it('omits telemetry when disabled or endpoint is not configured', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const disabledConfig = join(dir, 'disabled.yml');
    await writeFile(
      disabledConfig,
      [
        'profile: dev',
        'telemetry:',
        '  enabled: false',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(disabledConfig)).resolves.not.toHaveProperty('telemetry');

    const noEndpointConfig = join(dir, 'no-endpoint.yml');
    await writeFile(noEndpointConfig, ['profile: dev', 'telemetry:', '  enabled: true', ''].join('\n'));
    await expect(loadRouterConfig(noEndpointConfig)).resolves.not.toHaveProperty('telemetry');
  });

  it('rejects invalid telemetry config values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const invalidProtocol = join(dir, 'invalid-protocol.yml');
    await writeFile(
      invalidProtocol,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '  protocol: skiff-telemetry-v2',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidProtocol)).rejects.toThrow(
      /router config telemetry\.protocol must be skiff-telemetry-v1/
    );

    const duplicateTopic = join(dir, 'duplicate-topic.yml');
    await writeFile(
      duplicateTopic,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '  topics: [log, log]',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(duplicateTopic)).rejects.toThrow(
      /router config telemetry\.topics must not contain duplicates/
    );

    const emptyTopics = join(dir, 'empty-topics.yml');
    await writeFile(
      emptyTopics,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '  topics: []',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(emptyTopics)).rejects.toThrow(
      /router config telemetry\.topics must be a non-empty array/
    );

    const invalidTopic = join(dir, 'invalid-topic.yml');
    await writeFile(
      invalidTopic,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '  topics: [log, audit]',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidTopic)).rejects.toThrow(
      /router config telemetry\.topics\[1\] must be one of log, trace, metric, health, debug/
    );

    const invalidNumber = join(dir, 'invalid-number.yml');
    await writeFile(
      invalidNumber,
      [
        'profile: dev',
        'telemetry:',
        '  endpoint: ws://127.0.0.1:4002/telemetry',
        '  queueMaxEvents: 0',
        '',
      ].join('\n')
    );
    await expect(loadRouterConfig(invalidNumber)).rejects.toThrow(
      /router config telemetry\.queueMaxEvents must be a positive integer/
    );
  });

  it('rejects invalid dev reload values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const badConfig = join(dir, 'bad-dev-reload.yml');
    await writeFile(badConfig, ['profile: dev', 'devReload: latest', ''].join('\n'));
    await expect(loadRouterConfig(badConfig)).rejects.toThrow(
      /router config devReload must be a boolean/
    );
  });

  it('rejects invalid http body limit values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const zeroConfig = join(dir, 'zero-body-limit.yml');
    await writeFile(
      zeroConfig,
      ['profile: dev', 'http:', '  bodyLimitBytes: 0', ''].join('\n')
    );
    await expect(loadRouterConfig(zeroConfig)).rejects.toThrow(
      /router config http\.bodyLimitBytes must be a positive integer/
    );

    const fractionalConfig = join(dir, 'fractional-body-limit.yml');
    await writeFile(
      fractionalConfig,
      ['profile: dev', 'http:', '  bodyLimitBytes: 1.5', ''].join('\n')
    );
    await expect(loadRouterConfig(fractionalConfig)).rejects.toThrow(
      /router config http\.bodyLimitBytes must be a positive integer/
    );
  });

  it('requires top-level profile in router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, 'manifest: base.json\n');

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config profile is required/
    );
  });

  it('rejects values.profile in router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, ['values:', '  profile: prod', ''].join('\n'));

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config values\.profile is no longer supported/
    );
  });

  it('rejects profile names that cannot be used in config filenames', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, 'profile: prod-us\n');

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config profile must match \[A-Za-z_\]\[A-Za-z0-9_\]\*/
    );
  });

  it('loads multiple manifests for a shared router', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'manifests:',
        '  - manifests/websocket_fixture.json',
        '  - manifests/sample.json',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      manifests: [
        join(dir, 'manifests/websocket_fixture.json'),
        join(dir, 'manifests/sample.json'),
      ],
    });
  });

  it('rejects legacy single artifact root config fields', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    await mkdir(join(dir, 'artifacts'));
    const configPath = join(dir, 'router.yml');
    await writeFile(configPath, ['profile: dev', 'artifacts: artifacts', ''].join('\n'));

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config artifacts is no longer supported; use artifactRoots/
    );
  });

  it('loads ordered artifact roots from top-level config', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      [
        'profile: dev',
        'artifactRoots:',
        '  - artifacts/base',
        '  - artifacts/test',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      artifactRoots: [join(dir, 'artifacts/base'), join(dir, 'artifacts/test')],
    });
  });

  it('rejects old host-to-service mappings', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      ['profile: dev', 'hosts:', '  localhost:3011: sample', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config hosts is no longer supported/
    );
  });
});

function restoreEnv(name: string, value: string | undefined): void {
  if (value === undefined) {
    delete process.env[name];
    return;
  }
  process.env[name] = value;
}
