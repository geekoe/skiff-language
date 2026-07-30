import { describe, expect, it } from 'vitest';

import { RuntimeAssemblyActorMethodCatalog } from '../src/router/runtimeAssemblyActorMethodCatalog.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
} from '../src/router/runtimeAssemblySnapshot.js';

describe('RuntimeAssembly Actor method catalog', () => {
  it('switches the exact owner/ABI/implementation/method index atomically with the snapshot', () => {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    const catalog = new RuntimeAssemblyActorMethodCatalog(snapshots);
    const method = {
      declarationOwner: {
        unit: { kind: 'service' as const },
        file: { kind: 'fileIrIdentity' as const, value: 'file-1' },
        actorSymbol: 'example.Counter',
      },
      actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', 'a'),
      actorImplementationIdentity: identity(
        'skiff-actor-implementation-v1:sha256',
        'b'
      ),
      methodIdentity: identity('skiff-actor-method-v1:sha256', 'c'),
    };
    snapshots.replace({
      environment: 'test',
      generation: 1,
      assembly: { assemblyIdentity: identity('skiff-runtime-assembly-v3:sha256', 'd') },
      configSnapshot: {
        snapshotId:
          'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      },
      ingress: new RuntimeAssemblyIngressIndex([]),
      actorMethods: [method],
    });
    expect(catalog.hasMethod(method)).toBe(true);
    expect(catalog.hasMethod({ ...method, methodIdentity: identity('skiff-actor-method-v1:sha256', 'e') })).toBe(false);

    snapshots.replace({
      environment: 'test',
      generation: 2,
      assembly: { assemblyIdentity: identity('skiff-runtime-assembly-v3:sha256', 'f') },
      configSnapshot: {
        snapshotId:
          'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      },
      ingress: new RuntimeAssemblyIngressIndex([]),
      actorMethods: [],
    });
    expect(catalog.hasMethod(method)).toBe(false);
  });
});

function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
