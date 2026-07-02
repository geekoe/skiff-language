import type { DispatchMode } from '../protocol/envelope.js';

export type JsonSchema =
  | { type: 'any'; nullable?: boolean }
  | { type: 'null'; nullable?: boolean }
  | {
      type: 'string';
      nullable?: boolean;
      enum?: string[];
      contentEncoding?: string;
      format?: string;
      xSkiffSymbol?: string;
    }
  | { type: 'number'; nullable?: boolean }
  | { type: 'integer'; nullable?: boolean }
  | { type: 'boolean'; nullable?: boolean }
  | { type: 'json'; nullable?: boolean; xSkiffSymbol?: string }
  | { type: 'array'; nullable?: boolean; items: JsonSchema }
  | { type?: never; oneOf: JsonSchema[]; nullable?: boolean }
  | {
      type: 'object';
      nullable?: boolean;
      properties?: Record<string, JsonSchema>;
      required?: string[];
      additionalProperties?: boolean;
      xSkiffSymbol?: string;
    };

export interface OperationParameterManifest {
  name: string;
  schema: JsonSchema;
}

export interface OperationManifest {
  operation: string;
  operationAbiId: string;
  target: string;
  mode: DispatchMode;
  serviceProtocolIdentity?: string;
  parameters: OperationParameterManifest[];
  response: JsonSchema;
  timeoutMs?: number;
}

export type ServiceAccessVisibility = 'public' | 'internal';
export type ServiceAccessOrganizationRole = 'viewer' | 'maintainer' | 'owner';

export interface ServiceAccessManifest {
  visibility: ServiceAccessVisibility;
  organizationRole?: ServiceAccessOrganizationRole;
}

export interface RawHttpGatewayManifest {
  operation: string;
  target: string;
}

export interface HttpRouteServiceFunctionHandlerManifest {
  kind: 'serviceFunction';
  source?: string;
  modulePath?: string;
  symbol?: string;
}

export interface HttpRoutePackageFunctionHandlerManifest {
  kind: 'packageFunction';
  source?: string;
  packageId: string;
  alias?: string;
  symbolPath: string;
}

export type HttpRouteHandlerManifest =
  | HttpRouteServiceFunctionHandlerManifest
  | HttpRoutePackageFunctionHandlerManifest;

export interface HttpRouteTypedBodyManifest {
  schema?: JsonSchema | null;
}

export interface HttpRouteTypedResponseManifest {
  schema: JsonSchema;
}

export type HttpRouteAdapterKind = 'typedJson' | 'rawHttp';

export interface HttpRouteAdapterServiceFunctionManifest {
  kind: 'serviceFunction';
  modulePath: string;
  symbol: string;
}

export interface HttpRouteAdapterPackageFunctionManifest {
  kind: 'packageFunction';
  packageId: string;
  symbolPath: string;
}

export type HttpRouteAdapterCallableManifest =
  | HttpRouteAdapterServiceFunctionManifest
  | HttpRouteAdapterPackageFunctionManifest;

export type GatewayAdapterSourceKind =
  | 'http.request'
  | 'http.body'
  | 'http.context'
  | 'websocket.connectRequest'
  | 'websocket.receiveEvent'
  | 'websocket.connection'
  | 'websocket.connectionContext'
  | 'websocket.message'
  | 'websocket.messageBody'
  | 'websocket.connectionId'
  | 'websocket.businessIdentity';

export interface GatewayAdapterSourceManifest {
  kind: GatewayAdapterSourceKind;
}

export interface GatewayAdapterArgManifest {
  param: string;
  source: GatewayAdapterSourceManifest;
}

export interface HttpRouteAdapterManifest {
  kind: HttpRouteAdapterKind;
  handler: HttpRouteAdapterCallableManifest;
  guard?: HttpRouteAdapterCallableManifest;
  pre?: HttpRouteAdapterCallableManifest;
  adapterArgs?: GatewayAdapterArgManifest[];
}

