import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { stableStringify } from '../../src/manifest/identity.js';
import { serviceIdPathSegments } from '../../src/artifacts/pathProjection.js';

const execFileAsync = promisify(execFile);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const compilerManifestPath = join(repoRoot, 'compiler/Cargo.toml');
const websocketFixturePath = join(
  repoRoot,
  'compiler/tests/fixtures/router-websocket-fixture'
);

export interface CompilerGeneratedArtifactRoot {
  root: string;
  buildId: string;
  serviceId: string;
  serviceVersion: string;
  serviceAssembly: {
    assemblyIdentity: string;
    assemblyPath: string;
  };
  serviceUnit: {
    unitPath: string;
    schemaVersion?: string;
    unitIdentity?: string;
    unitHash?: string;
  };
  contractIdentity: string;
}

export async function writeCompilerGeneratedWebSocketFixtureArtifactRoot(
  root: string
): Promise<CompilerGeneratedArtifactRoot> {
  await execFileAsync(
    'cargo',
    [
      'run',
      '--quiet',
      '--manifest-path',
      compilerManifestPath,
      '--',
      websocketFixturePath,
      '--out',
      join(root, 'service-assembly.json'),
      '--artifact-root',
      root
    ],
    { cwd: repoRoot }
  );

  const index = await readSingleCompilerArtifactIndex(root);
  const serviceAssembly = readServiceAssemblyPointer(index);
  const serviceUnit = readServiceUnitPointer(index);
  const serviceId = readRequiredString(index.serviceId, 'compiler artifact index serviceId');
  const contractIdentity = readRequiredString(
    index.contractIdentity,
    'compiler artifact index contractIdentity'
  );
  const serviceVersion = `${serviceIdPathSegments(serviceId).join('-')}-compiler-fixture`;
  const buildId = `skiff-service-build-v1:sha256:${identityHash(
    fixtureIdentity('skiff-service-build-v1', stableStringify(index))
  )}`;

  await writeVersionPointer(root, { buildId, serviceId, version: serviceVersion });
  await writeBuildRecord(root, {
    buildId,
    serviceId,
    serviceVersion,
    contractIdentity,
    serviceAssembly,
    serviceUnit
  });

  return {
    root,
    buildId,
    serviceId,
    serviceVersion,
    serviceAssembly,
    serviceUnit,
    contractIdentity
  };
}

export async function writeCompilerGeneratedWebSocketFixtureDevReloadArtifactRoot(
  root: string,
  profile = 'prod'
): Promise<CompilerGeneratedArtifactRoot> {
  const generated = await writeCompilerGeneratedWebSocketFixtureArtifactRoot(root);
  const buildId = `skiff-service-build-v1:sha256:${identityHash(
    generated.serviceAssembly.assemblyIdentity
  )}`;
  const devPointerPath = serviceIdJsonPath(root, ['dev', 'services'], generated.serviceId);
  await mkdir(dirname(devPointerPath), { recursive: true });
  await writeFile(
    devPointerPath,
    JSON.stringify(
      {
        mode: 'dev',
        serviceId: generated.serviceId,
        profile,
        contractHash: identityHash(generated.contractIdentity),
        protocolIdentity: generated.contractIdentity,
        buildId,
        serviceAssembly: generated.serviceAssembly,
        serviceUnit: generated.serviceUnit
      },
      null,
      2
    )
  );

  return {
    ...generated,
    buildId
  };
}

async function readSingleCompilerArtifactIndex(root: string): Promise<Record<string, unknown>> {
  const indexRoot = join(root, 'indexes');
  const paths = await readJsonFilesRecursive(indexRoot);
  const indexes: Array<Record<string, unknown>> = [];
  for (const path of paths) {
    let candidate: Record<string, unknown>;
    try {
      candidate = JSON.parse(await readFile(path, 'utf8')) as Record<string, unknown>;
    } catch {
      continue;
    }
    if (
      candidate.schemaVersion === 'skiff-artifact-index-v1' &&
      candidate.serviceAssembly !== undefined
    ) {
      indexes.push(candidate);
    }
  }
  if (indexes.length !== 1) {
    throw new Error(
      `expected one compiler artifact index in ${indexRoot}, found ${indexes.length}`
    );
  }
  return indexes[0]!;
}

async function readJsonFilesRecursive(root: string): Promise<string[]> {
  const entries = await readdir(root, { withFileTypes: true });
  const paths: string[] = [];
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      paths.push(...(await readJsonFilesRecursive(path)));
    } else if (entry.isFile() && entry.name.endsWith('.json')) {
      paths.push(path);
    }
  }
  return paths;
}

