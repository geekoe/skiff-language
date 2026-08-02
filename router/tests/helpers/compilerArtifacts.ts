import { execFile } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import {
  ACTOR_ROUTING_PROJECTION_RECORD_PATH,
  ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
  canonicalJsonBytes,
} from '../../src/router/actorRoutingProjection.js';

const execFileAsync = promisify(execFile);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const compilerManifestPath = join(repoRoot, 'compiler/Cargo.toml');
const fixturePath = join(repoRoot, 'compiler/tests/fixtures/router-websocket-fixture');
const currentScopeFixturePath = join(
  repoRoot,
  'test-runner/fixtures/package-service-current-scope'
);
const testRunnerManifestPath = join(repoRoot, 'test-runner/Cargo.toml');

interface ObjectReceipt {
  recordPath: string;
  [key: string]: unknown;
}

interface PackageArtifactRefLike {
  packageId: string;
  packageVersion: string;
  packageBuildId: string;
  packageLocalAbiIdentity: string;
}

export interface CompilerGeneratedArtifactRoot {
  root: string;
  packageArtifact: ObjectReceipt & {
    artifact: {
      packageId: string;
      packageVersion: string;
      packageBuildId: string;
      packageLocalAbiIdentity: string;
    };
  };
  serviceContract: ObjectReceipt & {
    contract: {
      serviceId: string;
      contractVersion: string;
      serviceProtocolIdentity: string;
    };
  };
  serviceDeployment: ObjectReceipt & {
    deployment: Record<string, string>;
  };
  runtimeAssembly: ObjectReceipt & {
    environment: string;
    assembly: { assemblyIdentity: string };
  };
  packageValue: Record<string, unknown>;
  contractValue: Record<string, unknown>;
  deploymentValue: Record<string, unknown>;
  assemblyValue: Record<string, unknown>;
}

export interface CurrentScopeCompilerGeneratedArtifactRoot {
  root: string;
  receipt: {
    schemaVersion: 'skiff-package-service-host-fixture-v2';
    environment: string;
    contracts: {
      payments: Record<string, string>;
      consumer: Record<string, string>;
    };
    packages: {
      helper: Record<string, string>;
      provider: Record<string, string>;
      consumer: Record<string, string>;
    };
    deployments: {
      provider: Record<string, string>;
      consumer: Record<string, string>;
    };
    baseAssembly: { assemblyIdentity: string };
    baseConfigSnapshot: { snapshotId: string };
  };
}

export async function writeCompilerGeneratedFixtureArtifactRoot(
  root: string
): Promise<CompilerGeneratedArtifactRoot> {
  await mkdir(root, { recursive: true });
  const packageReceipt = await author('package', 'publish', fixturePath, root);
  const packageArtifact = objectReceipt(
    packageReceipt.packageArtifactReceipt,
    'packageArtifactReceipt'
  ) as CompilerGeneratedArtifactRoot['packageArtifact'];
  const serviceContract = objectReceipt(
    packageReceipt.serviceContractReceipt,
    'serviceContractReceipt'
  ) as CompilerGeneratedArtifactRoot['serviceContract'];
  const serviceDeployment = objectReceipt(
    packageReceipt.serviceDeploymentReceipt,
    'serviceDeploymentReceipt'
  ) as CompilerGeneratedArtifactRoot['serviceDeployment'];

  const assemblyReceipt = await projectRuntimeAssembly(
    root,
    'router-fixture',
    [serviceDeployment.deployment]
  );
  const runtimeAssembly = objectReceipt(
    assemblyReceipt.runtimeAssemblyReceipt,
    'runtimeAssemblyReceipt'
  ) as CompilerGeneratedArtifactRoot['runtimeAssembly'];

  await writeActorRoutingProjection(root, {
    schemaVersion: ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    methods: [],
  });

  return {
    root,
    packageArtifact,
    serviceContract,
    serviceDeployment,
    runtimeAssembly,
    packageValue: await readRecord(root, packageArtifact),
    contractValue: await readRecord(root, serviceContract),
    deploymentValue: await readRecord(root, serviceDeployment),
    assemblyValue: await readRecord(root, runtimeAssembly),
  };
}

