import type { ConfigShape } from '../config/index.js';
import type { RequestCancelReason } from './cancelReason.js';

export type { RequestCancelReason } from './cancelReason.js';

export type DispatchMode = 'unary' | 'serverStream';

export interface RuntimeCapabilitiesMetadata {
  dispatchModes?: readonly DispatchMode[];
  packageTestDispatch?: boolean;
  requestCancel?: boolean;
  runtimeProgram?: boolean;
}

export interface RuntimeClientSessionFrameMetadata {
  id: string;
}

export interface TraceContext {
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  sampled?: boolean;
}

export const TELEMETRY_PROTOCOL = 'skiff-telemetry-v1' as const;

export const TELEMETRY_TOPICS = ['log', 'trace', 'metric', 'health', 'debug'] as const;

export type TelemetryTopic = (typeof TELEMETRY_TOPICS)[number];

export type TelemetrySource = 'gateway' | 'router' | 'runtime' | 'provider' | 'test';

export type TelemetryLevel = 'debug' | 'info' | 'warn' | 'error';

export const SKIFF_BINARY_FRAME_MAGIC = Buffer.from([0x53, 0x4b, 0x42, 0x46]) as Buffer;
export const SKIFF_BINARY_FRAME_VERSION = 1;
export const SKIFF_BINARY_FRAME_HEADER_ENCODING_JSON = 1;
export const RUNTIME_FRAME_SCHEMA_VERSION = 'skiff-runtime-frame-v1' as const;

const BINARY_FRAME_FIXED_HEADER_BYTES = 14;
const UINT32_MAX = 0xffffffff;

export interface BinaryFrame<THeader extends Record<string, unknown> = Record<string, unknown>> {
  header: THeader;
  payloadBytes: Uint8Array;
}

export class BinaryFrameDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'BinaryFrameDecodeError';
  }
}

export type RuntimeFrameHeaderName =
  | 'runtime.register'
  | 'runtime.capabilities'
  | 'runtime.health'
  | 'runtime.registered'
  | 'router.control'
  | 'actor.put.request'
  | 'actor.put.response'
  | 'actor.put.error'
  | 'actor.find.request'
  | 'actor.find.response'
  | 'actor.find.error'
  | 'actor.remove.request'
  | 'actor.remove.response'
  | 'actor.remove.error'
  | 'spawn.submit.request'
  | 'spawn.submit.response'
  | 'spawn.submit.error'
  | 'spawn.claim.request'
  | 'spawn.claim.response'
  | 'spawn.claim.error'
  | 'spawn.renew.request'
  | 'spawn.renew.response'
  | 'spawn.renew.error'
  | 'spawn.complete.request'
  | 'spawn.complete.response'
  | 'spawn.complete.error'
  | 'spawn.fail.request'
  | 'spawn.fail.response'
  | 'spawn.fail.error'
  | 'request.start'
  | 'package-test.start'
  | 'request.cancel'
  | 'connection.send'
  | 'response.start'
  | 'response.chunk'
  | 'response.end'
  | 'response.error';

export interface RuntimeFrameHeaderBase<TType extends RuntimeFrameHeaderName> {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: TType;
}

export interface TelemetryControlConfig {
  endpoint: string;
  protocol: typeof TELEMETRY_PROTOCOL;
  topics: TelemetryTopic[];
  queueMaxEvents: number;
  batchMaxEvents: number;
  batchMaxBytes: number;
  flushIntervalMs: number;
  enabled: boolean;
}

export interface FileBackendControlConfig {
  local?: FileBackendLocalConfig;
  oss?: FileBackendOssConfig;
}

export interface FileBackendLocalConfig {
  root: string;
}

export interface FileBackendOssConfig {
  endpoint: string;
  bucket: string;
  region?: string;
  accessKeyId?: string;
  accessKeySecret?: string;
  accessKeyIdEnv?: string;
  accessKeySecretEnv?: string;
}

export interface TelemetryRegisterEnvelope {
  type: 'telemetry.register';
  protocol: typeof TELEMETRY_PROTOCOL;
  producerId: string;
  source: TelemetrySource;
  runtimeId?: string;
  topics: TelemetryTopic[];
}

