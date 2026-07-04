import { randomUUID } from 'node:crypto';
import type { IncomingMessage, ServerResponse } from 'node:http';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type DispatchMode,
  type PackageTestStartFrameHeader,
  type RouterControlEnvelope,
  type RuntimeServiceDbConfigInput,
  type RequestStartFrameHeader
} from '../protocol/envelope.js';
import { validateRouterToRuntimeFrameHeader } from '../protocol/runtimeProtocol.js';
import type { OperationManifest } from '../manifest/types.js';
import { isPublicationId, publicationStorageSegment } from '../publicationId.js';
import {
  RouterActiveSnapshotStore,
  summarizeRouterActiveSnapshot,
  type RouterActiveSnapshot
} from './activeSnapshot.js';
import { GatewayError, toGatewayError } from './errors.js';
import type {
  RuntimePruneKeep,
  RuntimeRegistry
} from './runtimeRegistry.js';
import type { RuntimeDispatcher } from './runtimeDispatcher.js';

export interface RuntimeControlBroadcaster {
  broadcastControl(control: Omit<RouterControlEnvelope, 'type'>): void;
}

export interface ControlPlaneOptions {
  controlBroadcaster: RuntimeControlBroadcaster;
  dispatcher: RuntimeDispatcher;
  registry: RuntimeRegistry;
  reloadArtifacts?: (overrides?: ReloadArtifactsOverrides) => Promise<RouterActiveSnapshot>;
  requestTimeoutMs?: number;
  snapshotStore: RouterActiveSnapshotStore;
}

export interface ReloadArtifactsOverrides {
  artifactRoots?: string[];
  configProfile?: string;
  serviceDb?: RuntimeServiceDbConfigInput;
}

type TestEffectDoubles = Record<string, Array<{
  expectRequest?: unknown;
  response: unknown;
}>>;

interface ServiceTestDispatchRequest {
  kind?: 'service';
  activationIdentity?: string;
  buildId: string;
  mode?: DispatchMode;
  operation?: string;
  operationAbiId?: string;
  payloadBase64?: string;
  serviceId?: string;
  serviceProtocolIdentity: string;
  target: string;
  testEffectDoubles?: TestEffectDoubles;
  testEffectsEnabled?: boolean;
  timeoutMs?: number;
  websocketEntryId?: string;
}

interface PackageTestDispatchRequest {
  kind: 'packageTest';
  activationId: string;
  entrypointId: string;
  packageId: string;
  packageVersion: string;
  payloadBase64?: string;
  testBuildIdentity: string;
  testEffectDoubles?: TestEffectDoubles;
  testEffectsEnabled?: boolean;
  timeoutMs?: number;
}

type TestDispatchRequest = ServiceTestDispatchRequest | PackageTestDispatchRequest;

interface ResolvedTestDispatch {
  activationIdentity: string | undefined;
  mode: DispatchMode;
  operationAbiId: string;
  payloadBytes: Buffer;
  serviceId: string;
  timeoutMs: number;
}

interface PruneRuntimesRequest {
  keep?: RuntimePruneKeep[];
  serviceIds?: string[];
}

export class RouterControlPlane {
  private readonly requestTimeoutMs: number;
  private reloadInFlight: Promise<RouterActiveSnapshot> | undefined;

  constructor(private readonly options: ControlPlaneOptions) {
    this.requestTimeoutMs = options.requestTimeoutMs ?? 2000;
  }

