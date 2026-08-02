// E-http live fixture authoring (`router-live:http`, plan §7/§8).
//
// Writes a real service source with HTTP gateway entries (rawHttp unary /
// stream, typedJson unary, service-managed CORS, service error, slow and
// burst endpoints), then produces the real compiler package/assembly/config
// snapshot artifacts and seeds the same semantic committed activation state
// into the canonical TS and Rust activation namespaces (the two Router
// implementations own separate Mongo namespaces; the rollback roundtrip
// switches the Router process while the Runtime, artifact and committed
// tuple stay fixed).

import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './package-service-authoring.mjs';
import {
  createDifferentialMongosh,
  seedActivationState,
} from './router-differential/mongo.mjs';

export const HTTP_LIVE_SERVICE_ID = 'test.skiff/router-rust-http-live';
export const HTTP_LIVE_VERSION = '0.1.0';
export const HTTP_LIVE_ENVIRONMENT = 'http-live';
export const HTTP_LIVE_GENERATION = 1;
export const HTTP_LIVE_REPLICA_ID = 'skiff-runtime-http-live-replica';

export const TS_HTTP_LIVE_DATABASE = 'skiff_router_ts_http_live';
export const TS_HTTP_LIVE_STATE_COLLECTION = 'router_assembly_activation_states';
export const RUST_HTTP_LIVE_DATABASE = 'skiff-router';
export const RUST_HTTP_LIVE_STATE_COLLECTION = 'activation_state';

const ACTIVATION_STATE_SCHEMA_VERSION = 'skiff-environment-activation-state-v2';
const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';

const BURST_CHUNK = 'B'.repeat(32 * 1024);
const BURST_EMITS = Array.from(
  { length: 40 },
  () => '  emit(std.http.streamChunk(chunk))',
).join('\n');

export function httpLiveMongoUrl(mongoPort) {
  return (
    `mongodb://127.0.0.1:${mongoPort}/${TS_HTTP_LIVE_DATABASE}`
    + '?directConnection=true&replicaSet=rs0&retryWrites=false'
  );
}

export async function writeHttpLiveServiceSource(sourceRoot) {
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    `id: ${HTTP_LIVE_SERVICE_ID}\nversion: ${HTTP_LIVE_VERSION}\n`,
  );
  await writeFile(
    join(sourceRoot, 'service.yml'),
    `id: ${HTTP_LIVE_SERVICE_ID}\n`,
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  await writeFile(
    join(sourceRoot, 'http.yml'),
    [
      'unary:',
      '  method: POST',
      '  path: /unary',
      '  kind: rawHttp',
      '  handler: main.unary',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'typed-unary:',
      '  method: POST',
      '  path: /typed-unary',
      '  kind: typedJson',
      '  handler: main.typedUnary',
      '  adapterArgs:',
      '    - param: body',
      '      source: { kind: http.body }',
      'echo:',
      '  method: POST',
      '  path: /echo',
      '  kind: rawHttp',
      '  handler: main.echo',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'stream:',
      '  method: POST',
      '  path: /stream',
      '  kind: rawHttp',
      '  handler: main.stream',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'echo-stream:',
      '  method: POST',
      '  path: /echo-stream',
      '  kind: rawHttp',
      '  handler: main.echoStream',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'slow:',
      '  method: GET',
      '  path: /slow',
      '  kind: rawHttp',
      '  handler: main.slow',
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
      'slow-stream:',
      '  method: GET',
      '  path: /slow-stream',
      '  kind: rawHttp',
      '  handler: main.slowStream',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'burst:',
      '  method: POST',
      '  path: /burst',
      '  kind: rawHttp',
      '  handler: main.burst',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'error:',
      '  method: GET',
      '  path: /error',
      '  kind: rawHttp',
      '  handler: main.fail',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
      'cors-options:',
      '  method: OPTIONS',
      '  path: /cors',
      '  kind: rawHttp',
      '  handler: main.cors',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: http.request }',
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
      'function unary(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  return textResponse(201, bytes.fromUtf8("unary:".concat(request.body.toUtf8String())))',
      '}',
      '',
      'function typedUnary(body: string) -> string {',
      '  return "typed:".concat(body)',
      '}',
      '',
      'function echo(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  return textResponse(200, request.body)',
      '}',
      '',
      'function stream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
      '  emit(std.http.streamStart(206, headers()))',
      '  emit(std.http.streamChunk(bytes.fromUtf8("alpha|")))',
      '  emit(std.http.streamChunk(request.body))',
      '  emit(std.http.streamChunk(bytes.fromUtf8("|omega")))',
      '  emit(std.http.streamEnd())',
      '  return null',
      '}',
      '',
      'function echoStream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
      '  emit(std.http.streamStart(200, headers()))',
      '  emit(std.http.streamChunk(request.body))',
      '  emit(std.http.streamEnd())',
      '  return null',
      '}',
      '',
      'function slow(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
      '  std.time.sleep(Duration.milliseconds(15000))',
      '  emit(std.http.streamStart(200, headers()))',
      '  emit(std.http.streamChunk(bytes.fromUtf8("late")))',
      '  emit(std.http.streamEnd())',
      '  return null',
      '}',
      '',
      'function slowUnary(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  std.time.sleep(Duration.milliseconds(15000))',
      '  return textResponse(200, bytes.fromUtf8("late"))',
      '}',
      '',
      'function slowStream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
      '  emit(std.http.streamStart(200, headers()))',
      '  emit(std.http.streamChunk(bytes.fromUtf8("first")))',
      '  std.time.sleep(Duration.milliseconds(15000))',
      '  emit(std.http.streamChunk(bytes.fromUtf8("late")))',
      '  emit(std.http.streamEnd())',
      '  return null',
      '}',
      '',
      'function burst(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {',
      `  const chunk = bytes.fromUtf8(${JSON.stringify(BURST_CHUNK)})`,
      '  emit(std.http.streamStart(200, headers()))',
      BURST_EMITS,
      '  emit(std.http.streamEnd())',
      '  return null',
      '}',
      '',
      'function fail(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  throw std.service.ProtocolError {',
      `    target: ${JSON.stringify(HTTP_LIVE_SERVICE_ID)},`,
      '    message: "intentional service failure",',
      '  }',
      '}',
      '',
      'function cors(request: std.http.HttpRequest) -> std.http.HttpResponse {',
      '  const corsHeaders = Array.empty<std.http.HttpHeader>()',
      '  corsHeaders.push(std.http.HttpHeader { name: "access-control-allow-origin", value: "https://service.example" })',
      '  corsHeaders.push(std.http.HttpHeader { name: "access-control-allow-methods", value: "OPTIONS, POST" })',
      '  return std.http.HttpResponse { status: 204, headers: corsHeaders, body: bytes.fromUtf8("") }',
      '}',
      '',
    ].join('\n'),
  );
}

