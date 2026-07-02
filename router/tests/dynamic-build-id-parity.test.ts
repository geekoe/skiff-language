import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  computeRuntimeProgramBuildId,
  readRuntimeProgramServiceUnit
} from '../src/artifacts/dynamicBuildId.js';
import { computeRuntimeProgramBuildIdWithIdentityCli } from '../src/artifacts/identityCli.js';
import { writeMockIdentityCli } from './helpers/mockIdentityCli.js';

type DynamicBuildIdFixture = {
  appliesTo: string[];
  serviceUnitPath: string;
  expectedDynamicBuildId: string;
  artifactRoot: Record<string, unknown>;
};

const fixturePath = new URL(
  '../../cross-system-fixtures/dynamic-build-id-parity/case.json',
  import.meta.url
);
const EXPECTED_DYNAMIC_BUILD_ID =
  'skiff-service-build-v1:sha256:7549503608c36594ee9cdc25c329ac1401cef5e7c32bcbaa919fef3df1976923';

async function dynamicBuildIdFixture(): Promise<DynamicBuildIdFixture> {
  return JSON.parse(await readFile(fixturePath, 'utf8')) as DynamicBuildIdFixture;
}

describe('dynamic build id fixture', () => {
  it('preserves the fixed cross-system fixture shape', async () => {
    const fixture = await dynamicBuildIdFixture();
    expect(fixture.appliesTo).toContain('router-cli-boundary');
    expect(fixture.expectedDynamicBuildId).toBe(EXPECTED_DYNAMIC_BUILD_ID);
    expect(fixtureContainsTypeRef(fixture.artifactRoot, 'packageSymbol', 'std.http.HttpClientRequest')).toBe(
      true
    );
    expect(
      fixtureContainsTypeRef(fixture.artifactRoot, 'packageSymbol', 'std.http.HttpResponseStreamEvent')
    ).toBe(true);
    expect(fixtureContainsTypeRef(fixture.artifactRoot, 'packageSymbol', 'std.file.ImmutableFile')).toBe(
      true
    );
    expect(fixtureContainsTypeRef(fixture.artifactRoot, 'builtin', 'bytes')).toBe(true);
    expect(fixtureServiceUnitArray(fixture, 'spawnTargets').length).toBeGreaterThan(0);
    expect(fixtureServiceUnitArray(fixture, 'actors').length).toBeGreaterThan(0);
    expect(fixtureServiceUnitTimeout(fixture)).toEqual({
      defaultMs: 120000,
      methods: {
        'managedLlmService.call': 90000,
        run: 45000
      }
    });
    expect(fixtureOperationTarget(fixture, 0).executableIndex).toBe(0);

    const root = await mkdtemp(join(tmpdir(), 'skiff-router-dynamic-build-id-'));
    try {
      await writeFixtureArtifactRoot(root, fixture.artifactRoot);
      const serviceUnit = await readRuntimeProgramServiceUnit({
        root,
        pointer: {
          indexPath: 'cross-system-fixtures/dynamic-build-id-parity/case.json',
          serviceUnit: fixture.serviceUnitPath
        },
        serviceAssembly: {}
      });
      const operations = serviceUnit.value.operations;
      if (!Array.isArray(operations)) {
        throw new Error('service unit operations should be an array');
      }
      expect(operationExecutableTarget(operations[0] as Record<string, unknown>)).toEqual(
        fixtureOperationTarget(fixture, 0)
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

describe('dynamic build id identity CLI boundary', () => {
  it('passes artifact root and service unit through the production build id path', async () => {
    const fixture = await dynamicBuildIdFixture();
    const root = await mkdtemp(join(tmpdir(), 'skiff-router-dynamic-build-id-cli-'));
    const capturePath = join(root, 'identity-cli-stdin.json');
    try {
      await writeFixtureArtifactRoot(root, fixture.artifactRoot);
      const identityCliPath = await writeMockIdentityCli({
        dir: join(root, 'bin'),
        dynamicBuildId: EXPECTED_DYNAMIC_BUILD_ID,
        capturePath,
      });
      const serviceUnit = await readRuntimeProgramServiceUnit({
        root,
        pointer: {
          indexPath: 'cross-system-fixtures/dynamic-build-id-parity/case.json',
          serviceUnit: fixture.serviceUnitPath
        },
        serviceAssembly: {}
      });

      await expect(
        computeRuntimeProgramBuildId({
          root,
          pointer: {
            indexPath: 'cross-system-fixtures/dynamic-build-id-parity/case.json',
            serviceUnit: fixture.serviceUnitPath
          },
          serviceAssembly: {},
          serviceUnit,
          identityCliPath
        })
      ).resolves.toBe(EXPECTED_DYNAMIC_BUILD_ID);

      const cliInput = JSON.parse(await readFile(capturePath, 'utf8')) as Record<string, any>;
      expect(cliInput.artifactRoot).toBe(root);
      expect(cliInput.services).toHaveLength(1);
      expect(cliInput.services[0].serviceUnit).toEqual(fixture.artifactRoot[fixture.serviceUnitPath]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it('fails closed when the identity CLI exits non-zero', async () => {
    const fixture = await dynamicBuildIdFixture();
    const root = await mkdtemp(join(tmpdir(), 'skiff-router-dynamic-build-id-error-'));
    try {
      await writeFixtureArtifactRoot(root, fixture.artifactRoot);
      const identityCliPath = await writeMockIdentityCli({
        dir: join(root, 'bin'),
        exitCode: 2,
        stderrJson: {
          error: {
            code: 'schema_invalid',
            message: 'service unit is invalid',
          },
        },
      });

      await expect(
        computeRuntimeProgramBuildIdWithIdentityCli({
          artifactRoot: root,
          serviceUnit: fixture.artifactRoot[fixture.serviceUnitPath] as Record<string, unknown>,
          identityCliPath,
        })
      ).rejects.toThrow(/schema_invalid: service unit is invalid/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it('fails closed when the identity CLI returns bad stdout', async () => {
    const fixture = await dynamicBuildIdFixture();
    const root = await mkdtemp(join(tmpdir(), 'skiff-router-dynamic-build-id-bad-stdout-'));
    try {
      await writeFixtureArtifactRoot(root, fixture.artifactRoot);
      const identityCliPath = await writeMockIdentityCli({
        dir: join(root, 'bin'),
        stdoutText: '{"results":[]}',
      });

      await expect(
        computeRuntimeProgramBuildIdWithIdentityCli({
          artifactRoot: root,
          serviceUnit: fixture.artifactRoot[fixture.serviceUnitPath] as Record<string, unknown>,
          identityCliPath,
        })
      ).rejects.toThrow(/stdout\.results must contain exactly one result/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

async function writeFixtureArtifactRoot(
  root: string,
  artifactRoot: Record<string, unknown>
): Promise<void> {
  for (const [relativePath, value] of Object.entries(artifactRoot)) {
    const path = join(root, relativePath);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, JSON.stringify(value, null, 2));
  }
}

function fixtureContainsTypeRef(
  value: unknown,
  kind: 'builtin' | 'packageSymbol',
  symbol: string
): boolean {
  if (Array.isArray(value)) {
    return value.some((item) => fixtureContainsTypeRef(item, kind, symbol));
  }
  if (!value || typeof value !== 'object') {
    return false;
  }
  const record = value as Record<string, unknown>;
  if (kind === 'builtin' && record.kind === 'builtin' && record.name === symbol) {
    return true;
  }
  if (
    kind === 'packageSymbol' &&
    record.kind === 'packageSymbol' &&
    typeSymbolPath(record.symbol) === symbol
  ) {
    return true;
  }
  return Object.values(record).some((item) => fixtureContainsTypeRef(item, kind, symbol));
}

function fixtureServiceUnitArray(
  fixture: DynamicBuildIdFixture,
  field: 'spawnTargets' | 'actors'
): unknown[] {
  const value = fixture.artifactRoot[fixture.serviceUnitPath];
  if (!value || typeof value !== 'object') {
    return [];
  }
  const fieldValue = (value as Record<string, unknown>)[field];
  return Array.isArray(fieldValue) ? fieldValue : [];
}

function fixtureServiceUnitTimeout(fixture: DynamicBuildIdFixture): unknown {
  const value = fixture.artifactRoot[fixture.serviceUnitPath];
  if (!value || typeof value !== 'object') {
    return undefined;
  }
  return (value as Record<string, unknown>).timeout;
}

function fixtureOperationTarget(
  fixture: DynamicBuildIdFixture,
  operationIndex: number
): Record<string, unknown> {
  const serviceUnit = fixture.artifactRoot[fixture.serviceUnitPath];
  if (!serviceUnit || typeof serviceUnit !== 'object') {
    throw new Error('fixture service unit should be an object');
  }
  const operations = (serviceUnit as Record<string, unknown>).operations;
  if (!Array.isArray(operations)) {
    throw new Error('fixture service unit operations should be an array');
  }
  const operation = operations[operationIndex];
  if (!operation || typeof operation !== 'object') {
    throw new Error(`fixture operation ${operationIndex} should be an object`);
  }
  const target = operationExecutableTarget(operation as Record<string, unknown>);
  if (!target || typeof target !== 'object' || Array.isArray(target)) {
    throw new Error(`fixture operation ${operationIndex} executable target should be an object`);
  }
  return target as Record<string, unknown>;
}

function operationExecutableTarget(operation: Record<string, unknown>): unknown {
  const executable = operation.executable;
  if (executable !== undefined) {
    return executable;
  }
  const receiverExecutable = operation.receiverExecutable;
  if (receiverExecutable && typeof receiverExecutable === 'object') {
    return (receiverExecutable as Record<string, unknown>).executableTarget;
  }
  return undefined;
}

function typeSymbolPath(value: unknown): string | undefined {
  if (!value || typeof value !== 'object') {
    return undefined;
  }
  const record = value as Record<string, unknown>;
  return typeof record.symbolPath === 'string' ? record.symbolPath : undefined;
}