export interface TelemetryEvent {
  topic: TelemetryTopic;
  ts: string;
  source: TelemetrySource;
  serviceId?: string;
  revisionId?: string;
  buildId?: string;
  activationIdentity?: string;
  runtimeId?: string;
  providerId?: string;
  providerRevision?: string;
  providerCapability?: string;
  providerTarget?: string;
  requestId?: string;
  clientRequestId?: string;
  traceId?: string;
  spanId?: string;
  parentSpanId?: string;
  target?: string;
  level?: TelemetryLevel;
  name?: string;
  message?: string;
  attrs?: Record<string, unknown>;
  error?: Record<string, unknown>;
  durationMs?: number;
  dropped?: Record<string, unknown>;
}

export interface TelemetryBatchEnvelope {
  type: 'telemetry.batch';
  producerId: string;
  seq: number;
  events: TelemetryEvent[];
}

export type TelemetryEnvelope = TelemetryRegisterEnvelope | TelemetryBatchEnvelope;

export interface RuntimeRegisterEnvelope {
  type: 'runtime.register';
  runtimeId: string;
  serviceId: string;
  // Stable published version this build serves; lets the registry tie a
  // registered build to the (serviceId, version) addressing coordinate.
  version?: string;
  revisionId: string;
  activationIdentity?: string;
  buildId: string;
  serviceProtocolIdentity: string;
  targets: string[];
  protocolVersion?: string;
  runtimeVersion?: string;
  codeRevisionId?: string;
  artifactIdentity?: string;
  gatewayEntryIdentities?: string[];
  capabilities?: RuntimeCapabilitiesMetadata;
}

export type RuntimeRegisterFrameHeader = RuntimeFrameHeaderBase<'runtime.register'> &
  Omit<RuntimeRegisterEnvelope, 'type'>;

export interface RuntimeCapabilitiesEnvelope {
  type: 'runtime.capabilities';
  runtimeId: string;
  capabilities: RuntimeCapabilitiesMetadata;
}

export type RuntimeCapabilitiesFrameHeader = RuntimeFrameHeaderBase<'runtime.capabilities'> &
  Omit<RuntimeCapabilitiesEnvelope, 'type'>;

export interface RuntimeHealthCounters {
  outboundRequestsPending: number;
  outboundStreamLeasesActive: number;
  streamRuntimeStreamsActive: number;
  flagBackedCancelWaitersActive: number;
  spawnedTasksActive: number;
}

export interface RuntimeHealthEnvelope {
  type: 'runtime.health';
  runtimeId: string;
  observedAt: string;
  counters: RuntimeHealthCounters;
}

export type RuntimeHealthFrameHeader = RuntimeFrameHeaderBase<'runtime.health'> &
  Omit<RuntimeHealthEnvelope, 'type'>;

export interface RuntimeRegisteredEnvelope {
  type: 'runtime.registered';
  runtimeId: string;
}

export type RuntimeRegisteredFrameHeader = RuntimeFrameHeaderBase<'runtime.registered'> &
  Omit<RuntimeRegisteredEnvelope, 'type'>;

export interface RouterControlEnvelope {
  type: 'router.control';
  artifactRoots: readonly string[];
  devReload?: boolean;
  mode?: 'dev' | 'release';
  generation?: string;
  fingerprint?: string;
  serviceBuilds?: readonly RuntimeControlServiceBuild[];
  serviceConfig?: RuntimeConfigActivationPayload[];
  telemetry?: TelemetryControlConfig;
  fileBackend?: FileBackendControlConfig;
}

export type RouterControlFrameHeader = RuntimeFrameHeaderBase<'router.control'> &
  Omit<RouterControlEnvelope, 'type' | 'serviceBuilds'>;

export interface RuntimeConfigActivationPayload {
  serviceId: string;
  buildId: string;
  activationIdentity: string;
  resolvedConfigIdentity: string;
  resolvedConfig: Record<string, unknown>;
  redactedResolvedConfig: Record<string, unknown>;
  redactionProjectionIdentity: string;
  configShape?: ConfigShape;
  serviceDb?: RuntimeServiceDbActivationPayload;
  packageConfigs?: RuntimePackageConfigActivationPayload[];
}

