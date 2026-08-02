import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { TELEMETRY_PROTOCOL, TELEMETRY_TOPICS } from '../src/protocol/envelope.js';
import {
  loadRouterConfig,
  redactRouterConfig,
  ROUTER_CONFIG_REDACTED_VALUE
} from '../src/router/config.js';

interface CorpusEntry {
  name: string;
  path: string;
  error?: string;
}

interface RouterConfigCorpus {
  schemaVersion: string;
  systems: string[];
  valid: CorpusEntry[];
  invalid: CorpusEntry[];
}

const fixturesDir = fileURLToPath(
  new URL('./fixtures/router-config', import.meta.url)
);
const corpus = JSON.parse(
  await readFile(join(fixturesDir, 'corpus.json'), 'utf8')
) as RouterConfigCorpus;

describe('router config golden corpus', () => {
  it('uses the frozen corpus schema and unique case names', () => {
    expect(corpus.schemaVersion).toBe('skiff-router-config-corpus-v1');
    expect(corpus.systems).toEqual(['router']);
    const validNames = corpus.valid.map((entry) => entry.name);
    const invalidNames = corpus.invalid.map((entry) => entry.name);
    expect(new Set(validNames).size).toBe(validNames.length);
    expect(new Set(invalidNames).size).toBe(invalidNames.length);
    expect(validNames.some((name) => invalidNames.includes(name))).toBe(false);
    for (const entry of corpus.invalid) {
      expect(entry.error, entry.name).toBeTruthy();
    }
  });

  it.each(corpus.valid)('accepts valid case $name', async ({ path }) => {
    await expect(loadRouterConfig(join(fixturesDir, path))).resolves.toBeTruthy();
  });

  it.each(corpus.invalid)(
    'rejects invalid case $name with the frozen error',
    async ({ path, error }) => {
      await expect(loadRouterConfig(join(fixturesDir, path))).rejects.toThrow(
        new RegExp(error!)
      );
    }
  );

  it('normalizes the canonical corpus cases exactly', async () => {
    const dir = fixturesDir;
    const configDir = join(dir, 'valid');
    const expectations: Record<string, Record<string, unknown>> = {
      canonical: {
        profile: 'dev',
        environment: 'dev',
        host: '127.0.0.1',
        artifactsPath: resolve(configDir, '../var/skiff-artifacts'),
        devReload: true,
        releaseMode: false,
        requestTimeoutMs: 20000,
        activationPrepareTimeoutMs: 120000,
        httpPort: 4000,
        httpMaxRequestBytes: 67108864,
        httpMaxResponseBytes: 8388608,
        runtimePort: 4001,
        runtimePath: '/runtime',
        runtimeMaxConcurrency: 256,
        websocketPath: '/ws',
        serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/?replicaSet=rs0' },
        telemetry: {
          enabled: true,
          endpoint: 'ws://127.0.0.1:4002/telemetry',
          protocol: TELEMETRY_PROTOCOL,
          topics: [...TELEMETRY_TOPICS],
          queueMaxEvents: 10000,
          batchMaxEvents: 200,
          batchMaxBytes: 262144,
          flushIntervalMs: 1000,
        },
        rewrite: [
          {
            host: 'account.localhost',
            service: 'skiff.run/account',
            version: '0.1.0',
          },
        ],
      },
      minimal: {
        profile: 'dev',
        host: '127.0.0.1',
        artifactsPath: join(configDir, 'artifacts'),
        httpPort: 4000,
        runtimePort: 4001,
        runtimePath: '/runtime',
        websocketPath: '/ws',
        requestTimeoutMs: 20000,
        activationPrepareTimeoutMs: 120000,
        manifests: [join(configDir, 'fixtures/hello/manifest.json')],
        runtimeMaxConcurrency: 1,
      },
      'renderer-canonical': {
        profile: 'dev',
        host: '127.0.0.1',
        environment: 'dev',
        artifactsPath: '/tmp/skiff/artifacts',
        devReload: true,
        requestTimeoutMs: 20000,
        activationPrepareTimeoutMs: 120000,
        httpPort: 4000,
        httpMaxRequestBytes: 67108864,
        httpMaxResponseBytes: 8388608,
        runtimePort: 4001,
        runtimePath: '/runtime',
        runtimeMaxConcurrency: 128,
      },
      aliases: {
        profile: 'staging',
        httpPort: 5010,
        runtimePort: 5011,
        runtimePath: '/runtime-dev',
        websocketPath: '/socket',
        manifests: [join(configDir, 'manifests/one.json')],
        runtimeMaxConcurrency: 2,
      },
      manifests: {
        manifests: [
          join(configDir, 'manifests/a.json'),
          join(configDir, 'manifests/b.json'),
        ],
      },
      telemetry: {
        telemetry: {
          enabled: true,
          endpoint: 'ws://127.0.0.1:4002/telemetry',
          protocol: TELEMETRY_PROTOCOL,
          topics: [...TELEMETRY_TOPICS],
          queueMaxEvents: 5,
          batchMaxEvents: 3,
          batchMaxBytes: 1024,
          flushIntervalMs: 500,
        },
      },
      'file-backend': {
        fileBackend: {
          local: { root: resolve(configDir, '../var/blobs') },
          oss: {
            endpoint: 'https://oss-cn-hangzhou.aliyuncs.com',
            bucket: 'skiff-files',
            region: 'cn-hangzhou',
            accessKeyIdEnv: 'SKIFF_OSS_ACCESS_KEY_ID',
            accessKeySecretEnv: 'SKIFF_OSS_ACCESS_KEY_SECRET',
          },
        },
      },
      'direct-secrets': {
        serviceDb: {
          mongoUrl: 'mongodb://user:pass@127.0.0.1:27017/skiff',
        },
        fileBackend: {
          oss: {
            accessKeyId: 'local-only-id',
            accessKeySecret: 'local-only-secret',
          },
        },
      },
      rewrite: {
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
      },
      'numeric-strings': {
        httpPort: 4000,
        runtimePort: 4001,
        requestTimeoutMs: 7000,
        httpMaxRequestBytes: 16777216,
        httpMaxResponseBytes: 8388608,
      },
    };

    for (const entry of corpus.valid) {
      const config = await loadRouterConfig(join(fixturesDir, entry.path));
      const expected = expectations[entry.name];
      if (expected !== undefined) {
        expect(config, entry.name).toMatchObject(expected);
      }
    }
  });

  it('redacts Router config secrets without mutating the parsed config', async () => {
    const direct = await loadRouterConfig(
      join(fixturesDir, 'valid/direct-secrets.yml')
    );
    const redactedDirect = redactRouterConfig(direct);
    expect(redactedDirect.serviceDb.mongoUrl).toBe(ROUTER_CONFIG_REDACTED_VALUE);
    expect(redactedDirect.fileBackend?.oss?.accessKeyId).toBe(
      ROUTER_CONFIG_REDACTED_VALUE
    );
    expect(redactedDirect.fileBackend?.oss?.accessKeySecret).toBe(
      ROUTER_CONFIG_REDACTED_VALUE
    );
    expect(direct.serviceDb.mongoUrl).toBe(
      'mongodb://user:pass@127.0.0.1:27017/skiff'
    );
    expect(redactedDirect.artifactsPath).toBe(direct.artifactsPath);
    expect(redactedDirect.fileBackend?.oss?.endpoint).toBe(
      'https://oss-cn-hangzhou.aliyuncs.com'
    );

    const env = await loadRouterConfig(
      join(fixturesDir, 'valid/file-backend.yml')
    );
    const redactedEnv = redactRouterConfig(env);
    expect(redactedEnv.serviceDb.mongoUrl).toBe(ROUTER_CONFIG_REDACTED_VALUE);
    expect(redactedEnv.fileBackend?.oss?.accessKeyIdEnv).toBe(
      'SKIFF_OSS_ACCESS_KEY_ID'
    );
    expect(redactedEnv.fileBackend?.oss?.accessKeySecretEnv).toBe(
      'SKIFF_OSS_ACCESS_KEY_SECRET'
    );
  });
});
