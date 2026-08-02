import { request as httpRequest } from 'node:http';

import WebSocket, { type RawData } from 'ws';
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

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const SERVICE_ID = 'example.com/cors';
const CONTRACT_VERSION = '1.0.0';
const HOST = 'cors.example.test';
const PATH = '/session';
const RUNTIME_ID = 'runtime-assembly-cors';
const DEPLOYMENT = {
  serviceId: SERVICE_ID,
  contractVersion: CONTRACT_VERSION,
  deploymentRevision: 'revision-a',
  deploymentArtifactIdentity:
    `skiff-deployment-artifact-v4:sha256:${'b'.repeat(64)}`
};
const POST_BINDING: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', method: 'POST', path: PATH },
  deployment: DEPLOYMENT,
  gatewayEntryKey: 'session-post',
  gatewayEntryIdentity:
    `skiff-gateway-entry-v2:sha256:${'c'.repeat(64)}`,
  adapterKind: 'typedJson',
  operationMode: 'unary'
};
const OPTIONS_BINDING: RuntimeAssemblyIngressBinding = {
  ...POST_BINDING,
  selector: { protocol: 'http', method: 'OPTIONS', path: PATH },
  gatewayEntryKey: 'session-options',
  gatewayEntryIdentity:
    `skiff-gateway-entry-v2:sha256:${'d'.repeat(64)}`
};

const fixtures: AssemblyCorsFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
});

describe('RuntimeAssembly HTTP CORS ownership', () => {
  it('answers automatic preflight only for an exact committed service/version/path', async () => {
    const fixture = await createFixture([POST_BINDING]);
    const origin = 'http://127.0.0.1:4006';
    const preflightHeaders = {
      Origin: origin,
      'Access-Control-Request-Method': 'POST',
      'Access-Control-Request-Headers': 'content-type, x-skiff-service'
    };

    const response = await sendHttp(fixture.url, {
      method: 'OPTIONS',
      path: PATH,
      headers: preflightHeaders
    });
    expect(response.status).toBe(204);
    expect(response.body).toEqual(Buffer.alloc(0));
    expect(response.headers['access-control-allow-origin']).toBe(origin);
    expect(response.headers['access-control-allow-credentials']).toBe('true');
    expect(response.headers['access-control-allow-methods']).toContain('POST');
    expect(response.headers['access-control-allow-methods']).toContain('OPTIONS');
    expect(response.headers['access-control-allow-headers']).toBe(
      'content-type, x-skiff-service'
    );
    expect(response.headers.vary).toContain('Origin');
    expect(response.headers.vary).toContain('Access-Control-Request-Method');
    expect(response.headers.vary).toContain('Access-Control-Request-Headers');
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });

    const invalidSelectors: Array<{
      path: string;
      serviceId?: string;
      contractVersion?: string;
    }> = [
      { path: '/unknown' },
      { path: PATH, serviceId: 'example.com/unknown' },
      { path: PATH, contractVersion: '2.0.0' }
    ];
    for (const invalid of invalidSelectors) {
      const rejected = await sendHttp(fixture.url, {
        ...invalid,
        method: 'OPTIONS',
        headers: preflightHeaders
      });
      expect(rejected.status).toBe(404);
      expect(JSON.parse(rejected.body.toString())).toMatchObject({
        error: { code: 'AssemblyIngressNotFound' }
      });
      expect(rejected.headers['access-control-allow-origin']).toBe(origin);
    }

    const invalidHost = await sendHttp(fixture.url, {
      method: 'OPTIONS',
      path: PATH,
      headers: { ...preflightHeaders, Host: 'invalid/host' }
    });
    expect(invalidHost.status).toBe(400);
    expect(JSON.parse(invalidHost.body.toString())).toMatchObject({
      error: { code: 'RequestHostInvalid' }
    });
    expect(invalidHost.headers['access-control-allow-origin']).toBe(origin);
  });

  it('keeps automatic CORS consistent for runtime and gateway error responses', async () => {
    const fixture = await createFixture([POST_BINDING]);
    const origin = 'http://127.0.0.1:4006';
    const response = sendHttp(fixture.url, {
      method: 'POST',
      path: PATH,
      headers: { Origin: origin }
    });
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: String(requestFrame.header.requestId),
      payloadPresent: false,
      httpResponse: {
        status: 200,
        headers: [
          {
            name: 'access-control-allow-origin',
            value: 'https://runtime-must-not-override.example'
          },
          { name: 'access-control-allow-credentials', value: 'false' }
        ]
      }
    }));

    const completed = await response;
    expect(completed.headers['access-control-allow-origin']).toBe(origin);
    expect(completed.headers['access-control-allow-credentials']).toBe('true');
    expect(completed.headers.vary).toContain('Origin');

    const rejected = await sendHttp(fixture.url, {
      method: 'GET',
      path: PATH,
      headers: { Origin: origin }
    });
    expect(rejected.status).toBe(404);
    expect(JSON.parse(rejected.body.toString())).toMatchObject({
      error: { code: 'AssemblyIngressNotFound' }
    });
    expect(rejected.headers['access-control-allow-origin']).toBe(origin);
    expect(rejected.headers['access-control-allow-credentials']).toBe('true');
  });

  it('dispatches explicit OPTIONS and leaves CORS ownership with the service', async () => {
    const fixture = await createFixture([POST_BINDING, OPTIONS_BINDING]);
    const origin = 'https://client.example';
    const preflight = sendHttp(fixture.url, {
      method: 'OPTIONS',
      path: PATH,
      headers: {
        Origin: origin,
        'Access-Control-Request-Method': 'POST',
        'Access-Control-Request-Headers': 'content-type'
      }
    });
    const optionsFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    expect(optionsFrame.header).toMatchObject({
      type: 'request.start',
      routing: {
        gatewayEntryIdentity: OPTIONS_BINDING.gatewayEntryIdentity,
        ingress: OPTIONS_BINDING.selector
      }
    });
    sendUnaryResponse(fixture.runtime, optionsFrame.header.requestId, {
      status: 403,
      headers: [{ name: 'vary', value: 'Origin' }],
      body: 'denied'
    });

    const rejected = await preflight;
    expect(rejected.status).toBe(403);
    expect(rejected.body).toEqual(Buffer.from('denied'));
    expect(rejected.headers['access-control-allow-origin']).toBeUndefined();
    expect(rejected.headers['access-control-allow-credentials']).toBeUndefined();
    expect(rejected.headers.vary).toBe('Origin');

    const ordinary = sendHttp(fixture.url, {
      method: 'POST',
      path: PATH,
      headers: { Origin: origin }
    });
    const postFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    sendUnaryResponse(fixture.runtime, postFrame.header.requestId, {
      status: 200,
      headers: [
        { name: 'access-control-allow-origin', value: origin },
        { name: 'access-control-allow-credentials', value: 'true' },
        { name: 'vary', value: 'Origin' }
      ]
    });
    const completed = await ordinary;
    expect(completed.headers['access-control-allow-origin']).toBe(origin);
    expect(completed.headers['access-control-allow-credentials']).toBe('true');
    expect(completed.headers.vary).toBe('Origin');
  });
});

