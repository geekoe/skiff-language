import { describe, expect, it } from 'vitest';

import {
  RuntimeAssemblyIngressIndex,
  runtimeAssemblyIngressKey,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

describe('RuntimeAssembly ingress index', () => {
  it('disambiguates the same method/path by canonical Host', () => {
    const codex = binding('codex-relay.localhost', 'models', '1');
    const aihub = binding('aihub.localhost', 'models', '2');
    const index = new RuntimeAssemblyIngressIndex([codex, aihub]);

    expect(index.get({
      protocol: 'http',
      host: 'CODEX-RELAY.LOCALHOST',
      method: 'get',
      path: '/v1/models'
    })).toEqual(codex);
    expect(index.get({
      protocol: 'http',
      host: 'aihub.localhost',
      method: 'GET',
      path: '/v1/models'
    })).toEqual(aihub);
    expect(index.get({
      protocol: 'http',
      host: 'unknown.localhost',
      method: 'GET',
      path: '/v1/models'
    })).toBeUndefined();
  });

  it('retains only exact deployment, gateway identity, adapter, mode and timeout facts', () => {
    const value = binding('stream.localhost', 'stream', '3', {
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

  it('fails closed for duplicates and keeps WebSocket selectors method-free', () => {
    const first = binding('echo.localhost', 'echo', '4');
    const duplicate = structuredClone(first);
    duplicate.selector.host = 'ECHO.LOCALHOST';
    duplicate.selector.method = 'get';

    expect(() => new RuntimeAssemblyIngressIndex([first, duplicate])).toThrow(
      /duplicate gateway ingress/
    );
    expect(runtimeAssemblyIngressKey({
      protocol: 'webSocket',
      host: 'echo.localhost',
      method: null,
      path: '/echo'
    })).toBe('webSocket\u0000echo.localhost\u0000/echo');
    expect(() => runtimeAssemblyIngressKey({
      protocol: 'webSocket',
      host: 'echo.localhost',
      method: 'GET',
      path: '/echo'
    } as never)).toThrow(/method must be null/);
  });

  it.each([
    {
      host: '',
      method: 'POST',
      path: '/echo'
    },
    {
      host: 'user@echo.localhost',
      method: 'POST',
      path: '/echo'
    },
    {
      host: 'echo.localhost',
      method: '',
      path: '/echo'
    },
    {
      host: 'echo.localhost',
      method: 'POST',
      path: 'echo'
    },
    {
      host: 'echo.localhost',
      method: 'POST',
      path: '/echo?query=true'
    }
  ])('rejects invalid canonical HTTP selector %#', (selector) => {
    expect(() => runtimeAssemblyIngressKey({
      protocol: 'http',
      ...selector
    })).toThrow();
  });
});

function binding(
  host: string,
  gatewayEntryKey: string,
  identityCharacter: string,
  options: {
    adapterKind?: 'rawHttp' | 'typedJson';
    operationMode?: 'unary' | 'serverStream';
    timeoutMs?: number;
  } = {}
): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'http',
      host,
      method: host.includes('stream') ? 'POST' : 'GET',
      path: host.includes('stream') ? '/stream' : '/v1/models'
    },
    deployment: {
      serviceId: `service/${host}`,
      contractVersion: '1.0.0',
      deploymentRevision: 'revision',
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v2:sha256:${identityCharacter.repeat(64)}`
    },
    gatewayEntryKey,
    gatewayEntryIdentity:
      `skiff-gateway-entry-v1:sha256:${identityCharacter.repeat(64)}`,
    adapterKind: options.adapterKind ?? 'typedJson',
    operationMode: options.operationMode ?? 'unary',
    ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs })
  };
}
