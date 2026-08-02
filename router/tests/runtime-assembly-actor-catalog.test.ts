import { describe, expect, it } from 'vitest';

import { RuntimeAssemblyActorMethodCatalog } from '../src/router/runtimeAssemblyActorMethodCatalog.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
} from '../src/router/runtimeAssemblySnapshot.js';

/**
 * TS-side differential baseline for E-actor-parity: the catalog admission key
 * is the A0 typed key `{serviceId, actorAbiIdentity,
 * actorImplementationIdentity, methodIdentity}`, matching the Rust
 * `ActorMethodCatalogView::CatalogQuery` (C-model-actor §3). `declarationOwner`
 * and File IR coordinates never participate.
 */
describe('RuntimeAssembly Actor method catalog', () => {
  it('switches the exact service/ABI/implementation/method index atomically with the snapshot', () => {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    const catalog = new RuntimeAssemblyActorMethodCatalog(snapshots);
    const method = actorMethod('example.com/docs', 'a', 'b', 'c');
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
    expect(catalog.hasMethod(query('example.com/docs', 'a', 'b', 'c'))).toBe(true);
    expect(
      catalog.hasMethod(query('example.com/docs', 'a', 'b', 'e'))
    ).toBe(false);
    expect(
      catalog.hasMethod(query('example.com/other', 'a', 'b', 'c'))
    ).toBe(false);
    expect(
      catalog.hasMethod(query('example.com/docs', 'a', '8', 'c'))
    ).toBe(false);
    expect(
      catalog.hasMethod(query('example.com/docs', '9', 'b', 'c'))
    ).toBe(false);

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
    expect(catalog.hasMethod(query('example.com/docs', 'a', 'b', 'c'))).toBe(false);
  });

  it('matches the Rust catalog semantics on a multi-entry projection', () => {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    const catalog = new RuntimeAssemblyActorMethodCatalog(snapshots);
    const first = actorMethod('example.com/docs', 'a', 'b', 'c');
    const second = actorMethod(
      'example.com/docs',
      'a',
      'b',
      'f',
      'pkg-b',
      '2.0.0'
    );
    const third = actorMethod('example.com/docs', '9', 'b', 'c', 'pkg-b');
    snapshots.replace({
      environment: 'test',
      generation: 1,
      assembly: { assemblyIdentity: identity('skiff-runtime-assembly-v3:sha256', 'd') },
      configSnapshot: {
        snapshotId:
          'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      },
      ingress: new RuntimeAssemblyIngressIndex([]),
      actorMethods: [first, second, third],
    });
    expect(catalog.hasMethod(query('example.com/docs', 'a', 'b', 'c'))).toBe(true);
    expect(catalog.hasMethod(query('example.com/docs', 'a', 'b', 'f'))).toBe(true);
    expect(catalog.hasMethod(query('example.com/docs', '9', 'b', 'c'))).toBe(true);
    expect(
      catalog.hasMethod(query('example.com/docs', 'a', '8', 'f'))
    ).toBe(false);
  });
});

function actorMethod(
  serviceId: string,
  abiDigit: string,
  implementationDigit: string,
  methodDigit: string,
  packageId = 'example.com/docs-package',
  packageVersion = '1.0.0'
) {
  return {
    actor: {
      serviceId,
      actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', abiDigit),
    },
    actorImplementationIdentity: identity(
      'skiff-actor-implementation-v1:sha256',
      implementationDigit
    ),
    methodIdentity: identity('skiff-actor-method-v1:sha256', methodDigit),
    deployment: {
      serviceId,
      contractVersion: '1.0.0',
      deploymentRevision: 'rev-1',
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v4:sha256:${'d'.repeat(64)}`,
    },
    package: {
      packageId,
      packageVersion,
      packageBuildId: `skiff-package-build-v10:sha256:${'e'.repeat(64)}`,
      packageLocalAbiIdentity:
        `skiff-package-local-abi-v7:sha256:${'f'.repeat(64)}`,
    },
  };
}

function query(
  serviceId: string,
  abiDigit: string,
  implementationDigit: string,
  methodDigit: string
) {
  return {
    serviceId,
    actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', abiDigit),
    actorImplementationIdentity: identity(
      'skiff-actor-implementation-v1:sha256',
      implementationDigit
    ),
    methodIdentity: identity('skiff-actor-method-v1:sha256', methodDigit),
  };
}

function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