interface AssemblyCorsFixture {
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  gateway: AssemblyHttpGateway;
  runtime: WebSocket;
  url: string;
  close(): Promise<void>;
}

async function createFixture(
  bindings: readonly RuntimeAssemblyIngressBinding[]
): Promise<AssemblyCorsFixture> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 1,
    assembly: { assemblyIdentity: ASSEMBLY },
    configSnapshot: {
      snapshotId:
        'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    resolvedDeployments: [DEPLOYMENT],
    resolvedContracts: [{
      serviceId: SERVICE_ID,
      contractVersion: CONTRACT_VERSION,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'e'.repeat(64)}`
    }],
    deploymentRuntimeBindings: [{
      deployment: DEPLOYMENT,
      packageBuildId:
        `skiff-package-build-v10:sha256:${'f'.repeat(64)}`
    }],
    ingress: new RuntimeAssemblyIngressIndex(bindings)
  });
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const endpoint = new RuntimeEndpoint({
    registry: new RuntimeRegistry(),
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 1024 },
      activation: {
        environment: 'test',
        generation: 1,
        assembly: { assemblyIdentity: ASSEMBLY },
        configSnapshot: {
          snapshotId:
            'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
        }
      }
    }
  });
  const dispatcher = new RuntimeDispatcher({
    registry: assemblyRegistry,
    frameSender: endpoint,
    maxConcurrency: 4
  });
  endpoint.setDispatcher(dispatcher);
  const endpointAddress = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyHttpGateway({
    snapshots,
    dispatcher,
    port: 0,
    maxRequestBytes: 1024,
    maxResponseBytes: 1024,
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
    generation: 1,
    assembly: { assemblyIdentity: ASSEMBLY },
    configSnapshot: {
      snapshotId:
        'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    replicaId: RUNTIME_ID
  }));
  await until(() =>
    assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID)
  );
  const fixture: AssemblyCorsFixture = {
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

function sendUnaryResponse(
  runtime: WebSocket,
  requestId: unknown,
  response: {
    status: number;
    headers: Array<{ name: string; value: string }>;
    body?: string;
  }
): void {
  const body = Buffer.from(response.body ?? '');
  runtime.send(encodeRuntimeFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId: String(requestId),
    payloadPresent: body.byteLength > 0,
    httpResponse: {
      status: response.status,
      headers: response.headers
    }
  }, body));
}

async function sendHttp(
  baseUrl: string,
  input: {
    method: string;
    path: string;
    serviceId?: string;
    contractVersion?: string;
    headers?: Record<string, string>;
  }
): Promise<{
  status: number;
  headers: Record<string, string | string[] | undefined>;
  body: Buffer;
}> {
  const url = new URL(input.path, baseUrl);
  return await new Promise((resolve, reject) => {
    const request = httpRequest(url, {
      method: input.method,
      headers: {
        Host: HOST,
        'x-skiff-service': input.serviceId ?? SERVICE_ID,
        'x-skiff-version': input.contractVersion ?? CONTRACT_VERSION,
        'content-length': '0',
        ...input.headers
      }
    }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => resolve({
        status: response.statusCode ?? 0,
        headers: response.headers,
        body: Buffer.concat(chunks)
      }));
    });
    request.once('error', reject);
    request.end();
  });
}

async function openSocket(url: string): Promise<WebSocket> {
  const socket = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
  return socket;
}

async function nextBinaryMessage(socket: WebSocket): Promise<Buffer> {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error('timed out waiting for RuntimeAssembly request'));
    }, 1_000);
    const onMessage = (data: RawData, isBinary: boolean) => {
      clearTimeout(timeout);
      if (!isBinary) {
        cleanup();
        reject(new Error('expected binary RuntimeAssembly request'));
        return;
      }
      const buffer = Array.isArray(data)
        ? Buffer.concat(data)
        : data instanceof ArrayBuffer
          ? Buffer.from(data)
          : Buffer.from(data.buffer, data.byteOffset, data.byteLength);
      try {
        if (decodeBinaryFrame(buffer).header.type === 'runtime.registered') {
          // Skip the registered ACK handshake frame.
          return;
        }
      } catch {
        // Not a decodable binary frame; pass through.
      }
      cleanup();
      resolve(buffer);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      socket.off('message', onMessage);
    };
    socket.on('message', onMessage);
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