  async handleRequest(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean> {
    const url = requestUrl(request);
    if (url.pathname === '/__router/health') {
      this.writeJson(response, 200, {
        ok: true,
        ...summarizeRouterActiveSnapshot(this.options.snapshotStore.get()),
        runtimes: this.options.registry.snapshot()
      });
      return true;
    }

    if (url.pathname === '/__skiff/reload-artifacts') {
      await this.handleReloadArtifacts(request, response);
      return true;
    }

    if (url.pathname === '/__router/prune-runtimes') {
      await this.handlePruneRuntimes(request, response);
      return true;
    }

    if (url.pathname === '/__skiff/test-dispatch') {
      await this.handleTestDispatch(request, response);
      return true;
    }

    return false;
  }

  async handleRequestWithErrors(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean> {
    try {
      return await this.handleRequest(request, response);
    } catch (error: unknown) {
      const gatewayError = toGatewayError(error);
      this.writeJson(response, gatewayError.statusCode, {
        error: gatewayError.toPayload()
      });
      return true;
    }
  }

  private async handleReloadArtifacts(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    if (request.method !== 'POST') {
      response.setHeader('allow', 'POST');
      this.writeJson(response, 405, {
        error: {
          code: 'MethodNotAllowed',
          message: 'reload artifacts requires POST'
        }
      });
      return;
    }
    const overrides = await readOptionalReloadArtifactsOverrides(request);
    if (!this.options.reloadArtifacts) {
      this.writeJson(response, 409, {
        error: {
          code: 'ArtifactReloadUnavailable',
          message: 'router was not started with artifact roots'
        }
      });
      return;
    }

    const snapshot = await this.reloadArtifactsOnce(overrides);
    this.writeJson(response, 200, {
      ok: true,
      ...summarizeRouterActiveSnapshot(snapshot),
      runtimes: this.options.registry.snapshot()
    });
  }

  private async handlePruneRuntimes(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    if (request.method !== 'POST') {
      response.setHeader('allow', 'POST');
      this.writeJson(response, 405, {
        error: {
          code: 'MethodNotAllowed',
          message: 'prune runtimes requires POST'
        }
      });
      return;
    }

    const snapshot = this.options.snapshotStore.get();
    const body = parsePruneRuntimesRequest(await readOptionalPruneRuntimesJson(request));
    const keep = body.keep ?? keepRuntimesFromSnapshot(snapshot);
    if (keep === undefined) {
      throw new GatewayError(
        409,
        'PruneRuntimesUnavailable',
        'router snapshot does not include service builds; provide keep entries'
      );
    }
    const result = this.options.registry.pruneRuntimes({
      keep,
      ...(body.serviceIds !== undefined ? { serviceIds: body.serviceIds } : {})
    });
    this.writeJson(response, 200, {
      ok: true,
      keep,
      ...(body.serviceIds !== undefined ? { serviceIds: body.serviceIds } : {}),
      deletedCount: result.deleted.length,
      keptCount: result.kept.length,
      deleted: result.deleted,
      kept: result.kept,
      runtimes: this.options.registry.snapshot()
    });
  }

  private async handleTestDispatch(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    if (request.method !== 'POST') {
      response.setHeader('allow', 'POST');
      throw new GatewayError(405, 'MethodNotAllowed', 'test dispatch requires POST');
    }

    const snapshot = this.options.snapshotStore.get();
    const body = parseTestDispatchRequest(await readJsonRequest(request));
    if (body.kind === 'packageTest') {
      await this.handlePackageTestDispatch(body, response);
      return;
    }

    const resolved = this.resolveTestDispatch(snapshot, body);
    const requestId = randomUUID();
    const header: RequestStartFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId,
      mode: resolved.mode,
      caller: {
        kind: 'gateway',
        target: '__skiff.test-dispatch'
      },
      target: body.target,
      operationAbiId: resolved.operationAbiId,
      selector: `operation:${resolved.operationAbiId}`,
      serviceId: resolved.serviceId,
      buildId: body.buildId,
      serviceProtocolIdentity: body.serviceProtocolIdentity,
      ...(resolved.activationIdentity !== undefined
        ? { activationIdentity: resolved.activationIdentity }
        : {}),
      deadline: {
        timeoutMs: resolved.timeoutMs,
        expiresAt: new Date(Date.now() + resolved.timeoutMs).toISOString()
      },
      trace: {
        traceId: randomUUID(),
        spanId: randomUUID()
      },
      ...(body.testEffectsEnabled !== undefined
        ? { testEffectsEnabled: body.testEffectsEnabled }
        : {}),
      ...(body.testEffectDoubles !== undefined
        ? { testEffectDoubles: body.testEffectDoubles }
        : {}),
      ...(body.websocketEntryId !== undefined
        ? { websocketEntryId: body.websocketEntryId }
        : {})
    };
    const validation = validateRouterToRuntimeFrameHeader(header);
    if (!validation.ok) {
      throw new GatewayError(400, 'InvalidTestDispatchRequest', validation.error);
    }

    const runtimeResponse = await this.options.dispatcher.dispatchBinaryFrame(
      {
        header,
        payloadBytes: resolved.payloadBytes
      },
      resolved.timeoutMs
    );
    this.writeJson(response, 200, {
      ok: true,
      header: runtimeResponse.header,
      payloadBase64: Buffer.from(runtimeResponse.payloadBytes).toString('base64')
    });
  }

  private async handlePackageTestDispatch(
    body: PackageTestDispatchRequest,
    response: ServerResponse
  ): Promise<void> {
    const requestId = randomUUID();
    const timeoutMs = body.timeoutMs ?? this.requestTimeoutMs;
    const header: PackageTestStartFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'package-test.start',
      requestId,
      caller: {
        kind: 'gateway',
        target: '__skiff.test-dispatch'
      },
      packageId: body.packageId,
      packageVersion: body.packageVersion,
      testBuildIdentity: body.testBuildIdentity,
      entrypointId: body.entrypointId,
      activationId: body.activationId,
      deadline: {
        timeoutMs,
        expiresAt: new Date(Date.now() + timeoutMs).toISOString()
      },
      trace: {
        traceId: randomUUID(),
        spanId: randomUUID()
      },
      ...(body.testEffectsEnabled !== undefined
        ? { testEffectsEnabled: body.testEffectsEnabled }
        : {}),
      ...(body.testEffectDoubles !== undefined
        ? { testEffectDoubles: body.testEffectDoubles }
        : {})
    };
    const validation = validateRouterToRuntimeFrameHeader(header);
    if (!validation.ok) {
      throw new GatewayError(400, 'InvalidTestDispatchRequest', validation.error);
    }

    const runtimeResponse = await this.options.dispatcher.dispatchBinaryFrame(
      {
        header,
        payloadBytes: decodePayloadBase64(body.payloadBase64)
      },
      timeoutMs
    );
    this.writeJson(response, 200, {
      ok: true,
      header: runtimeResponse.header,
      payloadBase64: Buffer.from(runtimeResponse.payloadBytes).toString('base64')
    });
  }

