import { mkdir, mkdtemp, rm, writeFile as writeFileRaw } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { TELEMETRY_PROTOCOL, TELEMETRY_TOPICS } from '../src/protocol/envelope.js';
import {
  loadRouterConfig,
  redactRouterConfig,
  ROUTER_CONFIG_REDACTED_VALUE,
  runtimeBootstrapForRouterConfig
} from '../src/router/config.js';

const tempDirs: string[] = [];
const originalDevHomeEnv = process.env.SKIFF_DEV_HOME;

async function writeRouterConfigFixture(path: string, contents: string): Promise<void> {
  const withHttpCeilings = /^http:/m.test(contents)
    ? contents.replace(
        /^http:\s*$/m,
        [
          'http:',
          /^\s+maxRequestBytes:/m.test(contents) ? '' : '  maxRequestBytes: 67108864',
          /^\s+maxResponseBytes:/m.test(contents) ? '' : '  maxResponseBytes: 67108864'
        ].filter(Boolean).join('\n')
      )
    : `${contents}\nhttp:\n  maxRequestBytes: 67108864\n  maxResponseBytes: 67108864`;
  const withRuntimeCapacity = /^runtime:/m.test(withHttpCeilings)
    ? withHttpCeilings.replace(
        /^runtime:\s*$/m,
        [
          'runtime:',
          /^\s+maxConcurrency:/m.test(withHttpCeilings)
            ? ''
            : '  maxConcurrency: 64'
        ].filter(Boolean).join('\n')
      )
    : `${withHttpCeilings}\nruntime:\n  maxConcurrency: 64`;
  const required = [
    withRuntimeCapacity,
    /^artifactsPath:/m.test(contents) ? '' : 'artifactsPath: ./artifacts',
    /^serviceDb:/m.test(contents)
      ? ''
      : 'serviceDb:\n  mongoUrl: mongodb://127.0.0.1:27017/skiff',
    ''
  ].filter((line) => line.length > 0).join('\n');
  await writeFileRaw(path, required);
}

beforeEach(() => {
  delete process.env.SKIFF_DEV_HOME;
});

afterEach(async () => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) {
      await rm(dir, { recursive: true, force: true });
    }
  }
  restoreEnv('SKIFF_DEV_HOME', originalDevHomeEnv);
});

