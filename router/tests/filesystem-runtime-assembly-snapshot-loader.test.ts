import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';

const roots: string[] = [];
const ASSEMBLY_IDENTITY =
  `skiff-runtime-assembly-v2:sha256:${'a'.repeat(64)}`;
const SERVICE_PROTOCOL_IDENTITY =
  `skiff-service-protocol-v4:sha256:${'b'.repeat(64)}`;
const GATEWAY_IDENTITIES = [
  `skiff-gateway-entry-v1:sha256:${'1'.repeat(64)}`,
  `skiff-gateway-entry-v1:sha256:${'2'.repeat(64)}`,
  `skiff-gateway-entry-v1:sha256:${'3'.repeat(64)}`
] as const;

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('filesystem RuntimeAssembly snapshot loader', () => {
  it('loads raw unary, raw server stream and typed unary from exact deployments', async () => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    await writeFixture(root, fixture);

    const loaded = await loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY });

    expect(loaded).toMatchObject({
      schemaVersion: 'skiff-runtime-assembly-v2',
      assemblyIdentity: ASSEMBLY_IDENTITY,
      resolvedContracts: [{
        serviceId: 'skiff.run/echo',
        contractVersion: '1.0.0',
        serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
      }],
      gatewayIngress: [
        {
          gatewayEntryKey: 'rawUnary',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[0],
          adapterKind: 'rawHttp',
          operationMode: 'unary'
        },
        {
          gatewayEntryKey: 'rawStream',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[1],
          adapterKind: 'rawHttp',
          operationMode: 'serverStream',
          timeoutMs: 2_500
        },
        {
          gatewayEntryKey: 'typedUnary',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[2],
          adapterKind: 'typedJson',
          operationMode: 'unary'
        }
      ]
    });
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('timeoutMs');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('handler');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('adapterPlan');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('contractOperationId');
  });

  it('uses declared v2 identities without recomputing Rust-owned content hashes', async () => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    fixture.assembly.roots = [...fixture.assembly.roots].reverse();
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).resolves.toMatchObject({ assemblyIdentity: ASSEMBLY_IDENTITY });
  });

  it('rejects a v1 RuntimeAssembly identity prefix before artifact lookup', async () => {
    const root = await fixtureRoot();
    await expect(loader(root).load({
      assemblyIdentity:
        `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`
    })).rejects.toThrow(/reference identity is invalid/);
  });

  it('fails closed for missing, malformed, mismatched and escaping records', async () => {
    const missing = await fixtureRoot();
    await expect(
      loader(missing).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/unavailable/);

    const malformed = await fixtureRoot();
    await writeText(
      malformed,
      assemblyPath(),
      `{"assemblyIdentity":${JSON.stringify(ASSEMBLY_IDENTITY)},"assemblyIdentity":"duplicate"}`
    );
    await expect(
      loader(malformed).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/strict JSON/);

    const mismatched = await fixtureRoot();
    const fixture = canonicalFixture();
    await writeFixture(mismatched, fixture);
    fixture.deployments[0]!.deploymentArtifactIdentity =
      `skiff-deployment-artifact-v2:sha256:${'f'.repeat(64)}`;
    await writeJson(
      mismatched,
      deploymentPath(deploymentRef(fixture.assembly, 0)),
      fixture.deployments[0]!
    );
    await expect(
      loader(mismatched).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/exact ServiceDeployment reference/);

    const escaping = await fixtureRoot();
    const outside = await fixtureRoot();
    const outsideAssembly = join(outside, 'assembly.json');
    await writeFile(outsideAssembly, JSON.stringify(canonicalFixture().assembly));
    const target = join(escaping, assemblyPath());
    await mkdir(dirname(target), { recursive: true });
    await symlink(outsideAssembly, target);
    await expect(
      loader(escaping).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/escapes artifactsPath/);
  });

  it.each([
    {
      name: 'v1 schema',
      mutate: (fixture: Fixture) => {
        fixture.assembly.schemaVersion = 'skiff-runtime-assembly-v1';
      }
    },
    {
      name: 'globalIngress',
      mutate: (fixture: Fixture) => {
        fixture.assembly.globalIngress = fixture.assembly.gatewayIngress;
        delete fixture.assembly.gatewayIngress;
      }
    },
    {
      name: 'contract operation',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].contractOperationId =
          `skiff-contract-operation-v1:sha256:${'c'.repeat(64)}`;
      }
    },
    {
      name: 'WebSocket selector',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].selector.protocol = 'webSocket';
        fixture.assembly.gatewayIngress[0].selector.method = null;
      }
    }
  ])('rejects legacy RuntimeAssembly surface: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it.each([
    {
      name: 'wrong key',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].gatewayEntryKey = 'other';
      }
    },
    {
      name: 'wrong identity',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].gatewayEntryIdentity =
          GATEWAY_IDENTITIES[2];
      }
    },
    {
      name: 'missing assembly selector',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress.pop();
      }
    },
    {
      name: 'extra assembly selector',
      mutate: (fixture: Fixture) => {
        const extra = structuredClone(fixture.assembly.gatewayIngress[0]);
        extra.selector.path = '/extra';
        fixture.assembly.gatewayIngress.push(extra);
      }
    },
    {
      name: 'duplicate assembly selector',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress.push(
          structuredClone(fixture.assembly.gatewayIngress[0])
        );
      }
    },
    {
      name: 'missing deployment key',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.ingress[0].gatewayEntryKey = 'missing';
      }
    },
    {
      name: 'duplicate deployment selector',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.ingress.push(
          structuredClone(fixture.deployments[0]!.ingress[0])
        );
      }
    }
  ])('rejects an inexact assembly/deployment join: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it.each([
    {
      name: 'contract service',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.contract.serviceId = 'skiff.run/other';
      }
    },
    {
      name: 'contract version',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.contract.contractVersion = '2.0.0';
      }
    },
    {
      name: 'revision',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.deploymentRevision = 'other-revision';
      }
    },
    {
      name: 'deployment identity',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.deploymentArtifactIdentity =
          `skiff-deployment-artifact-v2:sha256:${'f'.repeat(64)}`;
      }
    }
  ])('rejects a record with mismatched exact reference field: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    const reference = deploymentRef(fixture.assembly, 0);
    mutate(fixture.deployments[0]!);
    await writeFixture(root, fixture);
    await writeJson(
      root,
      deploymentPath(reference),
      fixture.deployments[0]!
    );

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/exact ServiceDeployment reference/);
  });

  it.each([
    {
      name: 'typed JSON server stream',
      mutate: (fixture: Fixture) => {
        const surface = httpSurface(fixture.deployments[2]!, 'typedUnary');
        surface.dispatchMode = 'serverStream';
        surface.responseSchema = null;
        surface.streamItemSchema = { kind: 'string' };
      }
    },
    {
      name: 'non-HTTP protocol',
      mutate: (fixture: Fixture) => {
        protocol(fixture.deployments[0]!, 'rawUnary').kind = 'webSocket';
      }
    },
    {
      name: 'adapter kind mismatch',
      mutate: (fixture: Fixture) => {
        gatewayEntry(fixture.deployments[0]!, 'rawUnary').adapterPlan.kind =
          'typedJson';
      }
    },
    {
      name: 'unary stream schema',
      mutate: (fixture: Fixture) => {
        httpSurface(fixture.deployments[0]!, 'rawUnary').streamItemSchema = {
          kind: 'string'
        };
      }
    },
    {
      name: 'zero timeout',
      mutate: (fixture: Fixture) => {
        fixture.deployments[1]!.policy.timeoutMs = 0;
      }
    },
    {
      name: 'null timeout',
      mutate: (fixture: Fixture) => {
        fixture.deployments[1]!.policy.timeoutMs = null;
      }
    }
  ])('rejects invalid deployment mode or policy: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });
});