  private resolveTestDispatch(
    snapshot: RouterActiveSnapshot,
    body: ServiceTestDispatchRequest
  ): ResolvedTestDispatch {
    const explicitRuntimeAddress =
      body.serviceId !== undefined && body.mode !== undefined;
    const operation = explicitRuntimeAddress
      ? undefined
      : snapshot.manifest.operationsByTarget.get(body.target);
    if (
      operation !== undefined &&
      body.operation !== undefined &&
      body.operation !== operation.operation
    ) {
      throw new GatewayError(
        400,
        'TestDispatchOperationMismatch',
        `target ${body.target} resolves to operation ${operation.operation}, not ${body.operation}`
      );
    }
    const operationProtocolIdentity = operation?.serviceProtocolIdentity;
    if (
      operationProtocolIdentity !== undefined &&
      operationProtocolIdentity !== body.serviceProtocolIdentity
    ) {
      throw new GatewayError(
        400,
        'TestDispatchProtocolMismatch',
        `target ${body.target} belongs to ${operationProtocolIdentity}, not ${body.serviceProtocolIdentity}`
      );
    }

    const mode = operation?.mode ?? body.mode;
    if (mode === undefined) {
      throw new GatewayError(
        400,
        'TestDispatchModeRequired',
        'mode is required when target is not present in the active manifest'
      );
    }
    const timeoutMs = explicitRuntimeAddress
      ? body.timeoutMs ?? this.requestTimeoutMs
      : operation?.timeoutMs ??
        timeoutFromManifest(snapshot, operation, body.operation) ??
        body.timeoutMs ??
        snapshot.manifest.timeout?.defaultMs ??
        this.requestTimeoutMs;
    const serviceId =
      body.serviceId ?? serviceIdFromTarget(body.target) ?? snapshot.manifest.service.id;
    const operationAbiId = operation?.operationAbiId ?? body.operationAbiId;
    if (operationAbiId === undefined) {
      throw new GatewayError(
        400,
        'TestDispatchOperationAbiIdRequired',
        'operationAbiId is required when target is not present in the active manifest'
      );
    }
    const activationIdentity = body.activationIdentity ?? (explicitRuntimeAddress
      ? undefined
      : snapshot.activationByServiceOperation.get({
          serviceId,
          buildId: body.buildId,
          target: body.target
        }));
    return {
      activationIdentity,
      mode,
      operationAbiId,
      payloadBytes: decodePayloadBase64(body.payloadBase64),
      serviceId,
      timeoutMs
    };
  }

