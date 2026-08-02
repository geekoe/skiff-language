// E-activation live artifact authoring (`router-live:activation-full-chain`).
//
// Writes real service sources with distinct HTTP gateway entries, then
// produces real compiler package/assembly/config-snapshot artifacts for
// three service variants: 0.1.0 (committed generation 1), 0.1.1 (candidate
// generation 2) and 0.1.2 (candidate generation 3). The deployment artifact
// identity strips human version labels, so each variant also carries a
// distinct ingress path (`/unary`, `/unary-new`, `/unary-third`) to make the
// assembly records immutable-distinct. The activation coordinator loads the
// candidate records strictly from the same artifact root, so all three
// assembly and snapshot records must coexist.

import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './package-service-authoring.mjs';
import { captureCheckedCommand } from './command-execution.mjs';

export const ACTIVATION_LIVE_SERVICE_ID = 'test.skiff/router-rust-activation-live';
export const ACTIVATION_LIVE_VERSION = '0.1.0';
export const ACTIVATION_LIVE_CANDIDATE_VERSION = '0.1.1';
export const ACTIVATION_LIVE_THIRD_VERSION = '0.1.2';
export const ACTIVATION_LIVE_ENVIRONMENT = 'activation-live';
export const ACTIVATION_LIVE_GENERATION = 1;
export const ACTIVATION_LIVE_REPLICA_ID = 'skiff-runtime-activation-live-replica';

const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';

export async function writeActivationLiveServiceSource(sourceRoot, variant) {
  const variantSpec = ACTIVATION_LIVE_VARIANTS[variant];
  if (variantSpec === undefined) {
    throw new Error(`unknown activation-live variant ${variant}`);
  }
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    `id: ${ACTIVATION_LIVE_SERVICE_ID}\nversion: ${variantSpec.version}\n`,
  );
  await writeFile(
    join(sourceRoot, 'service.yml'),
    `id: ${ACTIVATION_LIVE_SERVICE_ID}\n`,
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  const handlers = variantSpec.handlers.join('\n');
  await writeFile(
    join(sourceRoot, 'http.yml'),
    [
      ...variantSpec.entries,
      '',
    ].join('\n'),
  );
  await writeFile(
    join(sourceRoot, 'main.skiff'),
    [
      'import std',
      '',
      'function headers() -> Array<std.http.HttpHeader> {',
      '  const result = Array.empty<std.http.HttpHeader>()',
      '  result.push(std.http.HttpHeader { name: "content-type", value: "text/plain" })',
      '  return result',
      '}',
      '',
      'function textResponse(status: integer, body: bytes) -> std.http.HttpResponse {',
      '  return std.http.HttpResponse {',
      '    status: status,',
      '    headers: headers(),',
      '    body: body,',
      '  }',
      '}',
      '',
      handlers,
      '',
    ].join('\n'),
  );
}

const ACTIVATION_LIVE_VARIANTS = {
  committed: {
    version: ACTIVATION_LIVE_VERSION,
    handlers: [
      'function unary(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  return textResponse(200, bytes.fromUtf8("pong"))',
      '}',
      '',
      'function slowUnary(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  std.time.sleep(Duration.milliseconds(4000))',
      '  return textResponse(200, bytes.fromUtf8("late"))',
      '}',
    ],
    entries: [
      'unary:',
      '  method: GET',
      '  path: /unary',
      '  kind: rawHttp',
      '  handler: main.unary',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'slow-unary:',
      '  method: GET',
      '  path: /slow-unary',
      '  kind: rawHttp',
      '  handler: main.slowUnary',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
    ],
  },
  candidate: {
    version: ACTIVATION_LIVE_CANDIDATE_VERSION,
    handlers: [
      'function unaryNew(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  return textResponse(200, bytes.fromUtf8("pong-new"))',
      '}',
    ],
    entries: [
      'unary-new:',
      '  method: GET',
      '  path: /unary-new',
      '  kind: rawHttp',
      '  handler: main.unaryNew',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
    ],
  },
  third: {
    version: ACTIVATION_LIVE_THIRD_VERSION,
    handlers: [
      'function unaryThird(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  return textResponse(200, bytes.fromUtf8("pong-third"))',
      '}',
    ],
    entries: [
      'unary-third:',
      '  method: GET',
      '  path: /unary-third',
      '  kind: rawHttp',
      '  handler: main.unaryThird',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
    ],
  },
};

async function authorVersion({ skiffRoot, sourceRoot, artifactRoot, environment, variant }) {
  await writeActivationLiveServiceSource(sourceRoot, variant);
  const version = ACTIVATION_LIVE_VARIANTS[variant].version;
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    environment,
  });
  const deploymentRef = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (
    deploymentRef === null
    || typeof deploymentRef !== 'object'
    || Array.isArray(deploymentRef)
  ) {
    throw new Error(`activation-live package build returned no ServiceDeployment ref for ${version}`);
  }
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    environment,
    rootDeployments: [deploymentRef],
  });
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  if (typeof assembly?.assemblyIdentity !== 'string' || typeof recordPath !== 'string') {
    throw new Error(`activation-live assembly build returned no exact receipt for ${version}`);
  }
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    environment,
    profile: 'dev',
    assemblyRecord: recordPath,
    sources: [{ root: sourceRoot, deployment: deploymentRef }],
  });
  const snapshotId = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof snapshotId !== 'string') {
    throw new Error(`activation-live config snapshot returned no exact receipt for ${version}`);
  }
  return {
    version,
    assemblyIdentity: assembly.assemblyIdentity,
    configSnapshotId: snapshotId,
    recordPath,
    deploymentRef,
  };
}

export async function authorActivationLiveArtifact({
  skiffRoot,
  sourceRoot,
  artifactRoot,
  environment = ACTIVATION_LIVE_ENVIRONMENT,
}) {
  await mkdir(artifactRoot, { recursive: true });
  // HTTP services import the canonical skiff.run/std package; seed the
  // compiler-owned std records/pointer into the artifact store exactly like
  // the established service-fixture bootstrap.
  await captureCheckedCommand(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path',
      join(skiffRoot, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root',
      artifactRoot,
      '--environment',
      environment,
      '--platform-source-root',
      skiffRoot,
    ],
    { cwd: skiffRoot },
  );
  const committed = await authorVersion({
    skiffRoot,
    sourceRoot,
    artifactRoot,
    environment,
    variant: 'committed',
  });
  const candidate = await authorVersion({
    skiffRoot,
    sourceRoot,
    artifactRoot,
    environment,
    variant: 'candidate',
  });
  const third = await authorVersion({
    skiffRoot,
    sourceRoot,
    artifactRoot,
    environment,
    variant: 'third',
  });
  await mkdir(join(artifactRoot, 'records/actor-routing'), { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );
  return { committed, candidate, third };
}