export interface RuntimeControlServiceBuild {
  serviceId: string;
  buildId: string;
  pointerBuildId?: string;
  sourcePath: string;
}

export interface RuntimeServiceDbConfigInput {
  mongoUrl: string;
}

export interface RuntimeServiceDbActivationPayload extends RuntimeServiceDbConfigInput {
  storageServiceId: string;
}

export interface RuntimePackageConfigActivationPayload {
  packageId: string;
  packageSlot?: number;
  alias: string;
  resolvedConfigIdentity: string;
  resolvedConfig: Record<string, unknown>;
  redactedResolvedConfig: Record<string, unknown>;
  redactionProjectionIdentity: string;
  configShape?: ConfigShape;
}

export interface HttpHeaderFrameMetadata {
  name: string;
  value: string;
}

export interface HttpQueryParamFrameMetadata {
  name: string;
  value: string;
}

export interface HttpRequestFrameMetadata {
  method: string;
  url: string;
  path: string;
  query: HttpQueryParamFrameMetadata[];
  headers: HttpHeaderFrameMetadata[];
}

export interface HttpResponseFrameMetadata {
  status: number;
  headers: HttpHeaderFrameMetadata[];
}

export interface HttpAdapterServiceFunctionMetadata {
  kind: 'serviceFunction';
  modulePath: string;
  symbol: string;
}

export interface HttpAdapterPackageFunctionMetadata {
  kind: 'packageFunction';
  packageId: string;
  symbolPath: string;
}

export type HttpAdapterCallableMetadata =
  | HttpAdapterServiceFunctionMetadata
  | HttpAdapterPackageFunctionMetadata;

export type HttpAdapterSourceKind = 'http.request' | 'http.body' | 'http.context';

export interface HttpAdapterArgMetadata {
  param: string;
  source: {
    kind: HttpAdapterSourceKind;
  };
}

export interface HttpAdapterFrameMetadata {
  kind: 'typedJson' | 'rawHttp';
  handler: HttpAdapterCallableMetadata;
  guard?: HttpAdapterCallableMetadata;
  pre?: HttpAdapterCallableMetadata;
  adapterArgs?: HttpAdapterArgMetadata[];
}

export interface WebSocketCookieFrameMetadata {
  name: string;
  value: string;
}

export type WebSocketAdapterSourceKind =
  | 'websocket.connectRequest'
  | 'websocket.receiveEvent'
  | 'websocket.connection'
  | 'websocket.connectionContext'
  | 'websocket.message'
  | 'websocket.messageBody'
  | 'websocket.connectionId'
  | 'websocket.businessIdentity';

export interface WebSocketAdapterArgMetadata {
  param: string;
  source: {
    kind: WebSocketAdapterSourceKind;
  };
}

export interface WebSocketConnectRequestFrameMetadata {
  connectionId: string;
  url: string;
  query: HttpQueryParamFrameMetadata[];
  headers: HttpHeaderFrameMetadata[];
  cookies: WebSocketCookieFrameMetadata[];
  version?: string;
}

export interface WebSocketContextCodecFrameMetadata {
  operationAbiId: string;
  contextTypeIdentity: string;
}

export type WebSocketContextExpectationFrameMetadata =
  | {
      kind: 'null';
    }
  | {
      kind: 'typed';
      connectOperationAbiId: string;
      contextTypeIdentity: string;
    };

export interface WebSocketPayloadSegmentFrameMetadata {
  kind: 'websocket.context' | 'websocket.message';
  offset: number;
  length: number;
}

export interface WebSocketReceiveFrameMetadata {
  connectionId: string;
  businessIdentity?: string;
  message: {
    tag: 'text' | 'binary';
    encoding: 'utf8' | 'binary';
  };
  payloadSegments: WebSocketPayloadSegmentFrameMetadata[];
  contextCodec?: WebSocketContextCodecFrameMetadata;
}