  private reloadArtifactsOnce(
    overrides?: ReloadArtifactsOverrides
  ): Promise<RouterActiveSnapshot> {
    if (!this.options.reloadArtifacts) {
      throw new GatewayError(409, 'ArtifactReloadUnavailable', 'router was not started with artifact roots');
    }
    if (overrides !== undefined) {
      return this.reloadArtifacts(overrides);
    }
    if (this.reloadInFlight) {
      return this.reloadInFlight;
    }
    const reload = this.reloadArtifacts()
      .finally(() => {
        if (this.reloadInFlight === reload) {
          this.reloadInFlight = undefined;
        }
      });
    this.reloadInFlight = reload;
    return reload;
  }

  private async reloadArtifacts(
    overrides?: ReloadArtifactsOverrides
  ): Promise<RouterActiveSnapshot> {
    if (!this.options.reloadArtifacts) {
      throw new GatewayError(409, 'ArtifactReloadUnavailable', 'router was not started with artifact roots');
    }
    const snapshot = await this.options.reloadArtifacts(overrides);
    this.options.snapshotStore.replace(snapshot);
    // Keep cross-service version addressing pointed at the freshly loaded
    // service-version pointer records so a reload that publishes a new build for
    // a version immediately routes to it.
    this.options.registry.setServiceVersionIndex(snapshot.versionByService);
    this.options.registry.setActivationLookup(snapshot.activationByServiceOperation);
    if (snapshot.control) {
      this.options.controlBroadcaster.broadcastControl(snapshot.control);
    }
    return snapshot;
  }

  private writeJson(response: ServerResponse, statusCode: number, value: unknown): void {
    if (response.headersSent) {
      response.end();
      return;
    }
    response.statusCode = statusCode;
    response.setHeader('content-type', 'application/json; charset=utf-8');
    response.end(JSON.stringify(value));
  }
}

async function readOptionalReloadArtifactsOverrides(
  request: IncomingMessage
): Promise<ReloadArtifactsOverrides | undefined> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > 1024 * 1024) {
      throw new GatewayError(413, 'RequestTooLarge', 'reload artifacts request body is too large');
    }
    chunks.push(buffer);
  }
  const text = Buffer.concat(chunks).toString('utf8').trim();
  if (text.length === 0) {
    return undefined;
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      `request body is not valid JSON: ${message}`
    );
  }
  if (!isRecord(value)) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      'request body must be a JSON object'
    );
  }
  const artifactRoots = optionalReloadBodyStringArray(value, 'artifactRoots');
  if (Object.prototype.hasOwnProperty.call(value, 'artifactRoot')) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      'artifactRoot is no longer supported; use artifactRoots'
    );
  }
  const configProfile = optionalReloadBodyString(value, 'configProfile');
  const serviceDb = optionalReloadServiceDb(value.serviceDb);
  return artifactRoots === undefined && configProfile === undefined && serviceDb === undefined
    ? undefined
    : {
        ...(artifactRoots !== undefined ? { artifactRoots } : {}),
        ...(configProfile !== undefined ? { configProfile } : {}),
        ...(serviceDb !== undefined ? { serviceDb } : {})
      };
}

function optionalReloadServiceDb(value: unknown): RuntimeServiceDbConfigInput | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      'serviceDb must be an object'
    );
  }
  const mongoUrl = value.mongoUrl;
  if (typeof mongoUrl !== 'string' || mongoUrl.length === 0) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      'serviceDb.mongoUrl must be a non-empty string'
    );
  }
  return { mongoUrl };
}

