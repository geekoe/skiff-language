import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import { sha256Hex, stableStringify } from '../src/manifest/identity.js';

const roots: string[] = [];
const protocolIdentity = serviceProtocolIdentity();
const assemblyIdentity = runtimeAssemblyIdentity();
const assemblyPath = `records/runtime-assemblies/${identityHash(assemblyIdentity)}.json`;
const contractPath =
  `records/service-contracts/skiff~drun~secho/1.0.0/${identityHash(protocolIdentity)}.json`;

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('filesystem RuntimeAssembly snapshot loader', () => {
  it('loads the exact immutable assembly and contract records', async () => {
    const root = await fixtureRoot();
    await writeJson(root, assemblyPath, assembly());
    await writeJson(root, contractPath, contract());

    await expect(loader(root).load({ assemblyIdentity })).resolves.toMatchObject({
      assemblyIdentity,
      resolvedContracts: [{
        serviceId: 'skiff.run/echo',
        contractVersion: '1.0.0',
        serviceProtocolIdentity: protocolIdentity
      }],
      globalIngress: []
    });
  });

  it('fails closed for missing, malformed and identity-mismatched records', async () => {
    const missing = await fixtureRoot();
    await expect(loader(missing).load({ assemblyIdentity })).rejects.toThrow(/unavailable/);

    const malformed = await fixtureRoot();
    await writeText(
      malformed,
      assemblyPath,
      `{"assemblyIdentity":${JSON.stringify(assemblyIdentity)},"assemblyIdentity":"duplicate"}`
    );
    await expect(loader(malformed).load({ assemblyIdentity })).rejects.toThrow(/strict JSON/);

    const mismatched = await fixtureRoot();
    await writeJson(mismatched, assemblyPath, {
      ...assembly(),
      assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'c'.repeat(64)}`
    });
    await expect(loader(mismatched).load({ assemblyIdentity })).rejects.toThrow(
      /identity does not match/
    );
  });

  it('rejects records whose identity-bearing content was corrupted', async () => {
    const root = await fixtureRoot();
    await writeJson(root, assemblyPath, { ...assembly(), roots: [{ tampered: true }] });
    await writeJson(root, contractPath, contract());

    await expect(loader(root).load({ assemblyIdentity })).rejects.toThrow(
      /content does not match/
    );
  });

  it('fails closed when a referenced contract is absent or mismatched', async () => {
    const missing = await fixtureRoot();
    await writeJson(missing, assemblyPath, assembly());
    await expect(loader(missing).load({ assemblyIdentity })).rejects.toThrow(/ServiceContract.*unavailable/);

    const mismatched = await fixtureRoot();
    await writeJson(mismatched, assemblyPath, assembly());
    await writeJson(mismatched, contractPath, {
      ...contract(),
      serviceProtocolIdentity: `skiff-service-protocol-v2:sha256:${'c'.repeat(64)}`
    });
    await expect(loader(mismatched).load({ assemblyIdentity })).rejects.toThrow(
      /identity does not match/
    );
  });

  it('rejects record symlinks that escape artifactsPath', async () => {
    const root = await fixtureRoot();
    const outside = await fixtureRoot();
    const outsideAssembly = join(outside, 'assembly.json');
    await writeFile(outsideAssembly, JSON.stringify(assembly()));
    const target = join(root, assemblyPath);
    await mkdir(dirname(target), { recursive: true });
    await symlink(outsideAssembly, target);

    await expect(loader(root).load({ assemblyIdentity })).rejects.toThrow(/escapes artifactsPath/);
  });
});

function loader(root: string): FilesystemRuntimeAssemblySnapshotLoader {
  return new FilesystemRuntimeAssemblySnapshotLoader(root);
}

async function fixtureRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-snapshot-'));
  roots.push(root);
  return root;
}

async function writeJson(root: string, path: string, value: unknown): Promise<void> {
  await writeText(root, path, JSON.stringify(value));
}

async function writeText(root: string, path: string, value: string): Promise<void> {
  const target = join(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, value);
}

function assembly(): Record<string, unknown> {
  return {
    schemaVersion: 'skiff-runtime-assembly-v1',
    assemblyIdentity,
    roots: [],
    resolvedDeployments: [],
    resolvedContracts: [{
      serviceId: 'skiff.run/echo',
      contractVersion: '1.0.0',
      serviceProtocolIdentity: protocolIdentity
    }],
    resolvedPackages: [],
    packageLinkPlan: { links: [] },
    serviceBindingTemplates: [],
    activationTemplates: [],
    globalIngress: []
  };
}

function contract(): Record<string, unknown> {
  return {
    schemaVersion: 'skiff-service-contract-v2',
    serviceId: 'skiff.run/echo',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: protocolIdentity,
    operations: {},
    boundarySchema: {},
    diagnosticText: null
  };
}

function serviceProtocolIdentity(): string {
  return `skiff-service-protocol-v2:sha256:${sha256Hex(stableStringify({
    schema: 'skiff-service-protocol-identity-v2',
    serviceId: 'skiff.run/echo',
    contractVersion: '1.0.0',
    operations: {},
    boundarySchema: {}
  }))}`;
}

function runtimeAssemblyIdentity(): string {
  return `skiff-runtime-assembly-v1:sha256:${sha256Hex(stableStringify({
    schema: 'skiff-runtime-assembly-identity-v1',
    roots: [],
    resolvedDeployments: [],
    resolvedContracts: [{
      serviceId: 'skiff.run/echo',
      contractVersion: '1.0.0',
      serviceProtocolIdentity: protocolIdentity
    }],
    resolvedPackages: [],
    packageLinkPlan: { links: [] },
    serviceBindingTemplates: [],
    activationTemplates: [],
    globalIngress: []
  }))}`;
}

function identityHash(value: string): string {
  return value.slice(value.lastIndexOf(':') + 1);
}
