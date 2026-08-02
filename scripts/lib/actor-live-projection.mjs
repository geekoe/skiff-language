// Test-side A1 actor-routing projection producer for `router-live:actor`.
//
// The production A1 producer (compiler publish path) is a sibling batch-10
// node; this harness needs the same canonical record today so the Rust
// Router consumes the real projection. It reads only the compiler-produced
// PackageArtifact / File IR records inside the artifact root (test-side A1
// producer role, never imported by production code) and writes the canonical
// `records/actor-routing/current.json` byte stream.

import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

export const ACTOR_ROUTING_PROJECTION_RECORD_PATH =
  'records/actor-routing/current.json';
export const ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION =
  'skiff-actor-routing-projection-v1';

export async function synthesizeActorRoutingProjection({
  artifactRoot,
  deploymentRecord,
}) {
  const assembly = await loadAssemblyRecordForDeployment(
    artifactRoot,
    deploymentRecord.deployment.serviceId,
  );
  const activation = firstActivationTemplate(assembly);
  const packageRef = codeSlotPackageForBuild(
    assembly,
    activation.implementationPackageBuildId,
  );
  const deployment = {
    serviceId: deploymentRecord.deployment.serviceId,
    contractVersion: deploymentRecord.deployment.contractVersion,
    deploymentRevision: deploymentRecord.deployment.deploymentRevision,
    deploymentArtifactIdentity: deploymentRecord.deployment.deploymentArtifactIdentity,
  };
  const packageValue = await readPackageArtifact(artifactRoot, packageRef);
  const methods = [];
  for (const rawFile of Array.isArray(packageValue.files) ? packageValue.files : []) {
    const fileIrIdentity = rawFile.fileIrIdentity;
    if (typeof fileIrIdentity !== 'string') {
      continue;
    }
    const fileValue = await readFileIrRecord(artifactRoot, packageRef, fileIrIdentity);
    for (const rawActor of Array.isArray(fileValue.actorDeclarations)
      ? fileValue.actorDeclarations
      : []) {
      const implementations = rawActor.methodImplementations;
      if (
        typeof rawActor.actorAbiIdentity !== 'string'
        || typeof rawActor.actorImplementationIdentity !== 'string'
        || implementations === null
        || typeof implementations !== 'object'
        || Array.isArray(implementations)
      ) {
        continue;
      }
      for (const methodIdentity of Object.keys(implementations)) {
        methods.push({
          actor: {
            serviceId: deployment.serviceId,
            actorAbiIdentity: rawActor.actorAbiIdentity,
          },
          actorImplementationIdentity: rawActor.actorImplementationIdentity,
          methodIdentity,
          deployment,
          package: packageRef,
        });
      }
    }
  }
  methods.sort((left, right) => fullTypedKey(left).localeCompare(fullTypedKey(right)));
  const projection = {
    schemaVersion: ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    methods,
  };
  const bytes = Buffer.from(canonicalJsonString(projection), 'utf8');
  const target = join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes, { encoding: 'utf8', flag: 'wx' });
  return projection;
}

export function canonicalJsonBytes(value) {
  return Buffer.from(canonicalJsonString(value), 'utf8');
}

