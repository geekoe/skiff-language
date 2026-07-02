import { readFile } from 'node:fs/promises';

import { isPublicationId, publicationStorageSegment } from '../publicationId.js';
import { assertRevisionId } from './revisionId.js';
import {
  computeWebSocketConnectIdentity,
  computeWebSocketEntryIdentity,
  computeWebSocketReceiveIdentity,
  sha256Hex,
  stableStringify
} from './identity.js';
import type {
  GatewayAdapterArgManifest,
  GatewayAdapterSourceKind,
  SkiffRuntimeManifest,
  HttpRouteHandlerManifest,
  HttpRouteAdapterCallableManifest,
  HttpRouteAdapterKind,
  HttpRouteAdapterManifest,
  HttpRouteTypedManifest,
  JsonSchema,
  LoadedHttpRoute,
  LoadedManifest,
  LoadedRawHttpGateway,
  LoadedWebSocketConnect,
  LoadedWebSocketEntry,
  LoadedWebSocketReceive,
  OperationManifest,
  ServiceAccessManifest,
  WebSocketConnectManifest,
  WebSocketEntryManifest,
  WebSocketReceiveManifest
} from './types.js';

const PROTOCOL_IDENTITY_PATTERN = /^skiff-protocol-v1:sha256:[0-9a-f]{64}$/;
const GATEWAY_IDENTITY_PATTERN = /^skiff-gateway-v1:sha256:[0-9a-f]{64}$/;
const HTTP_INGRESS_IDENTITY_PATTERN = /^skiff-http-ingress-v1:sha256:[0-9a-f]{64}$/;
const GATEWAY_ADAPTER_SOURCE_KINDS = new Set<GatewayAdapterSourceKind>([
  'http.request',
  'http.body',
  'http.context',
  'websocket.connectRequest',
  'websocket.receiveEvent',
  'websocket.connection',
  'websocket.connectionContext',
  'websocket.message',
  'websocket.messageBody',
  'websocket.connectionId',
  'websocket.businessIdentity'
]);
const HTTP_ADAPTER_SOURCE_KINDS = new Set<GatewayAdapterSourceKind>([
  'http.request',
  'http.body',
  'http.context'
]);
const WEBSOCKET_CONNECT_ADAPTER_SOURCE_KINDS = new Set<GatewayAdapterSourceKind>([
  'websocket.connectRequest'
]);
const WEBSOCKET_RECEIVE_ADAPTER_SOURCE_KINDS = new Set<GatewayAdapterSourceKind>([
  'websocket.receiveEvent',
  'websocket.connection',
  'websocket.connectionContext',
  'websocket.message',
  'websocket.messageBody',
  'websocket.connectionId',
  'websocket.businessIdentity'
]);

export async function loadManifestFile(path: string): Promise<LoadedManifest> {
  const text = await readFile(path, 'utf8');
  return loadManifest(JSON.parse(text));
}

export async function loadManifestFiles(paths: string[]): Promise<LoadedManifest> {
  if (paths.length === 0) {
    throw new Error('at least one router manifest is required');
  }
  const manifests = await Promise.all(paths.map((path) => loadManifestFile(path)));
  if (manifests.length === 1) {
    return manifests[0]!;
  }
  return mergeLoadedManifests(manifests);
}

export function loadManifest(value: unknown): LoadedManifest {
  assertRecord(value, 'manifest');
  const manifest = value as unknown as SkiffRuntimeManifest;

  if (manifest.schemaVersion !== 'skiff-runtime-manifest-v1') {
    throw new Error('manifest.schemaVersion must be skiff-runtime-manifest-v1');
  }

  assertRecord(manifest.service, 'manifest.service');
  requireString(manifest.service.id, 'manifest.service.id');
  if (!isPublicationId(manifest.service.id)) {
    throw new Error('manifest.service.id must be a publication id');
  }
  requireString(manifest.service.revisionId, 'manifest.service.revisionId');
  assertRevisionId(manifest.service.revisionId, 'manifest.service.revisionId');
  requireString(manifest.service.protocolIdentity, 'manifest.service.protocolIdentity');
  assertProtocolIdentity(manifest.service.protocolIdentity, 'manifest.service.protocolIdentity');
  const serviceAccess = readServiceAccess(manifest.service.access, 'manifest.service.access');

  if (!Array.isArray(manifest.operations)) {
    throw new Error('manifest.operations must be an array');
  }

  const operationsByName = new Map<string, OperationManifest>();
  const operationsByTarget = new Map<string, OperationManifest>();
  const operations: OperationManifest[] = [];
  for (const operation of manifest.operations) {
    validateOperation(operation, manifest.service.id);
    if (operationsByName.has(operation.operation)) {
      throw new Error(`duplicate operation: ${operation.operation}`);
    }
    if (operationsByTarget.has(operation.target)) {
      throw new Error(`duplicate operation target: ${operation.target}`);
    }
    const serviceProtocolIdentity =
      operation.serviceProtocolIdentity ?? manifest.service.protocolIdentity;
    assertProtocolIdentity(
      serviceProtocolIdentity,
      `operation ${operation.operation}.serviceProtocolIdentity`
    );
    const loadedOperation = withGatewayOperationDefaults(
      operation,
      manifest,
      serviceProtocolIdentity
    );
    operationsByName.set(operation.operation, loadedOperation);
    operationsByTarget.set(operation.target, loadedOperation);
    operations.push(loadedOperation);
  }

  const httpGateway = loadHttpGateway(
    manifest.gateway?.http,
    manifest.service.id,
    manifest.service.protocolIdentity,
    operationsByName
  );

  const websocketEntry = manifest.gateway?.websocket
    ? loadWebSocketEntry({
        entry: manifest.gateway.websocket,
        manifest,
        serviceId: manifest.service.id,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        operationsByName
      })
    : undefined;

  const loadedManifest = {
    ...manifest,
    service: {
      ...manifest.service,
      access: serviceAccess
    },
    operations,
    operationsByName,
    operationsByTarget,
    httpRouteEntries: httpGateway.routeEntries,
    rawHttpEntries: httpGateway.rawEntries,
    websocketEntries: websocketEntry ? [websocketEntry] : []
  };
  if (websocketEntry) {
    return {
      ...loadedManifest,
      websocketEntry
    };
  }
  return loadedManifest;
}