export async function writeCurrentScopeCompilerGeneratedArtifactRoot(
  root: string,
  options: { writeActorProjection?: boolean } = {}
): Promise<CurrentScopeCompilerGeneratedArtifactRoot> {
  await mkdir(root, { recursive: true });
  const environment = 'current-scope';
  await runPackageServiceFixture([
    '--bootstrap-only',
    '--artifact-root',
    root,
    '--environment',
    environment,
    '--platform-source-root',
    repoRoot,
  ]);
  const workRoot = join(root, '.authoring', 'current-scope');
  const receiptPath = join(workRoot, 'receipt.json');
  await runPackageServiceFixture([
    '--prepare-host-base',
    currentScopeFixturePath,
    '--work-root',
    workRoot,
    '--receipt',
    receiptPath,
    '--artifact-root',
    root,
    '--environment',
    environment,
    '--platform-source-root',
    repoRoot,
  ]);
  const receipt = JSON.parse(await readFile(receiptPath, 'utf8')) as
    CurrentScopeCompilerGeneratedArtifactRoot['receipt'];
  if (
    receipt.schemaVersion !== 'skiff-package-service-host-fixture-v2' ||
    receipt.environment !== environment ||
    typeof receipt.baseAssembly?.assemblyIdentity !== 'string'
  ) {
    throw new Error('current-scope compiler fixture returned an invalid receipt');
  }
  if (options.writeActorProjection !== false) {
    await writeCurrentScopeActorRoutingProjection(root, receipt);
  }
  return { root, receipt };
}

async function writeCurrentScopeActorRoutingProjection(
  root: string,
  receipt: CurrentScopeCompilerGeneratedArtifactRoot['receipt']
): Promise<void> {
  const methods: Array<{
    actor: { serviceId: string; actorAbiIdentity: string };
    actorImplementationIdentity: string;
    methodIdentity: string;
    deployment: Record<string, string>;
    package: PackageArtifactRefLike;
  }> = [];
  for (const [packageName, deploymentName] of [
    ['consumer', 'consumer'],
    ['provider', 'provider'],
  ] as const) {
    const packageRef = receipt.packages[packageName]! as unknown as PackageArtifactRefLike;
    const deployment = receipt.deployments[deploymentName]!;
    const packageValue = await readRecord(
      root,
      { recordPath: packageRecordPath(packageRef) }
    );
    const files = Array.isArray(packageValue.files) ? packageValue.files : [];
    for (const rawFile of files as Array<Record<string, unknown>>) {
      const fileIrIdentity = rawFile.fileIrIdentity;
      if (typeof fileIrIdentity !== 'string') continue;
      const fileValue = await readRecord(root, {
        recordPath: fileIrRecordPath(packageRef, fileIrIdentity),
      });
      const actors = Array.isArray(fileValue.actorDeclarations)
        ? fileValue.actorDeclarations
        : [];
      for (const rawActor of actors as Array<Record<string, unknown>>) {
        const implementations = rawActor.methodImplementations;
        if (
          typeof rawActor.actorAbiIdentity !== 'string' ||
          typeof rawActor.actorImplementationIdentity !== 'string' ||
          implementations === null ||
          typeof implementations !== 'object' ||
          Array.isArray(implementations)
        ) {
          continue;
        }
        for (const methodIdentity of Object.keys(
          implementations as Record<string, unknown>
        )) {
          methods.push({
            actor: {
              serviceId: deployment.serviceId!,
              actorAbiIdentity: rawActor.actorAbiIdentity,
            },
            actorImplementationIdentity:
              rawActor.actorImplementationIdentity,
            methodIdentity,
            deployment,
            package: packageRef,
          });
        }
      }
    }
  }
  methods.sort((left, right) =>
    fullTypedKey(left).localeCompare(fullTypedKey(right))
  );
  await writeActorRoutingProjection(root, {
    schemaVersion: ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    methods,
  });
}