export interface WebSocketAdapterFrameMetadata {
  kind: 'connect' | 'receive';
  adapterArgs: WebSocketAdapterArgMetadata[];
  contextExpectation?: WebSocketContextExpectationFrameMetadata;
  connectRequest?: WebSocketConnectRequestFrameMetadata;
  receiveEvent?: WebSocketReceiveFrameMetadata;
}

export interface WebSocketConnectionPolicyFrameMetadata {
  maxConnections: number;
  overflow: 'close-oldest' | 'reject-new';
  closeCode?: number;
  closeReason?: string;
}

export interface WebSocketConnectResponseFrameMetadata {
  result: 'accept' | 'reject';
  businessIdentity?: string;
  connectionPolicy?: WebSocketConnectionPolicyFrameMetadata;
  contextCodec?: WebSocketContextCodecFrameMetadata;
  contextPayloadPresent: boolean;
  code?: number;
  reason?: string;
}

export interface RequestStartFrameHeader extends RuntimeFrameHeaderBase<'request.start'> {
  requestId: string;
  mode: DispatchMode;
  caller: {
    kind: 'gateway' | 'service';
    target: string;
  };
  target: string;
  operationAbiId: string;
  selector?: string;
  serviceId?: string;
  // Stable published addressing coordinate. When present (service-to-service
  // calls), the router resolves the current build for (serviceId, version) at
  // request time. buildId/serviceProtocolIdentity below are the caller's frozen
  // boundary-compatibility expectation, not the selector.
  version?: string;
  buildId: string;
  serviceProtocolIdentity: string;
  activationIdentity?: string;
  gatewayEntryIdentity?: string;
  businessIdentity?: string;
  websocketEntryId?: string;
  clientSession?: RuntimeClientSessionFrameMetadata;
  deadline?: {
    timeoutMs: number;
    expiresAt: string;
  };
  trace: TraceContext;
  httpRequest?: HttpRequestFrameMetadata;
  httpAdapter?: HttpAdapterFrameMetadata;
  websocketAdapter?: WebSocketAdapterFrameMetadata;
  testEffectsEnabled?: boolean;
  testEffectDoubles?: Record<string, Array<{
    expectRequest?: unknown;
    response: unknown;
  }>>;
}

export interface PackageTestStartFrameHeader extends RuntimeFrameHeaderBase<'package-test.start'> {
  requestId: string;
  caller: {
    kind: 'gateway';
    target: string;
  };
  packageId: string;
  packageVersion: string;
  testBuildIdentity: string;
  entrypointId: string;
  activationId: string;
  deadline?: {
    timeoutMs: number;
    expiresAt: string;
  };
  trace: TraceContext;
  testEffectsEnabled?: boolean;
  testEffectDoubles?: Record<string, Array<{
    expectRequest?: unknown;
    response: unknown;
  }>>;
}

export interface ResponseChunkFrameHeader extends RuntimeFrameHeaderBase<'response.chunk'> {
  requestId: string;
  seq: number;
}

export interface ResponseStartFrameHeader extends RuntimeFrameHeaderBase<'response.start'> {
  requestId: string;
  httpResponse: HttpResponseFrameMetadata;
}

export interface ResponseEndFrameHeader extends RuntimeFrameHeaderBase<'response.end'> {
  requestId: string;
  payloadPresent: boolean;
  httpResponse?: HttpResponseFrameMetadata;
  websocketConnect?: WebSocketConnectResponseFrameMetadata;
}

export interface ResponseErrorFrameHeader extends RuntimeFrameHeaderBase<'response.error'> {
  requestId: string;
  error: RuntimeErrorPayload;
}

export interface RequestCancelEnvelope {
  type: 'request.cancel';
  requestId: string;
  reason: RequestCancelReason;
}

export type RequestCancelFrameHeader = RuntimeFrameHeaderBase<'request.cancel'> &
  Omit<RequestCancelEnvelope, 'type'>;

export interface ConnectionSendEnvelope {
  type: 'connection.send';
  serviceId: string;
  websocketEntryId?: string;
  businessIdentity?: string;
  connectionId?: string;
  payloadKind: ConnectionSendPayloadKind;
  payloadBytes: Uint8Array;
}

export type ConnectionSendPayloadKind = 'text' | 'binary';