export function mergeLoadedManifests(manifests: LoadedManifest[]): LoadedManifest {
  const operationsByName = new Map<string, OperationManifest>();
  const operationsByTarget = new Map<string, OperationManifest>();
  const operations: OperationManifest[] = [];
  const websocketEntries = manifests.flatMap((manifest) => manifest.websocketEntries);
  const httpRouteEntries = manifests.flatMap((manifest) => manifest.httpRouteEntries);
  const rawHttpEntries = manifests.flatMap((manifest) => manifest.rawHttpEntries);

  for (const manifest of manifests) {
    for (const operation of manifest.operations) {
      const operationKey = `${manifest.service.id}.${operation.operation}`;
      const targetKey = operation.target;
      if (operationsByName.has(operationKey)) {
        throw new Error(`duplicate operation: ${operationKey}`);
      }
      if (operationsByTarget.has(targetKey)) {
        throw new Error(`duplicate operation target: ${targetKey}`);
      }
      const serviceProtocolIdentity =
        operation.serviceProtocolIdentity ?? manifest.service.protocolIdentity;
      assertProtocolIdentity(
        serviceProtocolIdentity,
        `operation ${operation.operation}.serviceProtocolIdentity`
      );
      const loadedOperation = withGatewayOperationDefaults(
        operation,
        manifest,
        serviceProtocolIdentity
      );
      operationsByName.set(operationKey, loadedOperation);
      operationsByTarget.set(targetKey, loadedOperation);
      operations.push(loadedOperation);
    }
  }

  const [first] = manifests;
  return {
    schemaVersion: 'skiff-runtime-manifest-v1',
    service: {
      id: '__multi__',
      revisionId: multiManifestRevisionId(manifests),
      protocolIdentity: multiManifestProtocolIdentity(manifests),
      access: {
        visibility: 'public'
      }
    },
    operations,
    gateway: {},
    httpRouteEntries,
    operationsByName,
    operationsByTarget,
    rawHttpEntries,
    websocketEntries,
    ...(websocketEntries[0] ? { websocketEntry: websocketEntries[0] } : {}),
    ...(first?.timeout ? { timeout: first.timeout } : {})
  };
}

function readServiceAccess(value: unknown, label: string): ServiceAccessManifest {
  if (value === undefined || value === null) {
    return { visibility: 'public' };
  }
  assertRecord(value, label);
  const keys = Object.keys(value).filter(
    (key) => key !== 'visibility' && key !== 'organizationRole'
  );
  if (keys.length > 0) {
    throw new Error(
      `${label} does not support ${keys.map((key) => JSON.stringify(key)).join(', ')}`
    );
  }
  const visibility =
    value.visibility === undefined || value.visibility === null
      ? 'public'
      : readServiceAccessVisibility(value.visibility, `${label}.visibility`);
  const organizationRole =
    value.organizationRole === undefined || value.organizationRole === null
      ? undefined
      : readServiceAccessOrganizationRole(
          value.organizationRole,
          `${label}.organizationRole`
        );
  if (visibility === 'public' && organizationRole !== undefined) {
    throw new Error(`${label}.organizationRole is only allowed when visibility is internal`);
  }
  return {
    visibility,
    ...(visibility === 'internal'
      ? { organizationRole: organizationRole ?? 'viewer' }
      : {})
  };
}