function fullTypedKey(method: {
  actor: { serviceId: string; actorAbiIdentity: string };
  actorImplementationIdentity: string;
  methodIdentity: string;
  deployment: Record<string, string>;
  package: PackageArtifactRefLike;
}): string {
  const actor = method.actor;
  const deployment = method.deployment;
  const packageRef = method.package;
  return [
    actor.serviceId,
    actor.actorAbiIdentity,
    method.actorImplementationIdentity,
    method.methodIdentity,
    deployment.serviceId,
    deployment.contractVersion,
    deployment.deploymentRevision,
    deployment.deploymentArtifactIdentity,
    packageRef.packageId,
    packageRef.packageVersion,
    packageRef.packageBuildId,
    packageRef.packageLocalAbiIdentity,
  ].join('\u0000');
}

async function writeActorRoutingProjection(
  root: string,
  projection: { schemaVersion: string; methods: unknown[] }
): Promise<void> {
  const bytes = canonicalJsonBytes(projection);
  const target = join(root, ACTOR_ROUTING_PROJECTION_RECORD_PATH);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
}

function packageRecordPath(packageRef: PackageArtifactRefLike): string {
  return [
    'records/package-artifacts',
    encodeCoordinate(packageRef.packageId),
    packageRef.packageVersion,
    identityHash(packageRef.packageBuildId),
    'package.json',
  ].join('/');
}

function fileIrRecordPath(
  packageRef: PackageArtifactRefLike,
  fileIrIdentity: string
): string {
  return [
    'records/package-artifacts',
    encodeCoordinate(packageRef.packageId),
    packageRef.packageVersion,
    identityHash(packageRef.packageBuildId),
    'file-ir',
    `${identityHash(fileIrIdentity)}.json`,
  ].join('/');
}

function encodeCoordinate(value: string): string {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function identityHash(value: string): string {
  return value.slice(value.lastIndexOf(':') + 1);
}

async function runPackageServiceFixture(args: string[]): Promise<void> {
  await execFileAsync(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path',
      testRunnerManifestPath,
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      ...args,
    ],
    { cwd: repoRoot }
  );
}

async function author(
  object: 'package',
  action: 'build' | 'publish',
  sourceRoot: string,
  artifactRoot: string
): Promise<Record<string, unknown>> {
  const { stdout } = await execFileAsync(
    'cargo',
    [
      'run',
      '--quiet',
      '--manifest-path',
      compilerManifestPath,
      '--bin',
      'skiff-compiler',
      '--',
      object,
      action,
      sourceRoot,
      '--artifact-root',
      artifactRoot,
      '--environment',
      'dev',
      '--platform-source-root',
      repoRoot,
      '--json',
    ],
    { cwd: repoRoot }
  );
  return JSON.parse(stdout) as Record<string, unknown>;
}

async function projectRuntimeAssembly(
  artifactRoot: string,
  environment: string,
  rootDeployments: ReadonlyArray<Record<string, string>>
): Promise<Record<string, unknown>> {
  const rootArguments = rootDeployments.flatMap((deployment) => [
    '--root-deployment',
    JSON.stringify(deployment),
  ]);
  const { stdout } = await execFileAsync(
    'cargo',
    [
      'run',
      '--quiet',
      '--manifest-path',
      compilerManifestPath,
      '--bin',
      'skiff-compiler',
      '--',
      'assembly',
      'build',
      '--artifact-root',
      artifactRoot,
      '--environment',
      environment,
      ...rootArguments,
      '--json',
    ],
    { cwd: repoRoot }
  );
  return JSON.parse(stdout) as Record<string, unknown>;
}

function objectReceipt(value: unknown, label: string): ObjectReceipt {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`compiler output is missing ${label}`);
  }
  const receipt = value as Record<string, unknown>;
  if (typeof receipt.recordPath !== 'string' || receipt.recordPath.length === 0) {
    throw new Error(`compiler ${label}.recordPath is missing`);
  }
  return receipt as ObjectReceipt;
}

async function readRecord(
  root: string,
  receipt: ObjectReceipt
): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(join(root, receipt.recordPath), 'utf8')) as Record<string, unknown>;
}