export interface ConnectionSendFrameHeader extends RuntimeFrameHeaderBase<'connection.send'> {
  serviceId: string;
  websocketEntryId?: string;
  businessIdentity?: string;
  connectionId?: string;
  payloadKind?: ConnectionSendPayloadKind;
}

export interface RuntimeErrorPayload {
  code: string;
  message: string;
  status?: number;
  details?: unknown;
}

export interface ActorKeyFrameMetadata {
  serviceId: string;
  actorTypeIdentity: string;
  actorIdTypeIdentity: string;
  actorIdEncodingVersion: string;
  canonicalActorIdKeyBytesBase64: string;
  actorIdHash?: string;
}

export interface ActorRefFrameMetadata extends ActorKeyFrameMetadata {
  actorIdHash: string;
  epoch?: number;
}

export interface RuntimeRpcFrameHeaderBase<TType extends RuntimeFrameHeaderName>
  extends RuntimeFrameHeaderBase<TType> {
  rpcId: string;
}

export interface RuntimeControlRequestFrameHeaderBase<TType extends RuntimeFrameHeaderName>
  extends RuntimeRpcFrameHeaderBase<TType> {
  runtimeId: string;
}

export interface ActorPutRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'actor.put.request'> {
  actorKey: ActorKeyFrameMetadata;
  objectSchemaIdentity: string;
  objectEncodingVersion: string;
}

export interface ActorPutResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'actor.put.response'> {
  actorRef: ActorRefFrameMetadata;
}

export interface ActorFindRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'actor.find.request'> {
  actorKey: ActorKeyFrameMetadata;
}

export interface ActorFindResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'actor.find.response'> {
  found: boolean;
  actorRef?: ActorRefFrameMetadata;
}

export interface ActorRemoveRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'actor.remove.request'> {
  actorKey: ActorKeyFrameMetadata;
}

export interface ActorRemoveResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'actor.remove.response'> {
  removed: boolean;
}

export type SpawnSubmitTargetKind = 'function';

export interface SpawnSubmitRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'spawn.submit.request'> {
  targetKind: SpawnSubmitTargetKind;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  target: string;
  spawnId?: string;
  buildId?: string;
  activationIdentity?: string;
  callerRequestId?: string;
  traceId?: string;
  callerTarget?: string;
  maxQueueWaitMs?: number;
}

export interface SpawnSubmitResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'spawn.submit.response'> {
  spawnId: string;
  itemId: string;
  status: 'submitted';
}

export interface SpawnClaimRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'spawn.claim.request'> {
  workerId: string;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  supportedTargets: string[];
  supportedSpawnCompatibilityKeys: string[];
  buildId?: string;
  maxExecutionMs?: number;
  maxConcurrency?: number;
}

export interface SpawnClaimDescriptorFrameMetadata {
  itemId: string;
  leaseId: string;
  spawnExecutionId: string;
  runtimeRequestId: string;
  spawnId: string;
  targetKind: SpawnSubmitTargetKind;
  target: string;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  buildId: string;
  payloadSchemaIdentity?: string;
  leaseExpiresAt?: string;
}

export interface SpawnClaimResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'spawn.claim.response'> {
  claimed: boolean;
  item?: SpawnClaimDescriptorFrameMetadata;
}

export interface SpawnRenewRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'spawn.renew.request'> {
  itemId: string;
  leaseId: string;
  workerId: string;
}

export interface SpawnRenewResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'spawn.renew.response'> {
  itemId: string;
  renewed: boolean;
  leaseExpiresAt?: string;
}

export interface SpawnCompleteRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'spawn.complete.request'> {
  itemId: string;
  leaseId: string;
  diagnostics?: Record<string, unknown>;
}

export interface SpawnCompleteResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'spawn.complete.response'> {
  itemId: string;
  status: 'completed';
}

export type SpawnFailReason = 'failed' | 'cancelled' | 'timed_out';

export interface SpawnFailRequestFrameHeader
  extends RuntimeControlRequestFrameHeaderBase<'spawn.fail.request'> {
  itemId: string;
  leaseId: string;
  reason: SpawnFailReason;
  diagnostics?: Record<string, unknown>;
}