function rejectUnsupportedManifestKeys(
  value: Record<string, unknown>,
  label: string,
  supported: ReadonlySet<string>
): void {
  const unsupported = Object.keys(value).filter((key) => !supported.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${label} does not support ${unsupported.map((key) => JSON.stringify(key)).join(', ')}`
    );
  }
}

function readServiceAccessVisibility(
  value: unknown,
  label: string
): ServiceAccessManifest['visibility'] {
  if (value === 'public' || value === 'internal') {
    return value;
  }
  throw new Error(`${label} must be public or internal`);
}

function readServiceAccessOrganizationRole(
  value: unknown,
  label: string
): NonNullable<ServiceAccessManifest['organizationRole']> {
  if (value === 'viewer' || value === 'maintainer' || value === 'owner') {
    return value;
  }
  throw new Error(`${label} must be viewer, maintainer, or owner`);
}

function withGatewayOperationDefaults(
  operation: OperationManifest,
  manifest: Pick<SkiffRuntimeManifest, 'service' | 'timeout'>,
  serviceProtocolIdentity: string
): OperationManifest {
  const timeoutMs =
    operation.timeoutMs ??
    manifest.timeout?.methods?.[operation.operation] ??
    manifest.timeout?.methods?.[operation.target] ??
    manifest.timeout?.defaultMs;
  return {
    ...operation,
    serviceProtocolIdentity,
    ...(timeoutMs !== undefined ? { timeoutMs } : {})
  };
}

function validateOperation(operation: OperationManifest, serviceId: string): void {
  assertRecord(operation, 'operation');
  requireString(operation.operation, 'operation.operation');
  requireString(operation.operationAbiId, `operation ${operation.operation}.operationAbiId`);
  requireString(operation.target, `operation ${operation.operation}.target`);
  validateProjectedTarget(
    operation.target,
    serviceId,
    `operation ${operation.operation}.target`
  );
  if (operation.mode !== 'unary' && operation.mode !== 'serverStream') {
    throw new Error(`operation ${operation.operation}.mode must be unary or serverStream`);
  }
  if (!Array.isArray(operation.parameters)) {
    throw new Error(`operation ${operation.operation}.parameters must be an array`);
  }
  for (const parameter of operation.parameters) {
    assertRecord(parameter, `operation ${operation.operation}.parameter`);
    requireString(parameter.name, `operation ${operation.operation}.parameter.name`);
    assertRecord(parameter.schema, `operation ${operation.operation}.parameter.schema`);
  }
  assertRecord(operation.response, `operation ${operation.operation}.response`);
}

function loadHttpGateway(
  http: unknown,
  serviceId: string,
  serviceProtocolIdentity: string,
  operationsByName: Map<string, OperationManifest>
): {
  rawEntries: LoadedRawHttpGateway[];
  routeEntries: LoadedHttpRoute[];
} {
  if (http === undefined || http === null) {
    return { rawEntries: [], routeEntries: [] };
  }
  assertRecord(http, 'gateway.http');
  let rawEntry: LoadedRawHttpGateway | undefined;
  if ('raw' in http) {
    rawEntry = loadRawHttpMetadata(
      http.raw,
      serviceId,
      serviceProtocolIdentity,
      operationsByName
    );
  }
  const routeEntries =
    'routes' in http
      ? loadHttpRoutes(
          http.routes,
          serviceId,
          serviceProtocolIdentity,
          operationsByName
        )
      : [];
  const keys = Object.keys(http).filter((key) => key !== 'raw' && key !== 'routes');
  if (keys.length > 0) {
    throw new Error(
      `gateway.http does not support ${keys
        .map((key) => JSON.stringify(key))
        .join(', ')}; HTTP dispatch uses routes[] or Skiff service selector headers`
    );
  }
  return {
    rawEntries: rawEntry ? [rawEntry] : [],
    routeEntries
  };
}

function loadHttpRoutes(
  value: unknown,
  serviceId: string,
  serviceProtocolIdentity: string,
  operationsByName: Map<string, OperationManifest>
): LoadedHttpRoute[] {
  if (!Array.isArray(value)) {
    throw new Error('gateway.http.routes must be an array');
  }
  const seen = new Set<string>();
  return value.map((routeValue, index) => {
    const label = `gateway.http.routes[${index}]`;
    assertRecord(routeValue, label);
    rejectUnsupportedHttpRouteKeys(routeValue, label);
    const path = readManifestString(routeValue.path, `${label}.path`);
    if (!path.startsWith('/')) {
      throw new Error(`${label}.path must start with /`);
    }
    const method = normalizeHttpMethod(
      routeValue.method === undefined || routeValue.method === null
        ? 'POST'
        : readManifestString(routeValue.method, `${label}.method`),
      `${label}.method`
    );
    const duplicateKey = `${method}\0${path}`;
    if (seen.has(duplicateKey)) {
      throw new Error(`${label} duplicates HTTP route ${method} ${path}`);
    }
    seen.add(duplicateKey);

    const handler = readHttpRouteHandler(routeValue.handler, `${label}.handler`);
    const typed = readHttpRouteTyped(routeValue.typed, `${label}.typed`);
    const adapter = readHttpRouteAdapter(routeValue.adapter, `${label}.adapter`);
    const operationName =
      routeValue.operation === undefined || routeValue.operation === null
        ? undefined
        : readManifestString(routeValue.operation, `${label}.operation`);
    const suppliedOperationAbiId =
      routeValue.operationAbiId === undefined || routeValue.operationAbiId === null
        ? undefined
        : readManifestString(routeValue.operationAbiId, `${label}.operationAbiId`);
    const adapterKind = adapter?.kind ?? typed?.adapter?.kind ?? (typed !== undefined ? 'typedJson' : undefined);
    const operation =
      handler?.kind === 'packageFunction' && operationName === undefined
        ? undefined
        : loadHttpRouteOperation(
            operationName,
            operationsByName,
            label,
            adapterKind
          );

    const routeServiceProtocolIdentity =
      operation?.serviceProtocolIdentity ?? serviceProtocolIdentity;
    const serviceOperationTarget =
      routeValue.serviceOperationTarget === undefined ||
      routeValue.serviceOperationTarget === null
        ? undefined
        : readManifestString(routeValue.serviceOperationTarget, `${label}.serviceOperationTarget`);
    if (
      serviceOperationTarget !== undefined &&
      (operation === undefined || serviceOperationTarget !== operation.target)
    ) {
      throw new Error(`${label}.serviceOperationTarget must match operation target`);
    }
    const suppliedServiceProtocolIdentity =
      routeValue.serviceProtocolIdentity === undefined ||
      routeValue.serviceProtocolIdentity === null
        ? undefined
        : readManifestString(routeValue.serviceProtocolIdentity, `${label}.serviceProtocolIdentity`);
    if (suppliedServiceProtocolIdentity !== undefined) {
      assertProtocolIdentity(suppliedServiceProtocolIdentity, `${label}.serviceProtocolIdentity`);
      if (suppliedServiceProtocolIdentity !== routeServiceProtocolIdentity) {
        throw new Error(`${label}.serviceProtocolIdentity must match operation serviceProtocolIdentity`);
      }
    }
    const target =
      routeValue.target === undefined || routeValue.target === null
        ? undefined
        : readManifestString(routeValue.target, `${label}.target`);
    if (target !== undefined) {
      validateProjectedTarget(target, serviceId, `${label}.target`);
    }
    const packageTarget =
      handler?.kind === 'packageFunction'
        ? packageHttpHandlerTarget(handler.packageId, handler.symbolPath)
        : undefined;
    if (handler?.kind === 'packageFunction' && target !== undefined && target !== packageTarget) {
      throw new Error(`${label}.target must match package handler target ${packageTarget}`);
    }
    const dispatchTarget = operation?.target ?? packageTarget;
    if (dispatchTarget === undefined) {
      throw new Error(`${label}.operation is required for service HTTP routes`);
    }
    let operationAbiId: string;
    if (operation !== undefined) {
      if (suppliedOperationAbiId === undefined) {
        throw new Error(`${label}.operationAbiId is required for service HTTP routes`);
      }
      if (suppliedOperationAbiId !== operation.operationAbiId) {
        throw new Error(`${label}.operationAbiId must match operation ${operation.operation}.operationAbiId`);
      }
      operationAbiId = suppliedOperationAbiId;
    } else {
      operationAbiId =
        suppliedOperationAbiId ??
        packageHttpRouteOperationAbiId({
          serviceId,
          method,
          path,
          dispatchTarget
        });
    }
    const selector = httpRouteSelector(method, path);
    const gatewayTarget =
      operation !== undefined && target && target !== operation.target
        ? target
        : defaultHttpRouteGatewayTarget(serviceId, method, path);
    const gatewayEntryIdentity =
      routeValue.gatewayEntryIdentity === undefined || routeValue.gatewayEntryIdentity === null
        ? undefined
        : readManifestString(routeValue.gatewayEntryIdentity, `${label}.gatewayEntryIdentity`);
    validateGatewayIdentityPattern(gatewayEntryIdentity, `${label}.gatewayEntryIdentity`);
    const requestParameterName = operation?.parameters[0]?.name ?? 'request';

    return {
      ...(routeValue.id !== undefined && routeValue.id !== null
        ? { id: readManifestString(routeValue.id, `${label}.id`) }
        : {}),
      path,
      method,
      ...(handler !== undefined ? { handler } : {}),
      ...(adapter !== undefined ? { adapter } : {}),
      ...(typed !== undefined ? { typed } : {}),
      ...(operationName !== undefined ? { operation: operationName } : {}),
      ...(suppliedOperationAbiId !== undefined ? { operationAbiId: suppliedOperationAbiId } : {}),
      ...(target !== undefined ? { target } : {}),
      ...(serviceOperationTarget !== undefined ? { serviceOperationTarget } : {}),
      ...(gatewayEntryIdentity !== undefined ? { gatewayEntryIdentity } : {}),
      gatewayTarget,
      dispatchTarget,
      operationAbiId,
      selector,
      requestParameterName,
      serviceId,
      serviceProtocolIdentity: routeServiceProtocolIdentity,
      ...(operation !== undefined ? { operationManifest: operation } : {})
    };
  });
}

function loadHttpRouteOperation(
  operationName: string | undefined,
  operationsByName: Map<string, OperationManifest>,
  label: string,
  adapterKind?: HttpRouteAdapterKind
): OperationManifest {
  if (operationName === undefined) {
    throw new Error(`${label}.operation is required for service HTTP routes`);
  }
  const operation = operationsByName.get(operationName);
  if (!operation) {
    throw new Error(`${label} references unknown operation ${operationName}`);
  }
  if (adapterKind !== undefined) {
    if (adapterKind === 'typedJson' && operation.mode !== 'unary') {
      throw new Error(`${label} typed HTTP adapter operation ${operationName} must be unary`);
    }
    if (
      adapterKind === 'rawHttp' &&
      operation.mode !== 'unary' &&
      operation.mode !== 'serverStream'
    ) {
      throw new Error(
        `${label} raw HTTP adapter operation ${operationName} must be unary or serverStream`
      );
    }
    return operation;
  }
  throw new Error(`${label}.adapter.kind is required for service HTTP routes`);
}

function readHttpRouteTyped(
  value: unknown,
  label: string
): HttpRouteTypedManifest | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  assertRecord(value, label);
  rejectUnsupportedHttpRouteTypedKeys(
    value,
    label,
    new Set(['body', 'response', 'ingressIdentity', 'adapter'])
  );

  const ingressIdentity = readManifestString(value.ingressIdentity, `${label}.ingressIdentity`);
  assertHttpIngressIdentity(ingressIdentity, `${label}.ingressIdentity`);

  const body = readHttpRouteTypedBody(value.body, `${label}.body`);
  const response = readHttpRouteTypedResponse(value.response, `${label}.response`);
  const adapter = readHttpRouteAdapter(value.adapter, `${label}.adapter`);
  if (adapter !== undefined && adapter.kind !== 'typedJson') {
    throw new Error(`${label}.adapter.kind must be typedJson`);
  }

  return {
    ...(body !== undefined ? { body } : {}),
    response,
    ingressIdentity,
    ...(adapter !== undefined ? { adapter } : {})
  };
}

function readHttpRouteTypedBody(
  value: unknown,
  label: string
): HttpRouteTypedManifest['body'] | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (value === null) {
    return null;
  }
  assertRecord(value, label);
  rejectUnsupportedHttpRouteTypedKeys(value, label, new Set(['schema']));
  if (value.schema === undefined || value.schema === null) {
    return {};
  }
  assertRecord(value.schema, `${label}.schema`);
  return { schema: value.schema as JsonSchema };
}

function readHttpRouteTypedResponse(
  value: unknown,
  label: string
): HttpRouteTypedManifest['response'] {
  assertRecord(value, label);
  rejectUnsupportedHttpRouteTypedKeys(value, label, new Set(['schema']));
  assertRecord(value.schema, `${label}.schema`);
  return { schema: value.schema as JsonSchema };
}

function readHttpRouteAdapter(
  value: unknown,
  label: string
): HttpRouteAdapterManifest | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  assertRecord(value, label);
  rejectUnsupportedHttpRouteTypedKeys(
    value,
    label,
    new Set(['kind', 'handler', 'guard', 'pre', 'adapterArgs'])
  );
  const kind = readManifestString(value.kind, `${label}.kind`);
  if (kind !== 'typedJson' && kind !== 'rawHttp') {
    throw new Error(`${label}.kind must be typedJson or rawHttp`);
  }
  const handler = readHttpRouteAdapterCallable(value.handler, `${label}.handler`);
  const guard =
    value.guard === undefined || value.guard === null
      ? undefined
      : readHttpRouteAdapterCallable(value.guard, `${label}.guard`);
  const pre =
    value.pre === undefined || value.pre === null
      ? undefined
      : readHttpRouteAdapterCallable(value.pre, `${label}.pre`);
  const adapterArgs = readOptionalGatewayAdapterArgs(
    value.adapterArgs,
    `${label}.adapterArgs`,
    HTTP_ADAPTER_SOURCE_KINDS,
    'http.request, http.body, or http.context'
  );
  return {
    kind,
    handler,
    ...(guard !== undefined ? { guard } : {}),
    ...(pre !== undefined ? { pre } : {}),
    ...(adapterArgs !== undefined ? { adapterArgs } : {})
  };
}

function readHttpRouteAdapterCallable(
  value: unknown,
  label: string
): HttpRouteAdapterCallableManifest {
  assertRecord(value, label);
  const kind = readManifestString(value.kind, `${label}.kind`);
  if (kind === 'serviceFunction') {
    rejectUnsupportedHttpRouteTypedKeys(value, label, new Set(['kind', 'modulePath', 'symbol']));
    return {
      kind,
      modulePath: readManifestString(value.modulePath, `${label}.modulePath`),
      symbol: readManifestString(value.symbol, `${label}.symbol`)
    };
  }
  if (kind === 'packageFunction') {
    rejectUnsupportedHttpRouteTypedKeys(value, label, new Set(['kind', 'packageId', 'symbolPath']));
    return {
      kind,
      packageId: readManifestString(value.packageId, `${label}.packageId`),
      symbolPath: readManifestString(value.symbolPath, `${label}.symbolPath`)
    };
  }
  throw new Error(`${label}.kind must be serviceFunction or packageFunction`);
}

function readOptionalGatewayAdapterArgs(
  value: unknown,
  label: string,
  allowedSourceKinds: ReadonlySet<GatewayAdapterSourceKind>,
  allowedSourceDescription: string
): GatewayAdapterArgManifest[] | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  return readGatewayAdapterArgs(value, label, allowedSourceKinds, allowedSourceDescription);
}

function readRequiredGatewayAdapterArgs(
  value: unknown,
  label: string,
  allowedSourceKinds: ReadonlySet<GatewayAdapterSourceKind>,
  allowedSourceDescription: string
): GatewayAdapterArgManifest[] {
  if (value === undefined || value === null) {
    throw new Error(`${label} must be an array`);
  }
  return readGatewayAdapterArgs(value, label, allowedSourceKinds, allowedSourceDescription);
}

function readGatewayAdapterArgs(
  value: unknown,
  label: string,
  allowedSourceKinds: ReadonlySet<GatewayAdapterSourceKind>,
  allowedSourceDescription: string
): GatewayAdapterArgManifest[] {
  if (!Array.isArray(value)) {
    throw new Error(`${label} must be an array`);
  }
  const params = new Set<string>();
  return value.map((item, index) => {
    const itemLabel = `${label}[${index}]`;
    assertRecord(item, itemLabel);
    rejectUnsupportedHttpRouteTypedKeys(item, itemLabel, new Set(['param', 'source']));
    const param = readManifestString(item.param, `${itemLabel}.param`);
    if (params.has(param)) {
      throw new Error(`${label} has duplicate param ${param}`);
    }
    params.add(param);
    assertRecord(item.source, `${itemLabel}.source`);
    rejectUnsupportedHttpRouteTypedKeys(item.source, `${itemLabel}.source`, new Set(['kind']));
    const kind = readManifestString(item.source.kind, `${itemLabel}.source.kind`);
    if (!isGatewayAdapterSourceKind(kind)) {
      throw new Error(`${itemLabel}.source.kind must be a known gateway adapter source`);
    }
    if (!allowedSourceKinds.has(kind)) {
      throw new Error(`${itemLabel}.source.kind must be ${allowedSourceDescription}`);
    }
    return { param, source: { kind } };
  });
}

function isGatewayAdapterSourceKind(value: string): value is GatewayAdapterSourceKind {
  return GATEWAY_ADAPTER_SOURCE_KINDS.has(value as GatewayAdapterSourceKind);
}

function readHttpRouteHandler(
  value: unknown,
  label: string
): HttpRouteHandlerManifest | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  assertRecord(value, label);
  const kind = readManifestString(value.kind, `${label}.kind`);
  if (kind === 'serviceFunction') {
    rejectUnsupportedHttpRouteHandlerKeys(
      value,
      label,
      new Set(['kind', 'source', 'modulePath', 'symbol'])
    );
    return {
      kind,
      ...(value.source !== undefined && value.source !== null
        ? { source: readManifestString(value.source, `${label}.source`) }
        : {}),
      ...(value.modulePath !== undefined && value.modulePath !== null
        ? { modulePath: readManifestString(value.modulePath, `${label}.modulePath`) }
        : {}),
      ...(value.symbol !== undefined && value.symbol !== null
        ? { symbol: readManifestString(value.symbol, `${label}.symbol`) }
        : {})
    };
  }
  if (kind === 'packageFunction') {
    rejectUnsupportedHttpRouteHandlerKeys(
      value,
      label,
      new Set(['kind', 'source', 'packageId', 'alias', 'symbolPath'])
    );
    return {
      kind,
      ...(value.source !== undefined && value.source !== null
        ? { source: readManifestString(value.source, `${label}.source`) }
        : {}),
      packageId: readManifestString(value.packageId, `${label}.packageId`),
      ...(value.alias !== undefined && value.alias !== null
        ? { alias: readManifestString(value.alias, `${label}.alias`) }
        : {}),
      symbolPath: readManifestString(value.symbolPath, `${label}.symbolPath`)
    };
  }
  throw new Error(`${label}.kind must be serviceFunction or packageFunction`);
}

function rejectUnsupportedHttpRouteHandlerKeys(
  handler: Record<string, unknown>,
  label: string,
  supported: ReadonlySet<string>
): void {
  const unsupported = Object.keys(handler).filter((key) => !supported.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${label} does not support ${unsupported.map((key) => JSON.stringify(key)).join(', ')}`
    );
  }
}