export interface HttpRouteTypedManifest {
  body?: HttpRouteTypedBodyManifest | null;
  response: HttpRouteTypedResponseManifest;
  ingressIdentity: string;
  adapter?: HttpRouteAdapterManifest;
}

export interface HttpRouteManifest {
  id?: string;
  path: string;
  method?: string;
  handler?: HttpRouteHandlerManifest;
  operation?: string;
  operationAbiId?: string;
  target?: string;
  serviceOperationTarget?: string;
  serviceProtocolIdentity?: string;
  gatewayEntryIdentity?: string;
  adapter?: HttpRouteAdapterManifest;
  typed?: HttpRouteTypedManifest;
}

export interface LoadedRawHttpGateway {
  buildId?: string;
  serviceId: string;
  serviceProtocolIdentity: string;
  operation: string;
  operationAbiId: string;
  target: string;
  operationManifest: OperationManifest;
}

export interface LoadedHttpRoute extends HttpRouteManifest {
  buildId?: string;
  serviceId: string;
  serviceProtocolIdentity: string;
  method: string;
  gatewayTarget: string;
  dispatchTarget: string;
  operationAbiId: string;
  selector: string;
  requestParameterName: string;
  operationManifest?: OperationManifest;
}

export interface WebSocketConnectManifest {
  operation: string;
  operationAbiId: string;
  adapterArgs: GatewayAdapterArgManifest[];
  serviceOperationTarget?: string;
  serviceProtocolIdentity?: string;
  gatewayEntryIdentity?: string;
}

export interface WebSocketReceiveManifest {
  operation: string;
  operationAbiId: string;
  adapterArgs: GatewayAdapterArgManifest[];
  serviceOperationTarget?: string;
  serviceProtocolIdentity?: string;
  gatewayEntryIdentity?: string;
}

export type WebSocketContextExpectationManifest =
  | {
      kind: 'null';
    }
  | {
      kind: 'typed';
      connectOperationAbiId: string;
      contextTypeIdentity: string;
    };

export interface WebSocketEntryManifest {
  id: string;
  path?: string;
  serviceParam?: string;
  context?: JsonSchema;
  contextExpectation?: WebSocketContextExpectationManifest;
  connect?: WebSocketConnectManifest;
  receive: WebSocketReceiveManifest;
  routes?: unknown[];
  gatewayEntryIdentity?: string;
}

export interface SkiffRuntimeManifest {
  schemaVersion: 'skiff-runtime-manifest-v1';
  service: {
    id: string;
    revisionId: string;
    protocolIdentity: string;
    access?: ServiceAccessManifest;
  };
  operations: OperationManifest[];
  gateway?: {
    http?: {
      raw?: RawHttpGatewayManifest;
      routes?: HttpRouteManifest[];
    };
    websocket?: WebSocketEntryManifest;
  };
  timeout?: {
    defaultMs?: number;
    methods?: Record<string, number>;
  };
}

export interface LoadedWebSocketConnect extends WebSocketConnectManifest {
  gatewayEntryIdentity: string;
  operationManifest: OperationManifest;
}

export interface LoadedWebSocketReceive extends WebSocketReceiveManifest {
  gatewayEntryIdentity: string;
  operationManifest: OperationManifest;
}

export interface LoadedWebSocketEntry extends WebSocketEntryManifest {
  buildId?: string;
  connect?: LoadedWebSocketConnect;
  receive: LoadedWebSocketReceive;
  gatewayEntryIdentity: string;
  serviceId: string;
  serviceProtocolIdentity: string;
}

export interface LoadedManifest extends SkiffRuntimeManifest {
  httpRouteEntries: LoadedHttpRoute[];
  operationsByName: Map<string, OperationManifest>;
  operationsByTarget: Map<string, OperationManifest>;
  rawHttpEntries: LoadedRawHttpGateway[];
  websocketEntry?: LoadedWebSocketEntry;
  websocketEntries: LoadedWebSocketEntry[];
}
