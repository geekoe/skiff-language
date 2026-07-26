import { describe, expect, it, vi } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeProtocol.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { ServiceProtocolBoundaryError } from '../src/router/errors.js';
import {
  RuntimeDispatcher,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v2:sha256:${'a'.repeat(64)}`;
const GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v1:sha256:${'b'.repeat(64)}`;
const HOST = 'chat.localhost';
const PATH = '/v1/chat';

describe('retired RuntimeAssembly WebSocket dispatch', () => {
  it('keeps the current ingress index HTTP-only', () => {
    const index = new RuntimeAssemblyIngressIndex([httpBinding()]);
    expect(index.values()).toHaveLength(1);
    expect(() => new RuntimeAssemblyIngressIndex([
      {
        ...httpBinding(),
        selector: {
          protocol: 'webSocket',
          host: HOST,
          method: null,
          path: PATH
        }
      } as never
    ])).toThrow(/only HTTP/);
  });

  it('rejects legacy WebSocket routing and adapter fields at the strict wire validator', () => {
    const validation = validateRuntimeAssemblyRequestStartFrameHeader(
      legacyWebSocketRequest()
    );
    expect(validation.ok).toBe(false);
    if (validation.ok) {
      throw new Error('legacy RuntimeAssembly WebSocket request was accepted');
    }
    expect(validation.error).toMatch(
      /gatewayEntryIdentity|routing|ingress|unknown|HTTP/i
    );
  });

  it('fails closed before selecting or writing to a Runtime connection', async () => {
    const snapshots = snapshotStore();
    const registry = new AssemblyRuntimeRegistry(snapshots);
    const sender = {
      sendFrame: vi.fn()
    } satisfies RuntimeFrameSender;
    const dispatcher = new RuntimeDispatcher({ registry, frameSender: sender });
    const request = legacyWebSocketRequest();

    expect(registry.pickDispatchConnection(request)).toBeInstanceOf(
      ServiceProtocolBoundaryError
    );
    await expect(dispatcher.dispatchBinary(
      { header: request, payloadBytes: new Uint8Array() },
      1_000
    )).rejects.toBeInstanceOf(ServiceProtocolBoundaryError);
    expect(sender.sendFrame).not.toHaveBeenCalled();
  });
});

function snapshotStore(): RouterActiveAssemblySnapshotStore {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex([httpBinding()])
  });
  return snapshots;
}

function httpBinding(): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'http',
      host: HOST,
      method: 'POST',
      path: PATH
    },
    deployment: {
      serviceId: 'example/chat',
      contractVersion: '1.0.0',
      deploymentRevision: 'revision-a',
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v2:sha256:${'c'.repeat(64)}`
    },
    gatewayEntryKey: 'chat',
    gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
    adapterKind: 'typedJson',
    operationMode: 'unary'
  };
}

function legacyWebSocketRequest(): RuntimeAssemblyRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: 'legacy-websocket-request',
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 7,
      gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
      ingress: {
        protocol: 'webSocket',
        host: HOST,
        method: null,
        path: PATH
      }
    },
    gatewayEntryIdentity:
      `skiff-gateway-v1:sha256:${'d'.repeat(64)}`,
    websocketEntryId:
      `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`,
    websocketAdapter: {
      kind: 'receive',
      adapterArgs: [
        { param: 'event', source: { kind: 'websocket.ingressEvent' } }
      ],
      receiveEvent: {
        connectionId: 'connection-a',
        message: { tag: 'text', encoding: 'utf8' },
        payloadSegments: []
      }
    },
    testEffectsEnabled: false
  } as unknown as RuntimeAssemblyRequestStartFrameHeader;
}
