import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { loadTelemetryConfig } from '../src/config.js';

const tempDirs: string[] = [];

afterEach(async () => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) {
      await rm(dir, { recursive: true, force: true });
    }
  }
});

describe('telemetry config', () => {
  it('loads telemetry and mongo settings from telemetry.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-telemetry-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'telemetry.yml');
    await writeFile(
      configPath,
      [
        'telemetry:',
        '  host: 0.0.0.0',
        '  port: 4021',
        '  path: telemetry',
        'mongo:',
        '  url: mongodb://example/skiff',
        '  database: skiff',
        '  ttlDays: 3',
        '',
      ].join('\n')
    );

    const config = await loadTelemetryConfig({ configPath }, {});

    expect(config.host).toBe('0.0.0.0');
    expect(config.port).toBe(4021);
    expect(config.path).toBe('/telemetry');
    await expect(config.store.health()).resolves.toMatchObject({ store: 'mongo' });
  });

  it('lets env and explicit overrides win over telemetry.yml', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-telemetry-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'telemetry.yml');
    await writeFile(
      configPath,
      [
        'telemetry:',
        '  host: 127.0.0.1',
        '  port: 4002',
        'mongo:',
        '  url: mongodb://file/skiff',
        '  database: file_db',
        '',
      ].join('\n')
    );

    const config = await loadTelemetryConfig(
      { configPath, host: '0.0.0.0', port: '5020', memory: true },
      {
        SKIFF_TELEMETRY_MONGO_URL: 'mongodb://env/skiff',
        SKIFF_TELEMETRY_DB: 'env_db'
      }
    );

    expect(config.host).toBe('0.0.0.0');
    expect(config.port).toBe(5020);
    await expect(config.store.health()).resolves.toMatchObject({ store: 'memory' });
  });

  it('requires mongo url unless memory mode is enabled', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-telemetry-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'telemetry.yml');
    await writeFile(configPath, ['telemetry:', '  port: 4002', ''].join('\n'));

    await expect(loadTelemetryConfig({ configPath }, {})).rejects.toThrow(
      /mongo\.url, SKIFF_TELEMETRY_MONGO_URL, or MONGO_URL is required/
    );

    await expect(loadTelemetryConfig({ configPath, memory: true }, {})).resolves.toMatchObject({
      port: 4002
    });
  });

  it('uses telemetry config and env names only', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-telemetry-config-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'telemetry.yml');
    await writeFile(
      configPath,
      [
        'telemetry:',
        '  host: 127.0.0.1',
        '  port: 4024',
        '',
      ].join('\n')
    );

    const config = await loadTelemetryConfig(
      { configPath },
      {
        SKIFF_TELEMETRY_HOST: '0.0.0.0',
        SKIFF_TELEMETRY_IN_MEMORY: 'true',
      }
    );

    expect(config.host).toBe('0.0.0.0');
    expect(config.port).toBe(4024);
    await expect(config.store.health()).resolves.toMatchObject({ store: 'memory' });
  });
});
