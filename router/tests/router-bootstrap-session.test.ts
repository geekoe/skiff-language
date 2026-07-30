import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import {
  decodeRuntimeFrame,
  encodeRuntimeFrame
} from '../src/protocol/envelope.js';
import { loadRouterConfig } from '../src/router/config.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadRawHttpManifest
} from './helpers/manifests.js';

const tempDirs: string[] = [];
const sockets: WebSocket[] = [];
const endpoints: RuntimeEndpoint[] = [];

afterEach(async () => {
  for (const socket of sockets.splice(0)) {
    socket.close();
  }
  for (const endpoint of endpoints.splice(0)) {
    await endpoint.close();
  }
  for (const dir of tempDirs.splice(0)) {
    await rm(dir, { recursive: true, force: true });
  }
});

describe('Router runtime bootstrap session', () => {
  it('normalizes required Router-owned bootstrap configuration', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-bootstrap-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'config', 'router.yml');
    await mkdir(join(dir, 'config'));
    await writeFile(
      configPath,
      [
        'profile: dev',
        'artifactsPath: ../shared/artifacts',
        'serviceDb:',
        '  mongoUrl: mongodb://mongo.internal:27017/skiff',
        'http:',
        '  maxRequestBytes: 1048576',
        '  maxResponseBytes: 2097152',
        'runtime:',
        '  maxConcurrency: 64',
        ''
      ].join('\n')
    );

    await expect(loadRouterConfig(configPath)).resolves.toMatchObject({
      artifactsPath: resolve(dir, 'shared/artifacts'),
      serviceDb: { mongoUrl: 'mongodb://mongo.internal:27017/skiff' },
      httpMaxRequestBytes: 1048576,
      httpMaxResponseBytes: 2097152
    });
  });

  it.each([
    ['missing artifactsPath', ['profile: dev', 'serviceDb:', '  mongoUrl: mongodb://mongo']],
    ['empty artifactsPath', ['profile: dev', 'artifactsPath: "  "', 'serviceDb:', '  mongoUrl: mongodb://mongo']],
    ['missing Mongo URL', ['profile: dev', 'artifactsPath: ./artifacts', 'serviceDb: {}']],
    ['empty Mongo URL', ['profile: dev', 'artifactsPath: ./artifacts', 'serviceDb:', '  mongoUrl: "  "']]
  ])('rejects %s', async (_name, lines) => {
    const dir = await mkdtemp(join(tmpdir(), 'skiff-router-bootstrap-'));
    tempDirs.push(dir);
    const configPath = join(dir, 'router.yml');
    await writeFile(
      configPath,
      `${lines.join('\n')}\nhttp:\n  maxRequestBytes: 1048576\n  maxResponseBytes: 2097152\nruntime:\n  maxConcurrency: 64\n`
    );
    await expect(loadRouterConfig(configPath)).rejects.toThrow(/must be a non-empty string/);
  });

  it('sends exactly one bootstrap before registration traffic', async () => {
    const registry = new RuntimeRegistry();
    const endpoint = new RuntimeEndpoint({
      registry,
      bootstrap: {
        artifactsPath: '/srv/skiff/artifacts',
        serviceDb: { mongoUrl: 'mongodb://mongo.internal:27017/skiff' },
        http: { maxResponseBytes: 2097152 },
        activation: {
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
        }
      }
    });
    endpoints.push(endpoint);
    endpoint.setDispatcher(new RuntimeDispatcher({
      registry,
      frameSender: endpoint,
      maxConcurrency: 64
    }));
    const listening = await endpoint.listen({ port: 0 });

    const socket = new WebSocket(listening.url);
    sockets.push(socket);
    const frames: unknown[] = [];
    socket.on('message', (data) => {
      frames.push(decodeRuntimeFrame(data).header);
    });
    await waitFor(() => frames.length === 1);

    expect(frames).toEqual([
      {
        schemaVersion: 'skiff-runtime-frame-v3',
        type: 'router.bootstrap',
        artifactsPath: '/srv/skiff/artifacts',
        serviceDb: { mongoUrl: 'mongodb://mongo.internal:27017/skiff' },
        http: { maxResponseBytes: 2097152 },
        activation: {
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
        }
      }
    ]);

    const manifest = loadRawHttpManifest();
    socket.send(encodeRuntimeFrame({
      schemaVersion: 'skiff-runtime-frame-v3',
      type: 'runtime.register',
      runtimeId: 'runtime-bootstrap-order',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [manifest.operations[0]!.target]
    }));
    await waitFor(() => frames.length === 2);
    expect(frames[1]).toMatchObject({
      type: 'runtime.registered',
      runtimeId: 'runtime-bootstrap-order'
    });
    expect(frames.filter((frame) =>
      typeof frame === 'object' && frame !== null && 'type' in frame &&
      frame.type === 'router.bootstrap'
    )).toHaveLength(1);
  });

  it('reads the active committed tuple for each Runtime connection', async () => {
    const registry = new RuntimeRegistry();
    let generation = 7;
    const endpoint = new RuntimeEndpoint({
      registry,
      bootstrap: () => ({
        artifactsPath: '/srv/skiff/artifacts',
        serviceDb: { mongoUrl: 'mongodb://mongo.internal:27017/skiff' },
        http: { maxResponseBytes: 2097152 },
        activation: {
          environment: 'test',
          generation,
          assembly: {
            assemblyIdentity:
              `skiff-runtime-assembly-v3:sha256:${(generation === 7 ? 'a' : 'b').repeat(64)}`
          },
          configSnapshot: {
            snapshotId:
              `skiff-runtime-config-snapshot-v1:${(generation === 7 ? 'a' : 'b').repeat(32)}`
          }
        }
      })
    });
    endpoints.push(endpoint);
    endpoint.setDispatcher(new RuntimeDispatcher({
      registry,
      frameSender: endpoint,
      maxConcurrency: 64
    }));
    const listening = await endpoint.listen({ port: 0 });

    const first = new WebSocket(listening.url);
    sockets.push(first);
    const firstFrames: unknown[] = [];
    first.on('message', (data) => {
      firstFrames.push(decodeRuntimeFrame(data).header);
    });
    await waitFor(() => firstFrames.length === 1);
    expect(firstFrames[0]).toMatchObject({
      type: 'router.bootstrap',
      activation: {
        generation: 7,
        assembly: {
          assemblyIdentity:
            `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`
        }
      }
    });
    first.close();

    generation = 8;
    const second = new WebSocket(listening.url);
    sockets.push(second);
    const secondFrames: unknown[] = [];
    second.on('message', (data) => {
      secondFrames.push(decodeRuntimeFrame(data).header);
    });
    await waitFor(() => secondFrames.length === 1);
    expect(secondFrames[0]).toMatchObject({
      type: 'router.bootstrap',
      activation: {
        generation: 8,
        assembly: {
          assemblyIdentity:
            `skiff-runtime-assembly-v3:sha256:${'b'.repeat(64)}`
        }
      }
    });
  });
});

async function waitFor(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 2_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error('timed out waiting for Router runtime frames');
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}