interface Fixture {
  assembly: Record<string, any>;
  deployments: Array<Record<string, any>>;
}

function canonicalFixture(): Fixture {
  const deployments = [
    deployment('raw-unary', 'rawUnary', GATEWAY_IDENTITIES[0], {
      adapterKind: 'rawHttp',
      operationMode: 'unary',
      path: '/raw'
    }),
    deployment('raw-stream', 'rawStream', GATEWAY_IDENTITIES[1], {
      adapterKind: 'rawHttp',
      operationMode: 'serverStream',
      path: '/stream',
      timeoutMs: 2_500
    }),
    deployment('typed-unary', 'typedUnary', GATEWAY_IDENTITIES[2], {
      adapterKind: 'typedJson',
      operationMode: 'unary',
      path: '/typed'
    })
  ];
  const references = deployments.map((value) => ({
    serviceId: value.contract.serviceId,
    contractVersion: value.contract.contractVersion,
    deploymentRevision: value.deploymentRevision,
    deploymentArtifactIdentity: value.deploymentArtifactIdentity
  }));
  return {
    assembly: {
      schemaVersion: 'skiff-runtime-assembly-v2',
      assemblyIdentity: ASSEMBLY_IDENTITY,
      roots: references,
      resolvedDeployments: references,
      resolvedContracts: [{
        serviceId: 'skiff.run/echo',
        contractVersion: '1.0.0',
        serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
      }],
      resolvedPackages: [],
      packageLinkPlan: { codeSlots: [], packageLinks: [] },
      serviceBindingTemplates: [],
      activationTemplates: [],
      gatewayIngress: deployments.map((value, index) => ({
        selector: structuredClone(value.ingress[0].selector),
        deployment: references[index],
        gatewayEntryKey: value.ingress[0].gatewayEntryKey,
        gatewayEntryIdentity:
          value.gatewayEntries[value.ingress[0].gatewayEntryKey].gatewayEntryIdentity
      }))
    },
    deployments
  };
}