function optionalReloadBodyString(
  value: Record<string, unknown>,
  field: string
): string | undefined {
  const raw = value[field];
  if (raw === undefined) {
    return undefined;
  }
  if (typeof raw !== 'string' || raw.trim().length === 0) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      `${field} must be a non-empty string`
    );
  }
  return raw.trim();
}

function optionalReloadBodyStringArray(
  value: Record<string, unknown>,
  field: string
): string[] | undefined {
  const raw = value[field];
  if (raw === undefined) {
    return undefined;
  }
  if (!Array.isArray(raw) || raw.length === 0) {
    throw new GatewayError(
      400,
      'InvalidArtifactReloadRequest',
      `${field} must be a non-empty string array`
    );
  }
  return raw.map((item, index) => {
    if (typeof item !== 'string' || item.trim().length === 0) {
      throw new GatewayError(
        400,
        'InvalidArtifactReloadRequest',
        `${field}[${index}] must be a non-empty string`
      );
    }
    return item.trim();
  });
}

async function readOptionalPruneRuntimesJson(
  request: IncomingMessage
): Promise<unknown | undefined> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > 1024 * 1024) {
      throw new GatewayError(
        413,
        'RequestTooLarge',
        'prune runtimes request body is too large'
      );
    }
    chunks.push(buffer);
  }
  const text = Buffer.concat(chunks).toString('utf8').trim();
  if (text.length === 0) {
    return undefined;
  }
  try {
    return JSON.parse(text);
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    throw new GatewayError(
      400,
      'InvalidPruneRuntimesRequest',
      `request body is not valid JSON: ${message}`
    );
  }
}

function parsePruneRuntimesRequest(value: unknown | undefined): PruneRuntimesRequest {
  if (value === undefined) {
    return {};
  }
  if (!isRecord(value)) {
    throw new GatewayError(
      400,
      'InvalidPruneRuntimesRequest',
      'request body must be a JSON object'
    );
  }
  for (const field of Object.keys(value)) {
    if (field !== 'keep' && field !== 'serviceIds') {
      throw new GatewayError(
        400,
        'InvalidPruneRuntimesRequest',
        `${field} is not supported for prune runtimes`
      );
    }
  }
  const keep = parsePruneKeep(value.keep);
  const serviceIds = parsePruneServiceIds(value.serviceIds);
  return {
    ...(keep !== undefined ? { keep } : {}),
    ...(serviceIds !== undefined ? { serviceIds } : {})
  };
}

function parsePruneKeep(value: unknown): RuntimePruneKeep[] | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!Array.isArray(value)) {
    throw new GatewayError(
      400,
      'InvalidPruneRuntimesRequest',
      'keep must be an array'
    );
  }
  const keep: RuntimePruneKeep[] = [];
  const seen = new Set<string>();
  for (const [index, item] of value.entries()) {
    if (!isRecord(item)) {
      throw new GatewayError(
        400,
        'InvalidPruneRuntimesRequest',
        `keep[${index}] must be an object`
      );
    }
    const serviceId = pruneBodyString(item, `keep[${index}].serviceId`, 'serviceId');
    if (!isPublicationId(serviceId)) {
      throw new GatewayError(
        400,
        'InvalidPruneRuntimesRequest',
        `keep[${index}].serviceId must be a publication id`
      );
    }
    const buildId = pruneBodyString(item, `keep[${index}].buildId`, 'buildId');
    const key = `${serviceId}\u0000${buildId}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    keep.push({ serviceId, buildId });
  }
  return keep;
}

function parsePruneServiceIds(value: unknown): string[] | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!Array.isArray(value) || value.length === 0) {
    throw new GatewayError(
      400,
      'InvalidPruneRuntimesRequest',
      'serviceIds must be a non-empty array'
    );
  }
  const serviceIds: string[] = [];
  const seen = new Set<string>();
  for (const [index, item] of value.entries()) {
    if (typeof item !== 'string' || item.length === 0) {
      throw new GatewayError(
        400,
        'InvalidPruneRuntimesRequest',
        `serviceIds[${index}] must be a non-empty string`
      );
    }
    if (!isPublicationId(item)) {
      throw new GatewayError(
        400,
        'InvalidPruneRuntimesRequest',
        `serviceIds[${index}] must be a publication id`
      );
    }
    if (seen.has(item)) {
      continue;
    }
    seen.add(item);
    serviceIds.push(item);
  }
  return serviceIds;
}

function pruneBodyString(
  value: Record<string, unknown>,
  label: string,
  field: string
): string {
  const parsed = value[field];
  if (typeof parsed !== 'string' || parsed.length === 0) {
    throw new GatewayError(
      400,
      'InvalidPruneRuntimesRequest',
      `${label} must be a non-empty string`
    );
  }
  return parsed;
}

function keepRuntimesFromSnapshot(
  snapshot: RouterActiveSnapshot
): RuntimePruneKeep[] | undefined {
  const serviceBuilds = snapshot.control?.serviceBuilds;
  if (serviceBuilds === undefined) {
    return undefined;
  }
  const keep: RuntimePruneKeep[] = [];
  const seen = new Set<string>();
  for (const build of serviceBuilds) {
    const key = `${build.serviceId}\u0000${build.buildId}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    keep.push({
      serviceId: build.serviceId,
      buildId: build.buildId
    });
  }
  return keep;
}