export async function authorHttpLiveArtifact({
  skiffRoot,
  sourceRoot,
  artifactRoot,
  environment = HTTP_LIVE_ENVIRONMENT,
}) {
  await mkdir(artifactRoot, { recursive: true });
  // External HTTP services compile against the canonical skiff.run/std
  // PackageArtifact. The repository's established service-fixture bootstrap
  // seeds the compiler-owned std records/pointer into the artifact store
  // (the same canonical records the Router/Runtime load at runtime).
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
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    environment,
  });
  const deploymentRef = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (!isPlainObject(deploymentRef)) {
    throw new Error('http live package build returned no exact ServiceDeployment reference');
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
  const assemblyIdentity = assembly?.assemblyIdentity;
  if (typeof assemblyIdentity !== 'string' || typeof recordPath !== 'string') {
    throw new Error('http live assembly build returned no exact RuntimeAssembly receipt');
  }
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    environment,
    profile: 'dev',
    assemblyRecord: recordPath,
    sources: [{ root: sourceRoot, deployment: deploymentRef }],
  });
  const configSnapshotId =
    snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('http live config snapshot production returned no exact snapshot reference');
  }
  const projectionDirectory = join(artifactRoot, 'records/actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );
  return {
    assemblyIdentity,
    configSnapshotId,
    deploymentRef,
    recordPath,
  };
}

export async function seedHttpLiveCommittedState({
  mongoUrl,
  environment = HTTP_LIVE_ENVIRONMENT,
  generation = HTTP_LIVE_GENERATION,
  assemblyIdentity,
  configSnapshotId,
}) {
  const mongosh = createDifferentialMongosh();
  const state = {
    schemaVersion: ACTIVATION_STATE_SCHEMA_VERSION,
    environment,
    committed: {
      generation,
      assembly: { assemblyIdentity },
      configSnapshot: { snapshotId: configSnapshotId },
    },
    pending: null,
  };
  await seedActivationState({
    mongosh,
    mongoUrl,
    database: TS_HTTP_LIVE_DATABASE,
    collection: TS_HTTP_LIVE_STATE_COLLECTION,
    environment,
    state,
  });
  await seedActivationState({
    mongosh,
    mongoUrl,
    database: RUST_HTTP_LIVE_DATABASE,
    collection: RUST_HTTP_LIVE_STATE_COLLECTION,
    environment,
    state,
  });
  return { environment, generation, assemblyIdentity, configSnapshotId };
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
