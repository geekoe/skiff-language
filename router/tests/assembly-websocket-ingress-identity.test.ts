import { describe, expect, it } from 'vitest';

import {
  canonicalWebSocketIngressIdentity
} from '../src/gateway/assemblyWebSocketGateway.js';
import {
  canonicalAssemblyWebSocketIngressIdentity
} from '../src/router/assemblyRuntimeRegistry.js';
import type {
  RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const binding = {
  selector: {
    protocol: 'webSocket',
    host: 'chat.localhost',
    method: null,
    path: '/v1/chat'
  },
  deployment: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v2:sha256:${'e'.repeat(64)}`
  },
  contract: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    serviceProtocolIdentity:
      `skiff-service-protocol-v5:sha256:${'d'.repeat(64)}`
  },
  operationMode: 'unary',
  contractOperationId:
    `skiff-contract-operation-v1:sha256:${'c'.repeat(64)}`
} as unknown as RuntimeAssemblyIngressBinding;

describe('canonical assembly WebSocket ingress identity export', () => {
  it('keeps the registry helper as the single gateway owner and preserves its digest', () => {
    expect(canonicalWebSocketIngressIdentity).toBe(
      canonicalAssemblyWebSocketIngressIdentity
    );
    expect(canonicalAssemblyWebSocketIngressIdentity(binding)).toEqual({
      websocketEntryId:
        'skiff-websocket-entry-v1:sha256:c22ef23a292517ae07ac5f42bbb721e83545f153c22e70f4d50c03494012e4b8',
      gatewayEntryIdentity:
        'skiff-gateway-v1:sha256:c22ef23a292517ae07ac5f42bbb721e83545f153c22e70f4d50c03494012e4b8'
    });
  });
});