async function readJsonRequest(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > 1024 * 1024) {
      throw new GatewayError(413, 'RequestTooLarge', 'test dispatch request body is too large');
    }
    chunks.push(buffer);
  }
  const text = Buffer.concat(chunks).toString('utf8').trim();
  if (text.length === 0) {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', 'request body must be JSON');
  }
  try {
    return JSON.parse(text);
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    throw new GatewayError(
      400,
      'InvalidTestDispatchRequest',
      `request body is not valid JSON: ${message}`
    );
  }
}

const SERVICE_TEST_DISPATCH_FIELDS = [
  'serviceId',
  'activationIdentity',
  'buildId',
  'serviceProtocolIdentity',
  'operation',
  'operationAbiId',
  'target',
  'mode',
  'websocketEntryId'
] as const;

const PACKAGE_TEST_DISPATCH_FIELDS = [
  'packageId',
  'packageVersion',
  'testBuildIdentity',
  'entrypointId',
  'activationId'
] as const;

const SHARED_TEST_DISPATCH_FIELDS = [
  'kind',
  'payloadBase64',
  'testEffectsEnabled',
  'testEffectDoubles',
  'timeoutMs'
] as const;

const PACKAGE_TEST_ALLOWED_FIELDS = new Set<string>([
  ...PACKAGE_TEST_DISPATCH_FIELDS,
  ...SHARED_TEST_DISPATCH_FIELDS
]);

function parseTestDispatchRequest(value: unknown): TestDispatchRequest {
  if (!isRecord(value)) {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', 'request body must be a JSON object');
  }
  const kind = parseTestDispatchKind(value);
  if (kind === 'packageTest') {
    return parsePackageTestDispatchRequest(value);
  }
  return parseServiceTestDispatchRequest(value, kind);
}

function parseTestDispatchKind(
  value: Record<string, unknown>
): ServiceTestDispatchRequest['kind'] | PackageTestDispatchRequest['kind'] | undefined {
  const kind = value.kind;
  if (kind === undefined) {
    return undefined;
  }
  if (kind === 'service' || kind === 'packageTest') {
    return kind;
  }
  throw new GatewayError(
    400,
    'InvalidTestDispatchKind',
    'kind must be "service" or "packageTest"'
  );
}

