import { execFile } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

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
    schemaVersion: 'skiff-package-service-host-fixture-v1';
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

  const assemblyRoot = join(root, '.authoring', 'assembly');
  await mkdir(assemblyRoot, { recursive: true });
  await writeFile(
    join(assemblyRoot, 'assembly.yml'),
    `${JSON.stringify({
      environment: 'router-fixture',
      rootDeployments: [serviceDeployment.deployment],
    }, null, 2)}\n`
  );
  const assemblyReceipt = await author('assembly', 'build', assemblyRoot, root);
  const runtimeAssembly = objectReceipt(
    assemblyReceipt.runtimeAssemblyReceipt,
    'runtimeAssemblyReceipt'
  ) as CompilerGeneratedArtifactRoot['runtimeAssembly'];

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
  root: string
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
    receipt.schemaVersion !== 'skiff-package-service-host-fixture-v1' ||
    receipt.environment !== environment ||
    typeof receipt.baseAssembly?.assemblyIdentity !== 'string'
  ) {
    throw new Error('current-scope compiler fixture returned an invalid receipt');
  }
  return { root, receipt };
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
  object: 'package' | 'assembly',
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