function readServiceAssemblyPointer(index: Record<string, unknown>): {
  assemblyIdentity: string;
  assemblyPath: string;
} {
  const serviceAssembly = index.serviceAssembly;
  if (!serviceAssembly || typeof serviceAssembly !== 'object') {
    throw new Error('compiler artifact index serviceAssembly must be an object');
  }
  const record = serviceAssembly as Record<string, unknown>;
  return {
    assemblyIdentity: readRequiredString(
      record.assemblyIdentity,
      'compiler artifact index serviceAssembly.assemblyIdentity'
    ),
    assemblyPath: readRequiredString(
      record.assemblyPath,
      'compiler artifact index serviceAssembly.assemblyPath'
    )
  };
}

function readServiceUnitPointer(index: Record<string, unknown>): {
  unitPath: string;
  schemaVersion?: string;
  unitIdentity?: string;
  unitHash?: string;
} {
  const serviceUnit = index.serviceUnit;
  if (!serviceUnit || typeof serviceUnit !== 'object') {
    throw new Error('compiler artifact index serviceUnit must be an object');
  }
  const record = serviceUnit as Record<string, unknown>;
  const pointer: {
    unitPath: string;
    schemaVersion?: string;
    unitIdentity?: string;
    unitHash?: string;
  } = {
    unitPath: readRequiredString(
      record.unitPath,
      'compiler artifact index serviceUnit.unitPath'
    )
  };
  const schemaVersion = readOptionalString(record.schemaVersion);
  if (schemaVersion !== undefined) {
    pointer.schemaVersion = schemaVersion;
  }
  const unitIdentity = readOptionalString(record.unitIdentity);
  if (unitIdentity !== undefined) {
    pointer.unitIdentity = unitIdentity;
  }
  const unitHash = readOptionalString(record.unitHash);
  if (unitHash !== undefined) {
    pointer.unitHash = unitHash;
  }
  return pointer;
}

async function writeVersionPointer(
  root: string,
  version: { buildId: string; serviceId: string; version: string }
) {
  const serviceIdSegments = serviceIdPathSegments(version.serviceId);
  await mkdir(join(root, 'versions', 'services', ...serviceIdSegments), { recursive: true });
  await writeFile(
    join(root, 'versions', 'services', ...serviceIdSegments, `${version.version}.json`),
    JSON.stringify(
      {
        schemaVersion: 'skiff-service-version-pointer-v1',
        serviceId: version.serviceId,
        version: version.version,
        buildId: version.buildId,
        updatedAt: '2026-05-05T00:00:00.000Z',
        updatedBy: 'compiler-fixture-test'
      },
      null,
      2
    )
  );
}

async function writeBuildRecord(
  root: string,
  input: {
    buildId: string;
    serviceId: string;
    serviceVersion: string;
    contractIdentity: string;
    serviceAssembly: {
      assemblyIdentity: string;
      assemblyPath: string;
    };
    serviceUnit: {
      unitPath: string;
      schemaVersion?: string;
      unitIdentity?: string;
      unitHash?: string;
    };
  }
) {
  const serviceIdSegments = serviceIdPathSegments(input.serviceId);
  await mkdir(join(root, 'builds', 'services', ...serviceIdSegments), { recursive: true });
  await writeFile(
    join(root, 'builds', 'services', ...serviceIdSegments, `${identityHash(input.buildId)}.json`),
    JSON.stringify(
      {
        schemaVersion: 'skiff-service-build-v1',
        serviceId: input.serviceId,
        serviceVersion: input.serviceVersion,
        buildId: input.buildId,
        contractIdentity: input.contractIdentity,
        serviceAssembly: input.serviceAssembly,
        serviceUnit: input.serviceUnit,
        fingerprint: input.serviceAssembly.assemblyIdentity,
        createdAt: '2026-05-05T00:00:00.000Z'
      },
      null,
      2
    )
  );
}

function readRequiredString(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function readOptionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function fixtureIdentity(prefix: string, seed: string): string {
  return `${prefix}:sha256:${createHash('sha256').update(seed).digest('hex')}`;
}

function identityHash(identity: string): string {
  const marker = ':sha256:';
  const index = identity.lastIndexOf(marker);
  return index === -1 ? identity : identity.slice(index + marker.length);
}

function serviceIdJsonPath(root: string, prefix: string[], serviceId: string): string {
  const segments = serviceIdPathSegments(serviceId);
  const lastSegment = segments.at(-1);
  if (lastSegment === undefined) {
    throw new Error(`serviceId ${serviceId} must have at least one path segment`);
  }
  return join(root, ...prefix, ...segments.slice(0, -1), `${lastSegment}.json`);
}
