import { request as httpRequest } from 'node:http';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import { AssemblyHttpGateway } from '../src/router/assemblyHttpGateway.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;
const RUNTIME_ID = 'runtime-assembly-stream';
const HOST = 'stream.example.test';
const PATH = '/events';

const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', host: HOST, method: 'POST', path: PATH },
  deployment: {
    serviceId: 'example.com/stream',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v1:sha256:${'c'.repeat(64)}`
  },
  contract: {
    serviceId: 'example.com/stream',
    contractVersion: '1.0.0',
    serviceProtocolIdentity:
      `skiff-service-protocol-v2:sha256:${'d'.repeat(64)}`
  },
  contractOperationId: OPERATION,
  operationMode: 'serverStream'
};

const fixtures: StreamFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
});

describe('RuntimeAssembly HTTP serverStream ingress', () => {
  it('selects the exact ServiceContract mode and preserves ordered binary chunks', async () => {
    const fixture = await createFixture();
    const response = sendHttp(fixture.url, Buffer.from([9, 8, 7]));
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    expect(requestFrame.header).toMatchObject({
      type: 'request.start',
      mode: 'serverStream',
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 4,
        contractOperationId: OPERATION,
        ingress: {
          protocol: 'http',
          host: HOST,
          method: 'POST',
          path: PATH
        }
      }
    });
    expect(Buffer.from(requestFrame.payloadBytes)).toEqual(Buffer.from([9, 8, 7]));
    const requestId = String(requestFrame.header.requestId);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId,
      httpResponse: {
        status: 202,
        headers: [
          { name: 'content-type', value: 'application/octet-stream' },
          { name: 'x-stream-mode', value: 'serverStream' }
        ]
      }
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 0
    }, Buffer.from([0, 255])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 1
    }, Buffer.from([17, 128])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: false
    }));

    await expect(response).resolves.toEqual({
      status: 202,
      headers: expect.objectContaining({
        'content-type': 'application/octet-stream',
        'x-stream-mode': 'serverStream'
      }),
      body: Buffer.from([0, 255, 17, 128])
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.gateway.streamLifecycleCounters()).toEqual({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    });
  });
});

interface StreamFixture {
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  gateway: AssemblyHttpGateway;
  runtime: WebSocket;
  url: string;
  close(): Promise<void>;
}

async function createFixture(): Promise<StreamFixture> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 4,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex([binding])
  });
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const endpoint = new RuntimeEndpoint({
    registry: new RuntimeRegistry(),
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' }
    }
  });
  const dispatcher = new RuntimeDispatcher({
    registry: assemblyRegistry,
    frameSender: endpoint
  });
  endpoint.setDispatcher(dispatcher);
  const endpointAddress = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyHttpGateway({
    snapshots,
    dispatcher,
    port: 0,
    requestTimeoutMs: 1_000
  });
  const gatewayAddress = await gateway.listen();
  const runtime = await openSocket(endpointAddress.url);
  runtime.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId: RUNTIME_ID
  }));
  runtime.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation: 4,
    assembly: { assemblyIdentity: ASSEMBLY },
    replicaId: RUNTIME_ID
  }));
  await until(() =>
    assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID)
  );
  const fixture: StreamFixture = {
    dispatcher,
    endpoint,
    gateway,
    runtime,
    url: gatewayAddress.url,
    close: async () => {
      await gateway.close();
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

async function sendHttp(
  baseUrl: string,
  body: Buffer
): Promise<{
  status: number;
  headers: Record<string, string | string[] | undefined>;
  body: Buffer;
}> {
  const url = new URL(PATH, baseUrl);
  return await new Promise((resolve, reject) => {
    const request = httpRequest(url, {
      method: 'POST',
      headers: {
        Host: HOST,
        'content-length': String(body.byteLength)
      }
    }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.once('end', () => resolve({
        status: response.statusCode ?? 0,
        headers: response.headers,
        body: Buffer.concat(chunks)
      }));
    });
    request.once('error', reject);
    request.end(body);
  });
}

async function openSocket(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  return ws;
}

async function nextBinaryMessage(ws: WebSocket): Promise<Buffer> {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('timed out waiting for RuntimeAssembly request')),
      1_000
    );
    ws.once('message', (data, isBinary) => {
      clearTimeout(timeout);
      if (!isBinary) {
        reject(new Error('expected binary runtime frame'));
        return;
      }
      if (Array.isArray(data)) {
        resolve(Buffer.concat(data));
        return;
      }
      if (data instanceof ArrayBuffer) {
        resolve(Buffer.from(new Uint8Array(data)));
        return;
      }
      resolve(Buffer.from(data.buffer, data.byteOffset, data.byteLength));
    });
  });
}

async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 1_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error('timed out waiting for runtime registration');
    }
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
}