function deployment(
  revision: string,
  gatewayEntryKey: string,
  gatewayEntryIdentity: string,
  options: {
    adapterKind: 'rawHttp' | 'typedJson';
    operationMode: 'unary' | 'serverStream';
    path: string;
    timeoutMs?: number;
  }
): Record<string, any> {
  const typed = options.adapterKind === 'typedJson';
  const stream = options.operationMode === 'serverStream';
  return {
    schemaVersion: 'skiff-service-deployment-v2',
    contract: {
      serviceId: 'skiff.run/echo',
      contractVersion: '1.0.0',
      serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
    },
    deploymentRevision: revision,
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v2:sha256:${(
        gatewayEntryKey === 'rawUnary'
          ? '4'
          : gatewayEntryKey === 'rawStream'
            ? '5'
            : '6'
      ).repeat(64)}`,
    implementation: {
      packageId: 'skiff.run/echo',
      packageVersion: '1.0.0',
      packageBuildId: `skiff-package-build-v8:sha256:${'d'.repeat(64)}`,
      packageLocalAbiIdentity:
        `skiff-package-local-abi-v4:sha256:${'e'.repeat(64)}`
    },
    operationBindings: [],
    packageBindings: [],
    serviceSelectors: [],
    gatewayEntries: {
      [gatewayEntryKey]: {
        gatewayEntryIdentity,
        protocolSurface: {
          protocol: {
            kind: 'http',
            surface: {
              adapterKind: options.adapterKind,
              dispatchMode: options.operationMode,
              externalSources: [{
                kind: typed ? 'http.body' : 'http.request'
              }],
              requestBodySchema: typed ? { kind: 'string' } : null,
              responseSchema: typed ? { kind: 'string' } : null,
              streamItemSchema: stream ? { kind: 'string' } : null
            }
          },
          externalErrorProjection: { kind: 'fixed', version: 'v1' }
        },
        handler: `skiff-package-callable-v1:sha256:${'f'.repeat(64)}`,
        pre: null,
        guard: null,
        adapterPlan: {
          kind: options.adapterKind,
          args: [{
            param: typed ? 'body' : 'request',
            source: { kind: typed ? 'http.body' : 'http.request' }
          }]
        }
      }
    },
    ingress: [{
      selector: {
        protocol: 'http',
        host: 'echo.example.test',
        method: 'POST',
        path: options.path
      },
      gatewayEntryKey
    }],
    configLiterals: [],
    secretRefs: [],
    stateBindings: [],
    resourceBindings: [],
    runtimeCapabilityBindings: [],
    policy: {
      ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs }),
      resources: { cpuMillis: 100, memoryBytes: 1_048_576 },
      activation: { maxConcurrency: 8, idleTimeoutMs: null },
      principal: 'service:skiff.run/echo'
    },
    diagnosticText: { displayName: gatewayEntryKey, notes: {} }
  };
}

function gatewayEntry(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return deploymentRecord.gatewayEntries[key];
}

function protocol(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return gatewayEntry(deploymentRecord, key).protocolSurface.protocol;
}

function httpSurface(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return protocol(deploymentRecord, key).surface;
}

function loader(root: string): FilesystemRuntimeAssemblySnapshotLoader {
  return new FilesystemRuntimeAssemblySnapshotLoader(root);
}

async function fixtureRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-snapshot-'));
  roots.push(root);
  return root;
}

async function writeFixture(root: string, fixture: Fixture): Promise<void> {
  await writeJson(root, assemblyPath(), fixture.assembly);
  for (const [index, deploymentRecord] of fixture.deployments.entries()) {
    await writeJson(
      root,
      deploymentPath(deploymentRef(fixture.assembly, index)),
      deploymentRecord
    );
  }
}

function deploymentRef(
  assembly: Record<string, any>,
  index: number
): DeploymentRefFixture {
  return assembly.resolvedDeployments[index] as DeploymentRefFixture;
}

function assemblyPath(): string {
  return `records/runtime-assemblies/${identityHash(ASSEMBLY_IDENTITY)}.json`;
}

function deploymentPath(reference: DeploymentRefFixture): string {
  return [
    'records/service-deployments',
    reference.serviceId.replaceAll('.', '~d').replaceAll('/', '~s'),
    reference.contractVersion,
    reference.deploymentRevision,
    `${identityHash(reference.deploymentArtifactIdentity)}.json`
  ].join('/');
}

interface DeploymentRefFixture {
  serviceId: string;
  contractVersion: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
}

async function writeJson(root: string, path: string, value: unknown): Promise<void> {
  await writeText(root, path, JSON.stringify(value));
}

async function writeText(root: string, path: string, value: string): Promise<void> {
  const target = join(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, value);
}

function identityHash(value: string): string {
  return value.slice(value.lastIndexOf(':') + 1);
}