function parseServiceTestDispatchRequest(
  value: Record<string, unknown>,
  kind: ServiceTestDispatchRequest['kind'] | undefined
): ServiceTestDispatchRequest {
  rejectPresentFields(
    value,
    PACKAGE_TEST_DISPATCH_FIELDS,
    'service test dispatch'
  );
  const buildId = requireBodyString(value, 'buildId');
  const activationIdentity = optionalBodyString(value, 'activationIdentity');
  const serviceProtocolIdentity = requireBodyString(value, 'serviceProtocolIdentity');
  const target = requireBodyString(value, 'target');
  const mode = optionalDispatchMode(value, 'mode');
  const operation = optionalBodyString(value, 'operation');
  const operationAbiId = optionalBodyString(value, 'operationAbiId');
  const payloadBase64 = optionalBodyString(value, 'payloadBase64', { allowEmpty: true });
  const serviceId = optionalBodyString(value, 'serviceId');
  const timeoutMs = optionalPositiveInteger(value, 'timeoutMs');
  const testEffectsEnabled = optionalBoolean(value, 'testEffectsEnabled');
  const testEffectDoubles = optionalTestEffectDoubles(value, 'testEffectDoubles');
  const websocketEntryId = optionalBodyString(value, 'websocketEntryId');
  return {
    ...(kind !== undefined ? { kind } : {}),
    ...(activationIdentity !== undefined ? { activationIdentity } : {}),
    buildId,
    serviceProtocolIdentity,
    target,
    ...(mode !== undefined ? { mode } : {}),
    ...(operation !== undefined ? { operation } : {}),
    ...(operationAbiId !== undefined ? { operationAbiId } : {}),
    ...(payloadBase64 !== undefined ? { payloadBase64 } : {}),
    ...(serviceId !== undefined ? { serviceId } : {}),
    ...(timeoutMs !== undefined ? { timeoutMs } : {}),
    ...(testEffectsEnabled !== undefined ? { testEffectsEnabled } : {}),
    ...(testEffectDoubles !== undefined ? { testEffectDoubles } : {}),
    ...(websocketEntryId !== undefined ? { websocketEntryId } : {})
  };
}

function parsePackageTestDispatchRequest(
  value: Record<string, unknown>
): PackageTestDispatchRequest {
  rejectPresentFields(
    value,
    SERVICE_TEST_DISPATCH_FIELDS,
    'packageTest test dispatch'
  );
  for (const field of Object.keys(value)) {
    if (!PACKAGE_TEST_ALLOWED_FIELDS.has(field)) {
      throw new GatewayError(
        400,
        'InvalidTestDispatchRequest',
        `${field} is not supported for packageTest test dispatch`
      );
    }
  }
  const packageId = requireBodyString(value, 'packageId');
  const packageVersion = requireBodyString(value, 'packageVersion');
  const testBuildIdentity = requireBodyString(value, 'testBuildIdentity');
  const entrypointId = requireBodyString(value, 'entrypointId');
  const activationId = requireBodyString(value, 'activationId');
  const payloadBase64 = optionalBodyString(value, 'payloadBase64', { allowEmpty: true });
  const timeoutMs = optionalPositiveInteger(value, 'timeoutMs');
  const testEffectsEnabled = optionalBoolean(value, 'testEffectsEnabled');
  const testEffectDoubles = optionalTestEffectDoubles(value, 'testEffectDoubles');
  return {
    kind: 'packageTest',
    packageId,
    packageVersion,
    testBuildIdentity,
    entrypointId,
    activationId,
    ...(payloadBase64 !== undefined ? { payloadBase64 } : {}),
    ...(timeoutMs !== undefined ? { timeoutMs } : {}),
    ...(testEffectsEnabled !== undefined ? { testEffectsEnabled } : {}),
    ...(testEffectDoubles !== undefined ? { testEffectDoubles } : {})
  };
}

function rejectPresentFields(
  value: Record<string, unknown>,
  fields: readonly string[],
  dispatchKind: string
): void {
  for (const field of fields) {
    if (Object.prototype.hasOwnProperty.call(value, field)) {
      throw new GatewayError(
        400,
        'InvalidTestDispatchRequest',
        `${field} is not supported for ${dispatchKind}`
      );
    }
  }
}