describe('router config', () => {
  it('keeps the checked-in example explicit about the shared artifact path', async () => {
    const examplePath = fileURLToPath(new URL('../router.example.yml', import.meta.url));
    await expect(loadRouterConfig(examplePath)).resolves.toMatchObject({
      artifactsPath: resolve(fileURLToPath(new URL('..', import.meta.url)), '../var/skiff-artifacts'),
      runtimeMaxConcurrency: 256
    });
  });

  it('loads router.yml values and resolves manifest relative to the config file', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'host: 0.0.0.0',
        'artifactsPath: ../var/skiff-artifacts',
        'releaseMode: true',
        'manifest: manifests/router-manifest.json',
        'requestTimeoutMs: 7000',
        'activation:',
        '  prepareTimeoutMs: 120000',
        'http:',
        '  port: 5010',
        '  maxRequestBytes: 16777216',
        '  maxResponseBytes: 8388608',
        'runtime:',
        '  port: 5011',
        '  path: /runtime-dev',
        '  maxConcurrency: 17',
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
      artifactsPath: join(dir, '..', 'var/skiff-artifacts'),
      serviceDb: {
        mongoUrl: 'mongodb://127.0.0.1:27017/skiff',
      },
      host: '0.0.0.0',
      httpMaxRequestBytes: 16777216,
      httpMaxResponseBytes: 8388608,
      httpPort: 5010,
      manifests: [join(dir, 'manifests/router-manifest.json')],
      profile: 'dev',
      releaseMode: true,
      requestTimeoutMs: 7000,
      activationPrepareTimeoutMs: 120000,
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
      runtimeMaxConcurrency: 17,
      websocketPath: '/socket',
    });
  });

  it('projects only the configured response ceiling onto Runtime bootstrap', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'http:',
        '  maxRequestBytes: 111',
        '  maxResponseBytes: 222',
        ''
      ].join('\n')
    );

    const config = await loadRouterConfig(configPath);
    const activation = {
      environment: 'test',
      generation: 7,
      assembly: {
        assemblyIdentity:
          `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`
      },
      configSnapshot: {
        snapshotId:
          'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      }
    };
    expect(runtimeBootstrapForRouterConfig(config, activation)).toEqual({
      artifactsPath: join(dir, 'artifacts'),
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff' },
      http: { maxResponseBytes: 222 },
      activation
    });
  });

  it('requires runtime.maxConcurrency and rejects non-positive unsafe values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const missingPath = join(dir, 'router-missing.yml');
    await writeFileRaw(
      missingPath,
      [
        'profile: dev',
        'artifactsPath: ./artifacts',
        'serviceDb:',
        '  mongoUrl: mongodb://127.0.0.1:27017/skiff',
        'http:',
        '  maxRequestBytes: 1',
        '  maxResponseBytes: 1',
        'runtime:',
        '  port: 4001',
        ''
      ].join('\n')
    );
    await expect(loadRouterConfig(missingPath)).rejects.toThrow(
      /runtime\.maxConcurrency must be a positive integer/
    );

    for (const [index, value] of [
      '0',
      '-1',
      '1.5',
      '"64"',
      '{}',
      '9007199254740992'
    ].entries()) {
      const configPath = join(dir, `router-runtime-capacity-${index}.yml`);
      await writeRouterConfigFixture(
        configPath,
        ['profile: dev', 'runtime:', `  maxConcurrency: ${value}`, ''].join('\n')
      );
      await expect(loadRouterConfig(configPath)).rejects.toThrow(
        /runtime\.maxConcurrency must be a positive integer/
      );
    }
  });

  it('allows command line overrides on top of router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(configPath, ['profile: base', 'manifest: base.json', ''].join('\n'));

    await expect(
      loadRouterConfig(configPath, {
        host: '127.0.0.2',
        artifactsPath: 'artifact-override',
        httpMaxRequestBytes: '33554432',
        httpMaxResponseBytes: '16777216',
        httpPort: '6010',
        manifest: 'override.json',
        requestTimeoutMs: '9000',
        activationPrepareTimeoutMs: '150000',
        runtimePath: '/override-runtime',
        runtimePort: '6011',
        websocketPath: '/override-ws',
        profile: 'prod',
        releaseMode: true,
      })
    ).resolves.toMatchObject({
      artifactsPath: join(dir, 'artifact-override'),
      host: '127.0.0.2',
      httpMaxRequestBytes: 33554432,
      httpMaxResponseBytes: 16777216,
      httpPort: 6010,
      manifests: [join(dir, 'override.json')],
      profile: 'prod',
      releaseMode: true,
      requestTimeoutMs: 9000,
      activationPrepareTimeoutMs: 150000,
      runtimePath: '/override-runtime',
      runtimePort: 6011,
      websocketPath: '/override-ws',
    });
  });

  it('defaults activation prepare independently from the business request timeout', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
      configPath,
      ['profile: dev', 'requestTimeoutMs: 7000', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      requestTimeoutMs: 7000,
      activationPrepareTimeoutMs: 120000,
    });
  });

  it('fails closed on invalid activation prepare timeout values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    for (const [index, value] of [
      '0',
      '-1',
      '1.5',
      '"120000"',
      '{}',
      '9007199254740992',
    ].entries()) {
      const configPath = join(dir, `router-${index}.yml`);
      await writeRouterConfigFixture(
        configPath,
        [
          'profile: dev',
          'activation:',
          `  prepareTimeoutMs: ${value}`,
          '',
        ].join('\n')
      );
      await expect(loadRouterConfig(configPath)).rejects.toThrow(
        /activation\.prepareTimeoutMs must be a positive integer/
      );
    }
  });

  it('loads router profile from top-level profile and allows overrides', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(configPath, 'profile: staging\n');

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
    await writeRouterConfigFixture(
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

  it('loads telemetry config with router-owned defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(emptyConfig, ['profile: dev', 'fileBackend: {}', ''].join('\n'));
    await expect(loadRouterConfig(emptyConfig)).rejects.toThrow(
      /router config fileBackend must configure local or oss/
    );

    const missingCredential = join(dir, 'file-missing-credential.yml');
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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

  it('rejects unknown top-level keys under the frozen schema', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'unknown-top-level.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'ecosystemStoreCliPath: /tmp/skiff/bin/skiff-compiler',
        ''
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config ecosystemStoreCliPath is not supported/
    );
  });

  it('rejects unknown nested keys under the frozen schema', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'unknown-nested.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'http:',
        '  additional: 1',
        ''
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config http\.additional is not supported/
    );
  });

  it('rejects duplicate YAML keys', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'duplicate-key.yml');
    await writeRouterConfigFixture(
      configPath,
      ['profile: dev', 'profile: prod', ''].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(/duplicate key/);
  });

  it('rejects YAML anchors, aliases, and tags', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const anchorPath = join(dir, 'anchor.yml');
    await writeRouterConfigFixture(
      anchorPath,
      ['profile: dev', 'host: &anchor 127.0.0.1', ''].join('\n')
    );
    await expect(loadRouterConfig(anchorPath)).rejects.toThrow(
      /config YAML anchors are not supported/
    );

    const aliasPath = join(dir, 'alias.yml');
    await writeRouterConfigFixture(
      aliasPath,
      [
        'profile: dev',
        'host: &host 127.0.0.1',
        'aliasHost: *host',
        ''
      ].join('\n')
    );
    await expect(loadRouterConfig(aliasPath)).rejects.toThrow(
      /config YAML aliases are not supported/
    );

    const tagPath = join(dir, 'tag.yml');
    await writeRouterConfigFixture(
      tagPath,
      ['profile: dev', 'requestTimeoutMs: !!str 20000', ''].join('\n')
    );
    await expect(loadRouterConfig(tagPath)).rejects.toThrow(
      /config YAML tags are not supported/
    );
  });

  it('redacts secret leaves for diagnostics without mutating the config', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'secrets.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'serviceDb:',
        '  mongoUrl: mongodb://user:pass@127.0.0.1:27017/skiff',
        'fileBackend:',
        '  oss:',
        '    endpoint: https://oss-cn-hangzhou.aliyuncs.com',
        '    bucket: skiff-files',
        '    accessKeyId: local-only-id',
        '    accessKeySecret: local-only-secret',
        '    accessKeyIdEnv: SKIFF_OSS_ACCESS_KEY_ID',
        '    accessKeySecretEnv: SKIFF_OSS_ACCESS_KEY_SECRET',
        ''
      ].join('\n')
    );

    const config = await loadRouterConfig(configPath);
    const redacted = redactRouterConfig(config);
    expect(redacted.serviceDb.mongoUrl).toBe(ROUTER_CONFIG_REDACTED_VALUE);
    expect(redacted.fileBackend?.oss?.accessKeyId).toBe(
      ROUTER_CONFIG_REDACTED_VALUE
    );
    expect(redacted.fileBackend?.oss?.accessKeySecret).toBe(
      ROUTER_CONFIG_REDACTED_VALUE
    );
    expect(redacted.fileBackend?.oss?.accessKeyIdEnv).toBe(
      'SKIFF_OSS_ACCESS_KEY_ID'
    );
    expect(redacted.fileBackend?.oss?.accessKeySecretEnv).toBe(
      'SKIFF_OSS_ACCESS_KEY_SECRET'
    );
    expect(redacted.fileBackend?.oss?.endpoint).toBe(
      'https://oss-cn-hangzhou.aliyuncs.com'
    );
    expect(config.serviceDb.mongoUrl).toBe(
      'mongodb://user:pass@127.0.0.1:27017/skiff'
    );
  });

  it('omits telemetry when disabled or endpoint is not configured', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const disabledConfig = join(dir, 'disabled.yml');
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(noEndpointConfig, ['profile: dev', 'telemetry:', '  enabled: true', ''].join('\n'));
    await expect(loadRouterConfig(noEndpointConfig)).resolves.not.toHaveProperty('telemetry');
  });

  it('rejects invalid telemetry config values', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);

    const invalidProtocol = join(dir, 'invalid-protocol.yml');
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(badConfig, ['profile: dev', 'devReload: latest', ''].join('\n'));
    await expect(loadRouterConfig(badConfig)).rejects.toThrow(
      /router config devReload must be a boolean/
    );
  });

  it.each(['maxRequestBytes', 'maxResponseBytes'])(
    'rejects invalid and missing http.%s values',
    async (field) => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const otherField =
      field === 'maxRequestBytes' ? 'maxResponseBytes' : 'maxRequestBytes';

    const zeroConfig = join(dir, `zero-${field}.yml`);
    await writeRouterConfigFixture(
      zeroConfig,
      ['profile: dev', 'http:', `  ${field}: 0`, `  ${otherField}: 16`, ''].join('\n')
    );
    await expect(loadRouterConfig(zeroConfig)).rejects.toThrow(
      new RegExp(`router config http\\.${field} must be a positive integer`)
    );

    const fractionalConfig = join(dir, `fractional-${field}.yml`);
    await writeRouterConfigFixture(
      fractionalConfig,
      ['profile: dev', 'http:', `  ${field}: 1.5`, `  ${otherField}: 16`, ''].join('\n')
    );
    await expect(loadRouterConfig(fractionalConfig)).rejects.toThrow(
      new RegExp(`router config http\\.${field} must be a positive integer`)
    );

    const overflowConfig = join(dir, `overflow-${field}.yml`);
    await writeRouterConfigFixture(
      overflowConfig,
      [
        'profile: dev',
        'http:',
        `  ${field}: 9007199254740992`,
        `  ${otherField}: 16`,
        ''
      ].join('\n')
    );
    await expect(loadRouterConfig(overflowConfig)).rejects.toThrow(
      new RegExp(`router config http\\.${field} must be a positive integer`)
    );

    const missingConfig = join(dir, `missing-${field}.yml`);
    await writeFileRaw(
      missingConfig,
      [
        'profile: dev',
        'artifactsPath: ./artifacts',
        'serviceDb:',
        '  mongoUrl: mongodb://127.0.0.1:27017/skiff',
        'http:',
        `  ${otherField}: 16`,
        ''
      ].join('\n')
    );
    await expect(loadRouterConfig(missingConfig)).rejects.toThrow(
      new RegExp(`router config http\\.${field} must be a positive integer`)
    );
  });

  it('rejects the removed http.bodyLimitBytes alias', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'removed-body-limit.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'http:',
        '  bodyLimitBytes: 16',
        '  maxRequestBytes: 16',
        '  maxResponseBytes: 16',
        ''
      ].join('\n')
    );
    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /http\.bodyLimitBytes is no longer supported/
    );
  });

  it('requires top-level profile in router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(configPath, 'manifest: base.json\n');

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config profile is required/
    );
  });

  it('rejects values.profile in router.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(configPath, ['values:', '  profile: prod', ''].join('\n'));

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config values\.profile is no longer supported/
    );
  });

  it('rejects profile names that cannot be used in config filenames', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(configPath, 'profile: prod-us\n');

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config profile must match \[A-Za-z_\]\[A-Za-z0-9_\]\*/
    );
  });

  it('loads multiple manifests for a shared router', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
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
    await writeRouterConfigFixture(configPath, ['profile: dev', 'artifacts: artifacts', ''].join('\n'));

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config artifacts is no longer supported; use artifactsPath/
    );
  });

  it('rejects plural artifact roots', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
      configPath,
      [
        'profile: dev',
        'artifactRoots:',
        '  - artifacts/base',
        '  - artifacts/test',
        '',
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).rejects.toThrow(
      /router config artifactRoots is no longer supported; use artifactsPath/
    );
  });

  it('rejects old host-to-service mappings', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeRouterConfigFixture(
      configPath,
      ['profile: dev', 'hosts:', '  localhost: sample', ''].join('\n')
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