function rejectUnsupportedHttpRouteTypedKeys(
  value: Record<string, unknown>,
  label: string,
  supported: ReadonlySet<string>
): void {
  const unsupported = Object.keys(value).filter((key) => !supported.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${label} does not support ${unsupported.map((key) => JSON.stringify(key)).join(', ')}`
    );
  }
}

function rejectUnsupportedHttpRouteKeys(
  route: Record<string, unknown>,
  label: string
): void {
  const supported = new Set([
    'id',
    'path',
    'method',
    'handler',
    'operation',
    'operationAbiId',
    'target',
    'serviceOperationTarget',
    'serviceProtocolIdentity',
    'gatewayEntryIdentity',
    'adapter',
    'typed'
  ]);
  const unsupported = Object.keys(route).filter((key) => !supported.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${label} does not support ${unsupported.map((key) => JSON.stringify(key)).join(', ')}`
    );
  }
}

function normalizeHttpMethod(value: string, label: string): string {
  const method = value.trim().toUpperCase();
  if (!/^[!#$%&'*+.^_`|~0-9A-Z-]+$/.test(method)) {
    throw new Error(`${label} must be a valid HTTP method`);
  }
  return method;
}

function defaultHttpRouteGatewayTarget(serviceId: string, method: string, path: string): string {
  const serviceTargetComponent = publicationStorageSegment(serviceId);
  const route = path
    .replace(/[^A-Za-z0-9]+/g, '.')
    .replace(/^\.+|\.+$/g, '')
    .toLowerCase();
  return `gateway.${serviceTargetComponent}.http.${method.toLowerCase()}${route ? `.${route}` : ''}`;
}

export function packageHttpHandlerTarget(packageId: string, symbolPath: string): string {
  return `package.${encodePackageTargetSegment(packageId)}.${encodePackageTargetSegment(symbolPath)}`;
}

function packageHttpRouteOperationAbiId(input: {
  serviceId: string;
  method: string;
  path: string;
  dispatchTarget: string;
}): string {
  return `operation:http-route:${sha256Hex(
    stableStringify({
      serviceId: input.serviceId,
      method: input.method.toUpperCase(),
      path: input.path,
      dispatchTarget: input.dispatchTarget
    })
  )}`;
}

function httpRouteSelector(method: string, path: string): string {
  return `${method.toUpperCase()} ${path}`;
}

function encodePackageTargetSegment(value: string): string {
  let encoded = '';
  for (const byte of Buffer.from(value, 'utf8')) {
    if (
      (byte >= 0x41 && byte <= 0x5a) ||
      (byte >= 0x61 && byte <= 0x7a) ||
      (byte >= 0x30 && byte <= 0x39) ||
      byte === 0x5f ||
      byte === 0x2d
    ) {
      encoded += String.fromCharCode(byte);
    } else {
      encoded += `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
    }
  }
  return encoded;
}

function loadRawHttpMetadata(
  value: unknown,
  serviceId: string,
  serviceProtocolIdentity: string,
  operationsByName: Map<string, OperationManifest>
): LoadedRawHttpGateway {
  assertRecord(value, 'gateway.http.raw');
  requireString(value.operation, 'gateway.http.raw.operation');
  requireString(value.target, 'gateway.http.raw.target');
  validateProjectedTarget(value.target, serviceId, 'gateway.http.raw.target');
  const expectedTarget = `gateway.${publicationStorageSegment(serviceId)}.http.raw`;
  if (value.target !== expectedTarget) {
    throw new Error(`gateway.http.raw.target must be ${expectedTarget}`);
  }
  const operation = operationsByName.get(value.operation);
  if (!operation) {
    throw new Error(`gateway.http.raw references unknown operation ${value.operation}`);
  }
  if (operation.mode !== 'unary' && operation.mode !== 'serverStream') {
    throw new Error(
      `gateway.http.raw operation ${value.operation} must be unary or serverStream`
    );
  }
  return {
    serviceId,
    serviceProtocolIdentity: operation.serviceProtocolIdentity ?? serviceProtocolIdentity,
    operation: value.operation,
    operationAbiId: operation.operationAbiId,
    target: value.target,
    operationManifest: operation
  };
}

function validateProjectedTarget(target: string, serviceId: string, label: string): void {
  if (target.startsWith('package.')) {
    validatePackageTarget(target, label);
    return;
  }

  if (!target.startsWith('service.') && !target.startsWith('gateway.')) {
    return;
  }

  const [namespace, serviceComponent, ...suffix] = target.split('.');
  const expectedServiceComponent = publicationStorageSegment(serviceId);
  const expectedPrefix = `${namespace}.${expectedServiceComponent}`;

  if (
    serviceComponent !== expectedServiceComponent ||
    serviceComponent.includes('/') ||
    suffix.length === 0 ||
    suffix.some((component) => component.length === 0 || component.includes('/'))
  ) {
    throw new Error(`${label} must be ${expectedPrefix}.<target suffix>`);
  }
}

function validatePackageTarget(target: string, label: string): void {
  const [namespace, packageComponent, symbolComponent, ...extra] = target.split('.');
  if (
    namespace !== 'package' ||
    !packageComponent ||
    !symbolComponent ||
    extra.length > 0 ||
    packageComponent.includes('/') ||
    symbolComponent.includes('/')
  ) {
    throw new Error(`${label} must be package.<encoded package id>.<encoded symbol path>`);
  }
}

function isObjectSchema(
  schema: JsonSchema | undefined
): schema is Extract<JsonSchema, { type: 'object' }> {
  return schema?.type === 'object';
}

function isStringSchema(schema: JsonSchema | undefined): boolean {
  return schema?.type === 'string';
}

function loadWebSocketEntry(input: {
  entry: WebSocketEntryManifest;
  manifest: SkiffRuntimeManifest;
  serviceId: string;
  serviceProtocolIdentity: string;
  operationsByName: Map<string, OperationManifest>;
}): LoadedWebSocketEntry {
  validateWebSocketEntry(input.entry);
  const entry = input.entry;

  let connect: LoadedWebSocketConnect | undefined;
  let connectIdentityInput:
    | {
        connect: WebSocketConnectManifest;
        serviceProtocolIdentity: string;
      }
    | undefined;
  if (entry.connect) {
    const operation = input.operationsByName.get(entry.connect.operation);
    if (!operation) {
      throw new Error(
        `websocket ${entry.id} connect references unknown operation ${entry.connect.operation}`
      );
    }
    validateAdapterArgTargets(
      operation,
      entry.connect.adapterArgs,
      `websocket ${entry.id}.connect.adapterArgs`
    );
    assertWebSocketConnectResultSchema(
      operation.response,
      `websocket ${entry.id}.connect.response`
    );
    const serviceProtocolIdentity =
      operation.serviceProtocolIdentity ?? input.serviceProtocolIdentity;
    validateWebSocketOperationMetadata(
      entry.connect,
      operation,
      serviceProtocolIdentity,
      `websocket ${entry.id}.connect`
    );
    const operationManifest = withGatewayOperationDefaults(
      operation,
      input.manifest,
      serviceProtocolIdentity
    );
    const connectGatewayEntryIdentity = computeWebSocketConnectIdentity({
      serviceId: input.serviceId,
      entry,
      connect: entry.connect,
      serviceProtocolIdentity
    });
    validateOptionalGatewayIdentity(
      entry.connect.gatewayEntryIdentity,
      connectGatewayEntryIdentity,
      `websocket ${entry.id}.connect.gatewayEntryIdentity`
    );
    connect = {
      ...entry.connect,
      gatewayEntryIdentity: connectGatewayEntryIdentity,
      operationManifest
    };
    connectIdentityInput = {
      connect: entry.connect,
      serviceProtocolIdentity
    };
  }

  if (entry.context && !connect) {
    throw new Error(`websocket ${entry.id} declares context without connect`);
  }

  const receiveOperation = input.operationsByName.get(entry.receive.operation);
  if (!receiveOperation) {
    throw new Error(
      `websocket ${entry.id} receive references unknown operation ${entry.receive.operation}`
    );
  }
  validateAdapterArgTargets(
    receiveOperation,
    entry.receive.adapterArgs,
    `websocket ${entry.id}.receive.adapterArgs`
  );
  assertWebSocketReceiveResponseSchema(
    receiveOperation.response,
    `websocket ${entry.id}.receive.response`
  );
  const receiveServiceProtocolIdentity =
    receiveOperation.serviceProtocolIdentity ?? input.serviceProtocolIdentity;
  validateWebSocketOperationMetadata(
    entry.receive,
    receiveOperation,
    receiveServiceProtocolIdentity,
    `websocket ${entry.id}.receive`
  );
  const receiveOperationManifest = withGatewayOperationDefaults(
    receiveOperation,
    input.manifest,
    receiveServiceProtocolIdentity
  );
  const receiveGatewayEntryIdentity = computeWebSocketReceiveIdentity({
    serviceId: input.serviceId,
    entry,
    receive: entry.receive,
    serviceProtocolIdentity: receiveServiceProtocolIdentity
  });
  validateOptionalGatewayIdentity(
    entry.receive.gatewayEntryIdentity,
    receiveGatewayEntryIdentity,
    `websocket ${entry.id}.receive.gatewayEntryIdentity`
  );
  const receive: LoadedWebSocketReceive = {
    ...entry.receive,
    gatewayEntryIdentity: receiveGatewayEntryIdentity,
    operationManifest: receiveOperationManifest
  };

  const entryIdentityInput = {
    serviceId: input.serviceId,
    entry,
    receive: {
      receive: entry.receive,
      serviceProtocolIdentity: receiveServiceProtocolIdentity
    }
  };
  const gatewayEntryIdentity =
    connectIdentityInput
      ? computeWebSocketEntryIdentity({
          ...entryIdentityInput,
          connect: connectIdentityInput
        })
      : computeWebSocketEntryIdentity(entryIdentityInput);
  validateOptionalGatewayIdentity(
    entry.gatewayEntryIdentity,
    gatewayEntryIdentity,
    `websocket ${entry.id}.gatewayEntryIdentity`
  );

  const loadedEntry = {
    id: entry.id,
    ...(entry.path !== undefined ? { path: entry.path } : {}),
    ...(entry.serviceParam !== undefined ? { serviceParam: entry.serviceParam } : {}),
    ...(entry.context !== undefined ? { context: entry.context } : {}),
    ...(entry.contextExpectation !== undefined
      ? { contextExpectation: entry.contextExpectation }
      : {}),
    receive,
    gatewayEntryIdentity,
    serviceId: input.serviceId,
    serviceProtocolIdentity: input.serviceProtocolIdentity
  };
  if (connect) {
    return {
      ...loadedEntry,
      connect
    };
  }
  return loadedEntry;
}

function validateWebSocketOperationMetadata(
  entry: WebSocketConnectManifest | WebSocketReceiveManifest,
  operation: OperationManifest,
  serviceProtocolIdentity: string,
  label: string
): void {
  if (entry.operationAbiId !== operation.operationAbiId) {
    throw new Error(`${label}.operationAbiId must match operation ${operation.operation}.operationAbiId`);
  }
  if (
    entry.serviceOperationTarget !== undefined &&
    entry.serviceOperationTarget !== operation.target
  ) {
    throw new Error(`${label}.serviceOperationTarget must match operation target`);
  }
  if (
    entry.serviceProtocolIdentity !== undefined &&
    entry.serviceProtocolIdentity !== serviceProtocolIdentity
  ) {
    throw new Error(`${label}.serviceProtocolIdentity must match operation serviceProtocolIdentity`);
  }
}

function validateWebSocketEntry(entry: WebSocketEntryManifest): void {
  assertRecord(entry, 'gateway.websocket');
  rejectUnsupportedManifestKeys(
    entry,
    'gateway.websocket',
    new Set([
      'id',
      'path',
      'serviceParam',
      'context',
      'contextExpectation',
      'connect',
      'receive',
      'routes',
      'gatewayEntryIdentity'
    ])
  );
  requireString(entry.id, 'gateway.websocket.id');
  if (entry.path !== undefined) {
    requireString(entry.path, `websocket ${entry.id}.path`);
  }
  if (entry.path !== undefined && !entry.path.startsWith('/')) {
    throw new Error(`websocket ${entry.id}.path must start with /`);
  }
  if (entry.context !== undefined) {
    assertRecord(entry.context, `websocket ${entry.id}.context`);
  }
  if (entry.contextExpectation !== undefined) {
    validateWebSocketContextExpectation(
      entry.contextExpectation,
      `websocket ${entry.id}.contextExpectation`
    );
  }
  if (entry.serviceParam !== undefined) {
    requireString(entry.serviceParam, `websocket ${entry.id}.serviceParam`);
  }
  if (entry.connect !== undefined) {
    validateWebSocketConnect(entry.connect, `websocket ${entry.id}.connect`);
  }
  validateWebSocketReceive(entry.receive, `websocket ${entry.id}.receive`);
  if (entry.routes !== undefined) {
    if (!Array.isArray(entry.routes)) {
      throw new Error(`websocket ${entry.id}.routes must be an array`);
    }
    throw new Error(`websocket ${entry.id}.routes are no longer supported; use receive for application messages or HTTP for request-response`);
  }
}

function validateWebSocketContextExpectation(value: unknown, name: string): void {
  assertRecord(value, name);
  if (value.kind !== 'null' && value.kind !== 'typed') {
    throw new Error(`${name}.kind must be null or typed`);
  }
  if (value.kind === 'null') {
    rejectUnsupportedManifestKeys(value, name, new Set(['kind']));
    return;
  }
  rejectUnsupportedManifestKeys(
    value,
    name,
    new Set(['kind', 'connectOperationAbiId', 'contextTypeIdentity'])
  );
  requireString(value.connectOperationAbiId, `${name}.connectOperationAbiId`);
  requireString(value.contextTypeIdentity, `${name}.contextTypeIdentity`);
}

function validateWebSocketConnect(connect: WebSocketConnectManifest, name: string): void {
  assertRecord(connect, name);
  rejectUnsupportedManifestKeys(
    connect,
    name,
    new Set([
      'operation',
      'operationAbiId',
      'adapterArgs',
      'serviceOperationTarget',
      'serviceProtocolIdentity',
      'gatewayEntryIdentity'
    ])
  );
  requireString(connect.operation, `${name}.operation`);
  requireString(connect.operationAbiId, `${name}.operationAbiId`);
  readRequiredGatewayAdapterArgs(
    connect.adapterArgs,
    `${name}.adapterArgs`,
    WEBSOCKET_CONNECT_ADAPTER_SOURCE_KINDS,
    'websocket.connectRequest'
  );
}

function validateWebSocketReceive(receive: WebSocketReceiveManifest, name: string): void {
  assertRecord(receive, name);
  rejectUnsupportedManifestKeys(
    receive,
    name,
    new Set([
      'operation',
      'operationAbiId',
      'adapterArgs',
      'serviceOperationTarget',
      'serviceProtocolIdentity',
      'gatewayEntryIdentity'
    ])
  );
  requireString(receive.operation, `${name}.operation`);
  requireString(receive.operationAbiId, `${name}.operationAbiId`);
  readRequiredGatewayAdapterArgs(
    receive.adapterArgs,
    `${name}.adapterArgs`,
    WEBSOCKET_RECEIVE_ADAPTER_SOURCE_KINDS,
    'websocket.receiveEvent, websocket.connection, websocket.connectionContext, websocket.message, websocket.messageBody, websocket.connectionId, or websocket.businessIdentity'
  );
}

function validateAdapterArgTargets(
  operation: OperationManifest,
  adapterArgs: GatewayAdapterArgManifest[],
  name: string
): void {
  const parameters = new Set(operation.parameters.map((parameter) => parameter.name));
  const adapterParams = new Set(adapterArgs.map((arg) => arg.param));
  for (const parameter of operation.parameters) {
    if (!adapterParams.has(parameter.name)) {
      throw new Error(`${name} is missing operation parameter ${parameter.name}`);
    }
  }
  for (const arg of adapterArgs) {
    if (!parameters.has(arg.param)) {
      throw new Error(`${name} references unknown operation parameter ${arg.param}`);
    }
  }
}

function assertWebSocketConnectResultSchema(schema: JsonSchema, name: string): void {
  if (isWebSocketConnectResultUnionSchema(schema)) {
    return;
  }
  throw new Error(`${name} must be canonical WebSocketConnectResult oneOf schema`);
}

function isWebSocketConnectResultUnionSchema(schema: JsonSchema): boolean {
  const record = schema as Record<string, unknown>;
  const oneOf = record.oneOf;
  if (!Array.isArray(oneOf)) {
    return false;
  }
  if (oneOf.length !== 2) {
    return false;
  }
  return (
    oneOf.some((branch) =>
      isTaggedWebSocketConnectResultBranch(branch, {
        tag: 'accept',
        requiredFields: ['context'],
        optionalFields: ['businessIdentity', 'connectionPolicy'],
        nullableFields: ['businessIdentity'],
        fieldSchemas: {
          businessIdentity: 'string'
        }
      })
    ) &&
    oneOf.some((branch) =>
      isTaggedWebSocketConnectResultBranch(branch, {
        tag: 'reject',
        requiredFields: ['code', 'reason'],
        fieldSchemas: {
          code: 'integer',
          reason: 'string'
        }
      })
    )
  );
}

function isTaggedWebSocketConnectResultBranch(
  value: unknown,
  expected: {
    tag: 'accept' | 'reject';
    requiredFields: string[];
    optionalFields?: string[];
    nullableFields?: string[];
    fieldSchemas?: Record<string, JsonSchema['type']>;
  }
): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return false;
  }
  const schema = value as JsonSchema;
  if (schema.type !== 'object' || schema.additionalProperties !== false) {
    return false;
  }
  const properties = schema.properties ?? {};
  const expectedFields = ['tag', ...expected.requiredFields, ...(expected.optionalFields ?? [])].sort();
  const expectedRequiredFields = ['tag', ...expected.requiredFields].sort();
  const propertyFields = Object.keys(properties).sort();
  const requiredFields = [...(schema.required ?? [])].sort();
  return (
    schema.properties?.tag?.type === 'string' &&
    schema.properties.tag.enum?.length === 1 &&
    schema.properties.tag.enum[0] === expected.tag &&
    sameStrings(propertyFields, expectedFields) &&
    sameStrings(requiredFields, expectedRequiredFields) &&
    [...expected.requiredFields, ...(expected.optionalFields ?? [])].every((field) => {
      const expectedType = expected.fieldSchemas?.[field];
      if (expectedType !== undefined && properties[field]?.type !== expectedType) {
        return false;
      }
      if (field === 'connectionPolicy') {
        return isWebSocketConnectionPolicySchema(properties[field]);
      }
      if ((expected.nullableFields ?? []).includes(field)) {
        return properties[field]?.nullable === true;
      }
      return true;
    })
  );
}

function isWebSocketConnectionPolicySchema(schema: JsonSchema | undefined): boolean {
  if (!isObjectSchema(schema) || schema.additionalProperties !== false) {
    return false;
  }
  const properties = schema.properties ?? {};
  return (
    sameStrings(Object.keys(properties).sort(), [
      'closeCode',
      'closeReason',
      'maxConnections',
      'overflow'
    ]) &&
    sameStrings([...(schema.required ?? [])].sort(), ['maxConnections', 'overflow']) &&
    properties.maxConnections?.type === 'integer' &&
    isStringSchema(properties.overflow) &&
    properties.closeCode?.type === 'integer' &&
    properties.closeReason?.type === 'string'
  );
}

function sameStrings(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function assertWebSocketReceiveResponseSchema(schema: JsonSchema, name: string): void {
  if (schema.type === 'null') {
    return;
  }
  throw new Error(`${name} must be null or void response schema`);
}

function validateOptionalGatewayIdentity(
  supplied: string | undefined,
  expected: string | string[],
  name: string
): void {
  if (supplied === undefined) {
    return;
  }
  if (!GATEWAY_IDENTITY_PATTERN.test(supplied)) {
    throw new Error(`${name} must be skiff-gateway-v1:sha256:<64 lowercase hex>`);
  }
  const expectedIdentities = Array.isArray(expected) ? expected : [expected];
  if (!expectedIdentities.includes(supplied)) {
    throw new Error(`${name} must match computed gateway identity ${expectedIdentities[0]}`);
  }
}

function validateGatewayIdentityPattern(supplied: string | undefined, name: string): void {
  if (supplied === undefined) {
    return;
  }
  if (!GATEWAY_IDENTITY_PATTERN.test(supplied)) {
    throw new Error(`${name} must be skiff-gateway-v1:sha256:<64 lowercase hex>`);
  }
}

function assertHttpIngressIdentity(value: string, name: string): void {
  if (!HTTP_INGRESS_IDENTITY_PATTERN.test(value)) {
    throw new Error(`${name} must be skiff-http-ingress-v1:sha256:<64 lowercase hex>`);
  }
}

function assertProtocolIdentity(value: string, name: string): void {
  if (!PROTOCOL_IDENTITY_PATTERN.test(value)) {
    throw new Error(`${name} must be skiff-protocol-v1:sha256:<64 lowercase hex>`);
  }
}

function multiManifestRevisionId(manifests: LoadedManifest[]): string {
  const body = manifests.map((manifest) => ({
    serviceId: manifest.service.id,
    revisionId: manifest.service.revisionId,
    protocolIdentity: manifest.service.protocolIdentity
  }));
  return sha256Hex(stableStringify(body));
}

function multiManifestProtocolIdentity(manifests: LoadedManifest[]): string {
  const body = manifests.map((manifest) => ({
    serviceId: manifest.service.id,
    revisionId: manifest.service.revisionId,
    protocolIdentity: manifest.service.protocolIdentity
  }));
  return `skiff-protocol-v1:sha256:${sha256Hex(stableStringify(body))}`;
}

function assertRecord(value: unknown, name: string): asserts value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${name} must be an object`);
  }
}

function requireString(value: unknown, name: string): asserts value is string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${name} must be a non-empty string`);
  }
}

function readManifestString(value: unknown, name: string): string {
  requireString(value, name);
  return value;
}