export interface SpawnFailResponseFrameHeader
  extends RuntimeRpcFrameHeaderBase<'spawn.fail.response'> {
  itemId: string;
  status: SpawnFailReason;
}

export type ActorSpawnRuntimeRequestFrameHeader =
  | ActorPutRequestFrameHeader
  | ActorFindRequestFrameHeader
  | ActorRemoveRequestFrameHeader
  | SpawnSubmitRequestFrameHeader
  | SpawnClaimRequestFrameHeader
  | SpawnRenewRequestFrameHeader
  | SpawnCompleteRequestFrameHeader
  | SpawnFailRequestFrameHeader;

export type ActorSpawnRuntimeResponseFrameHeader =
  | ActorPutResponseFrameHeader
  | ActorFindResponseFrameHeader
  | ActorRemoveResponseFrameHeader
  | SpawnSubmitResponseFrameHeader
  | SpawnClaimResponseFrameHeader
  | SpawnRenewResponseFrameHeader
  | SpawnCompleteResponseFrameHeader
  | SpawnFailResponseFrameHeader;

export type ActorSpawnRuntimeErrorFrameHeaderName =
  | 'actor.put.error'
  | 'actor.find.error'
  | 'actor.remove.error'
  | 'spawn.submit.error'
  | 'spawn.claim.error'
  | 'spawn.renew.error'
  | 'spawn.complete.error'
  | 'spawn.fail.error';

export type ActorSpawnRuntimeErrorFrameHeader = {
  [Type in ActorSpawnRuntimeErrorFrameHeaderName]: RuntimeRpcFrameHeaderBase<Type> & {
    error: RuntimeErrorPayload;
  };
}[ActorSpawnRuntimeErrorFrameHeaderName];

export type RouterToRuntimeFrameHeader =
  | RouterControlFrameHeader
  | RuntimeRegisteredFrameHeader
  | ActorSpawnRuntimeResponseFrameHeader
  | ActorSpawnRuntimeErrorFrameHeader
  | RequestStartFrameHeader
  | PackageTestStartFrameHeader
  | RequestCancelFrameHeader
  | ResponseStartFrameHeader
  | ResponseChunkFrameHeader
  | ResponseEndFrameHeader
  | ResponseErrorFrameHeader;

export type RuntimeToRouterFrameHeader =
  | RuntimeRegisterFrameHeader
  | RuntimeCapabilitiesFrameHeader
  | RuntimeHealthFrameHeader
  | ActorSpawnRuntimeRequestFrameHeader
  | RequestStartFrameHeader
  | RequestCancelFrameHeader
  | ConnectionSendFrameHeader
  | ResponseStartFrameHeader
  | ResponseChunkFrameHeader
  | ResponseEndFrameHeader
  | ResponseErrorFrameHeader;

export type RuntimeFrameHeader = RouterToRuntimeFrameHeader | RuntimeToRouterFrameHeader;

export type RuntimeBinaryFrame<THeader extends RuntimeFrameHeader = RuntimeFrameHeader> =
  BinaryFrame<THeader & Record<string, unknown>>;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function encodeBinaryFrame<THeader extends Record<string, unknown>>(
  header: THeader,
  payloadBytes: Uint8Array = new Uint8Array()
): Buffer {
  const headerBytes = Buffer.from(JSON.stringify(header), 'utf8');
  if (headerBytes.byteLength === 0) {
    throw new Error('invalid skiff binary frame: header must not be empty');
  }
  if (headerBytes.byteLength > UINT32_MAX) {
    throw new Error('invalid skiff binary frame: header length exceeds u32');
  }
  if (payloadBytes.byteLength > UINT32_MAX) {
    throw new Error('invalid skiff binary frame: payload length exceeds u32');
  }

  const payloadBuffer = Buffer.from(
    payloadBytes.buffer,
    payloadBytes.byteOffset,
    payloadBytes.byteLength
  );
  const frame = Buffer.alloc(
    BINARY_FRAME_FIXED_HEADER_BYTES + headerBytes.byteLength + payloadBuffer.byteLength
  );
  SKIFF_BINARY_FRAME_MAGIC.copy(frame, 0);
  frame.writeUInt8(SKIFF_BINARY_FRAME_VERSION, 4);
  frame.writeUInt8(SKIFF_BINARY_FRAME_HEADER_ENCODING_JSON, 5);
  frame.writeUInt32BE(headerBytes.byteLength, 6);
  frame.writeUInt32BE(payloadBuffer.byteLength, 10);
  headerBytes.copy(frame, BINARY_FRAME_FIXED_HEADER_BYTES);
  payloadBuffer.copy(frame, BINARY_FRAME_FIXED_HEADER_BYTES + headerBytes.byteLength);
  return frame;
}