function timeoutFromManifest(
  snapshot: RouterActiveSnapshot,
  operation: OperationManifest | undefined,
  operationName: string | undefined
): number | undefined {
  if (operation !== undefined) {
    return (
      snapshot.manifest.timeout?.methods?.[operation.operation] ??
      snapshot.manifest.timeout?.methods?.[operation.target] ??
      snapshot.manifest.timeout?.defaultMs
    );
  }
  if (operationName !== undefined) {
    return snapshot.manifest.timeout?.methods?.[operationName];
  }
  return undefined;
}

function decodePayloadBase64(payloadBase64: string | undefined): Buffer {
  if (payloadBase64 === undefined) {
    return Buffer.alloc(0);
  }
  if (
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(payloadBase64)
  ) {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', 'payloadBase64 must be valid base64');
  }
  return Buffer.from(payloadBase64, 'base64');
}

function serviceIdFromTarget(target: string): string | undefined {
  const [namespace, serviceComponent] = target.split('.');
  if (
    (namespace !== 'service' && namespace !== 'gateway') ||
    serviceComponent === undefined ||
    serviceComponent.length === 0
  ) {
    return undefined;
  }
  const serviceId = serviceComponent.replaceAll('~~', '/').replaceAll('~', '.');
  return isPublicationId(serviceId) && publicationStorageSegment(serviceId) === serviceComponent
    ? serviceId
    : undefined;
}

function requireBodyString(value: Record<string, unknown>, field: string): string {
  const parsed = optionalBodyString(value, field);
  if (parsed === undefined) {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', `${field} is required`);
  }
  return parsed;
}

function optionalBodyString(
  value: Record<string, unknown>,
  field: string,
  options: { allowEmpty?: boolean } = {}
): string | undefined {
  const parsed = value[field];
  if (parsed === undefined) {
    return undefined;
  }
  if (typeof parsed !== 'string' || (!options.allowEmpty && parsed.length === 0)) {
    throw new GatewayError(
      400,
      'InvalidTestDispatchRequest',
      `${field} must be a non-empty string`
    );
  }
  return parsed;
}

function optionalDispatchMode(
  value: Record<string, unknown>,
  field: string
): DispatchMode | undefined {
  const parsed = value[field];
  if (parsed === undefined) {
    return undefined;
  }
  if (parsed !== 'unary' && parsed !== 'serverStream') {
    throw new GatewayError(
      400,
      'InvalidTestDispatchRequest',
      `${field} must be unary or serverStream`
    );
  }
  return parsed;
}

function optionalPositiveInteger(
  value: Record<string, unknown>,
  field: string
): number | undefined {
  const parsed = value[field];
  if (parsed === undefined) {
    return undefined;
  }
  if (!Number.isInteger(parsed) || Number(parsed) <= 0) {
    throw new GatewayError(
      400,
      'InvalidTestDispatchRequest',
      `${field} must be a positive integer`
    );
  }
  return Number(parsed);
}

function optionalBoolean(value: Record<string, unknown>, field: string): boolean | undefined {
  const parsed = value[field];
  if (parsed === undefined) {
    return undefined;
  }
  if (typeof parsed !== 'boolean') {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', `${field} must be a boolean`);
  }
  return parsed;
}

function optionalTestEffectDoubles(
  value: Record<string, unknown>,
  field: string
): TestEffectDoubles | undefined {
  const parsed = value[field];
  if (parsed === undefined) {
    return undefined;
  }
  if (!isRecord(parsed)) {
    throw new GatewayError(400, 'InvalidTestDispatchRequest', `${field} must be an object`);
  }
  for (const [target, sequence] of Object.entries(parsed)) {
    if (!Array.isArray(sequence) || sequence.length === 0) {
      throw new GatewayError(
        400,
        'InvalidTestDispatchRequest',
        `${field}.${target} must be a non-empty array`
      );
    }
    for (const [index, step] of sequence.entries()) {
      if (!isRecord(step) || !Object.prototype.hasOwnProperty.call(step, 'response')) {
        throw new GatewayError(
          400,
          'InvalidTestDispatchRequest',
          `${field}.${target}[${index}].response is required`
        );
      }
    }
  }
  return parsed as TestEffectDoubles;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function requestUrl(request: IncomingMessage): URL {
  return new URL(request.url ?? '/', `http://${request.headers.host ?? 'localhost'}`);
}
