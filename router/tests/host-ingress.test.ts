import { describe, expect, it } from 'vitest';

import {
  RuntimeAssemblyIngressIndex,
  runtimeAssemblyIngressKey,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

describe('service-scoped RuntimeAssembly ingress index', () => {
  it('allows different services to share the same method/path', () => {
    const codex = binding('skiff.run/codex-relay', 'models', '1');
    const aihub = binding('skiff.run/aihub', 'models', '2');
    const index = new RuntimeAssemblyIngressIndex([codex, aihub]);
    const selector = {
      protocol: 'http' as const,
      method: 'GET',
      path: '/v1/models'
    };

    expect(index.get(
      { serviceId: 'skiff.run/codex-relay', contractVersion: '1.0.0' },
      selector
    )).toEqual(codex);
    expect(index.get(
      { serviceId: 'skiff.run/aihub', contractVersion: '1.0.0' },
      selector
    )).toEqual(aihub);
    expect(index.get(
      { serviceId: 'skiff.run/unknown', contractVersion: '1.0.0' },
      selector
    )).toBeUndefined();
  });

  it('retains exact deployment and gateway facts', () => {
    const value = binding('skiff.run/stream', 'stream', '3', {
      method: 'POST',
      path: '/stream',
      adapterKind: 'rawHttp',
      operationMode: 'serverStream',
      timeoutMs: 4_000
    });
    const loaded = new RuntimeAssemblyIngressIndex([value]).values()[0]!;

    expect(loaded).toEqual(value);
    expect(Object.keys(loaded).sort()).toEqual([
      'adapterKind',
      'deployment',
      'gatewayEntryIdentity',
      'gatewayEntryKey',
      'operationMode',
      'selector',
      'timeoutMs'
    ]);
  });

  it('rejects same-service duplicates and multiple active revisions', () => {
    const first = binding('skiff.run/echo', 'echo', '4');
    const duplicate = structuredClone(first);
    duplicate.selector.method = 'get';
    expect(() => new RuntimeAssemblyIngressIndex([first, duplicate])).toThrow(
      /duplicate gateway ingress/
    );

    const otherRevision = binding('skiff.run/echo', 'other', '5', {
      path: '/other'
    });
    expect(() => new RuntimeAssemblyIngressIndex([first, otherRevision])).toThrow(
      /multiple deployments/
    );
  });

  it('keeps WebSocket methods out of the physical attach index', () => {
    expect(runtimeAssemblyIngressKey({
      protocol: 'webSocket',
      method: null,
      path: '/echo'
    })).toBe('webSocket\u0000/echo');
    expect(runtimeAssemblyIngressKey({
      protocol: 'webSocket',
      method: 'status.get',
      path: '/echo'
    })).toBe('webSocket\u0000status.get\u0000/echo');
    expect(() => new RuntimeAssemblyIngressIndex([{
      ...binding('skiff.run/echo', 'echo', '6'),
      selector: {
        protocol: 'webSocket',
        method: 'status.get',
        path: '/echo'
      }
    } as never])).toThrow(/attach ingress method must be null/);
  });

  it.each([
    { method: '', path: '/echo' },
    { method: 'POST', path: 'echo' },
    { method: 'POST', path: '/echo?query=true' }
  ])('rejects invalid canonical HTTP selector %#', (selector) => {
    expect(() => runtimeAssemblyIngressKey({
      protocol: 'http',
      ...selector
    })).toThrow();
  });
});

function binding(
  serviceId: string,
  gatewayEntryKey: string,
  identityCharacter: string,
  options: {
    method?: string;
    path?: string;
    adapterKind?: 'rawHttp' | 'typedJson';
    operationMode?: 'unary' | 'serverStream';
    timeoutMs?: number;
  } = {}
): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'http',
      method: options.method ?? 'GET',
      path: options.path ?? '/v1/models'
    },
    deployment: {
      serviceId,
      contractVersion: '1.0.0',
      deploymentRevision: `revision-${identityCharacter}`,
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v4:sha256:${identityCharacter.repeat(64)}`
    },
    gatewayEntryKey,
    gatewayEntryIdentity:
      `skiff-gateway-entry-v2:sha256:${identityCharacter.repeat(64)}`,
    adapterKind: options.adapterKind ?? 'typedJson',
    operationMode: options.operationMode ?? 'unary',
    ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs })
  };
}