export function decodeBinaryFrame(data: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string): BinaryFrame {
  const frame = rawDataToBuffer(data);
  if (frame.byteLength < BINARY_FRAME_FIXED_HEADER_BYTES) {
    throw new BinaryFrameDecodeError('invalid skiff binary frame: frame is too short');
  }
  if (!frame.subarray(0, 4).equals(SKIFF_BINARY_FRAME_MAGIC)) {
    throw new BinaryFrameDecodeError('invalid skiff binary frame: expected skiff binary frame magic');
  }
  const version = frame.readUInt8(4);
  if (version !== SKIFF_BINARY_FRAME_VERSION) {
    throw new BinaryFrameDecodeError(
      `invalid skiff binary frame: unsupported frame version ${version}`
    );
  }
  const headerEncoding = frame.readUInt8(5);
  if (headerEncoding !== SKIFF_BINARY_FRAME_HEADER_ENCODING_JSON) {
    throw new BinaryFrameDecodeError(
      `invalid skiff binary frame: unsupported header encoding ${headerEncoding}`
    );
  }

  const headerLength = frame.readUInt32BE(6);
  const payloadLength = frame.readUInt32BE(10);
  if (headerLength === 0) {
    throw new BinaryFrameDecodeError('invalid skiff binary frame: header must not be empty');
  }

  const expectedLength = BINARY_FRAME_FIXED_HEADER_BYTES + headerLength + payloadLength;
  if (frame.byteLength !== expectedLength) {
    throw new BinaryFrameDecodeError(
      `invalid skiff binary frame: frame length ${frame.byteLength} does not match header length ${headerLength} plus payload length ${payloadLength}`
    );
  }

  const headerStart = BINARY_FRAME_FIXED_HEADER_BYTES;
  const payloadStart = headerStart + headerLength;
  const headerText = frame.subarray(headerStart, payloadStart).toString('utf8');
  let header: unknown;
  try {
    header = JSON.parse(headerText);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new BinaryFrameDecodeError(`invalid skiff binary frame: header is not valid JSON: ${message}`);
  }
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError('invalid skiff binary frame: header must be an object');
  }
  return {
    header,
    payloadBytes: frame.subarray(payloadStart)
  };
}

export function encodeRuntimeFrame<THeader extends RuntimeFrameHeader>(
  header: THeader,
  payloadBytes: Uint8Array = new Uint8Array()
): Buffer {
  return encodeBinaryFrame(header as THeader & Record<string, unknown>, payloadBytes);
}

export function decodeRuntimeFrame(data: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string): RuntimeBinaryFrame {
  const frame = decodeBinaryFrame(data);
  if (frame.header.schemaVersion !== RUNTIME_FRAME_SCHEMA_VERSION) {
    throw new BinaryFrameDecodeError(
      `invalid skiff runtime frame: schemaVersion must be ${RUNTIME_FRAME_SCHEMA_VERSION}`
    );
  }
  if (typeof frame.header.type !== 'string') {
    throw new BinaryFrameDecodeError('invalid skiff runtime frame: type must be a string');
  }
  return frame as RuntimeBinaryFrame;
}

function rawDataToBuffer(data: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string): Buffer {
  if (Array.isArray(data)) {
    return Buffer.concat(data);
  }
  if (typeof data === 'string') {
    return Buffer.from(data, 'utf8');
  }
  if (data instanceof ArrayBuffer) {
    return Buffer.from(new Uint8Array(data));
  }
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}