function canonicalJsonString(value) {
  if (value === null) return 'null';
  if (value === true) return 'true';
  if (value === false) return 'false';
  if (typeof value === 'string') return `"${canonicalEscape(value)}"`;
  if (typeof value === 'number') return canonicalNumber(value);
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJsonString).join(',')}]`;
  }
  if (typeof value === 'object') {
    const record = value;
    const keys = Object.keys(record).sort();
    return `{${keys
      .map((key) => `"${canonicalEscape(key)}":${canonicalJsonString(record[key])}`)
      .join(',')}}`;
  }
  throw new Error('actor live projection contains a non-JSON value');
}

function canonicalNumber(value) {
  if (Object.is(value, -0)) return '0';
  if (!Number.isFinite(value)) {
    throw new Error('actor live projection contains a non-finite number');
  }
  if (!Number.isInteger(value) || !Number.isSafeInteger(value)) {
    throw new Error('actor live projection contains a non-canonical number');
  }
  return String(value);
}

function canonicalEscape(value) {
  let result = '';
  for (const character of value) {
    const code = character.codePointAt(0);
    if (character === '"') {
      result += '\\"';
    } else if (character === '\\') {
      result += '\\\\';
    } else if (code === 0x08) {
      result += '\\b';
    } else if (code === 0x0c) {
      result += '\\f';
    } else if (code === 0x0a) {
      result += '\\n';
    } else if (code === 0x0d) {
      result += '\\r';
    } else if (code === 0x09) {
      result += '\\t';
    } else if (code < 0x20) {
      result += `\\u${code.toString(16).padStart(4, '0')}`;
    } else {
      result += character;
    }
  }
  return result;
}

function fullTypedKey(method) {
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

async function loadAssemblyRecordForDeployment(artifactRoot, serviceId) {
  const directory = join(artifactRoot, 'records', 'runtime-assemblies');
  const files = await collectJsonFiles(directory);
  for (const file of files) {
    const assembly = JSON.parse(await readFile(file, 'utf8'));
    const templates = Array.isArray(assembly.activationTemplates)
      ? assembly.activationTemplates
      : [];
    if (
      templates.some(
        (template) => template?.deployment?.serviceId === serviceId,
      )
    ) {
      return assembly;
    }
  }
  throw new Error(
    `actor live artifact has no runtime assembly for deployment service ${serviceId} `
    + `(scanned ${files.length} assembly records)`,
  );
}

function firstActivationTemplate(assembly) {
  const templates = Array.isArray(assembly.activationTemplates)
    ? assembly.activationTemplates
    : [];
  const template = templates[0];
  if (
    template === null
    || typeof template !== 'object'
    || typeof template.implementationPackageBuildId !== 'string'
  ) {
    throw new Error('actor live runtime assembly has no activation template package build');
  }
  return template;
}

function codeSlotPackageForBuild(assembly, packageBuildId) {
  const codeSlots = assembly?.packageLinkPlan?.codeSlots;
  if (!Array.isArray(codeSlots)) {
    throw new Error('actor live runtime assembly has no packageLinkPlan.codeSlots');
  }
  const slot = codeSlots.find(
    (candidate) => candidate?.package?.packageBuildId === packageBuildId,
  );
  const packageRef = slot?.package;
  if (
    packageRef === null
    || typeof packageRef !== 'object'
    || typeof packageRef.packageId !== 'string'
    || typeof packageRef.packageVersion !== 'string'
    || typeof packageRef.packageBuildId !== 'string'
    || typeof packageRef.packageLocalAbiIdentity !== 'string'
  ) {
    throw new Error(
      `actor live assembly code slot for build ${packageBuildId} has no exact package ref`,
    );
  }
  return {
    packageId: packageRef.packageId,
    packageVersion: packageRef.packageVersion,
    packageBuildId: packageRef.packageBuildId,
    packageLocalAbiIdentity: packageRef.packageLocalAbiIdentity,
  };
}

async function readPackageArtifact(artifactRoot, packageRef) {
  const path = join(
    artifactRoot,
    'records',
    'package-artifacts',
    encodeCoordinate(packageRef.packageId),
    packageRef.packageVersion,
    identityHash(packageRef.packageBuildId),
    'package.json',
  );
  return JSON.parse(await readFile(path, 'utf8'));
}

async function readFileIrRecord(artifactRoot, packageRef, fileIrIdentity) {
  const path = join(
    artifactRoot,
    'records',
    'package-artifacts',
    encodeCoordinate(packageRef.packageId),
    packageRef.packageVersion,
    identityHash(packageRef.packageBuildId),
    'file-ir',
    `${identityHash(fileIrIdentity)}.json`,
  );
  return JSON.parse(await readFile(path, 'utf8'));
}

async function collectJsonFiles(directory, output = []) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return output;
    }
    throw error;
  }
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await collectJsonFiles(path, output);
    } else if (entry.isFile() && entry.name.endsWith('.json')) {
      output.push(path);
    }
  }
  return output;
}

function encodeCoordinate(value) {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function identityHash(value) {
  return value.slice(value.lastIndexOf(':') + 1);
}
