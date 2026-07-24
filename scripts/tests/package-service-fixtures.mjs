import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function writeContractRoot(root, {
  serviceId = 'example.com/health',
  contractVersion = '1.0.0',
  returnType = builtin('string'),
} = {}) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'contract.yml'), `${JSON.stringify({
    schemaVersion: 'skiff-service-contract-definition-v1',
    serviceId,
    contractVersion,
    operations: {
      health: operation(returnType),
    },
    boundarySchema: {},
    diagnosticText: {
      service: 'Health',
      operations: { health: 'Health' },
      types: {},
    },
  }, null, 2)}\n`);
}

export async function writePackageRoot(root, {
  packageId = 'example.com/provider',
  services = [],
  api = 'health: main.health\n',
  source = 'function health() -> string { return "ok" }\n',
} = {}) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'package.yml'), `${JSON.stringify({
    id: packageId,
    version: '1.0.0',
    ...(services.length === 0 ? {} : { services }),
  }, null, 2)}\n`);
  await writeFile(join(root, 'api.yml'), api);
  await writeFile(join(root, 'main.skiff'), source);
}

export async function writeDeploymentRoot(root, {
  contract,
  implementation,
  operationId,
  deploymentRevision = 'revision-1',
  packagePublicPath = 'health',
  serviceSelectors = [],
} = {}) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'deployment.yml'), `${JSON.stringify({
    schemaVersion: 'skiff-service-deployment-input-v1',
    contract,
    deploymentRevision,
    implementation,
    operationBindings: [{ contractOperationId: operationId, packagePublicPath }],
    packageBindings: [],
    serviceSelectors,
    ingress: [],
    configLiterals: [],
    secretRefs: [],
    stateBindings: [],
    resourceBindings: [],
    runtimeCapabilityBindings: [],
    policy: {
      timeoutMs: 1000,
      resources: { cpuMillis: 100, memoryBytes: 1048576 },
      activation: { maxConcurrency: 1, idleTimeoutMs: null },
      principal: 'service:health',
    },
    diagnosticText: { displayName: 'Health', notes: {} },
  }, null, 2)}\n`);
}

export async function writeAssemblyRoot(root, environment, rootDeployments) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'assembly.yml'), `${JSON.stringify({
    environment,
    rootDeployments,
  }, null, 2)}\n`);
}

export async function readReceiptRecord(artifactRoot, receipt) {
  return JSON.parse(await readFile(join(artifactRoot, receipt.recordPath), 'utf8'));
}

export function contractCoordinate(alias = 'health') {
  return {
    alias,
    id: 'example.com/health',
    version: '1.0.0',
  };
}

function operation(returnType) {
  return {
    parameters: [],
    returnValue: {
      ty: returnType,
      valuePlan: valuePlan('provider'),
    },
    errors: { kind: 'none' },
    stream: { kind: 'unary' },
    cancellation: { kind: 'notCancellable' },
    callbacks: { kind: 'none' },
    maySuspend: false,
    effectGuarantee: {
      detachedParameters: true,
      detachedReturn: true,
      detachedError: true,
      noCallerReachableMutation: true,
      noCallerValueEscape: true,
      noSameHeapIdentity: true,
    },
  };
}

function valuePlan(owner) {
  return {
    kind: 'linkable',
    carrier: 'detachedValueGraph',
    encoding: 'canonicalValue',
    owner,
    lifetime: 'call',
  };
}

function builtin(name) {
  return { kind: 'builtin', name, arguments: [] };
}
