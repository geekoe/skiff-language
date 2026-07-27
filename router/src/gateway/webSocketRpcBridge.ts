import { randomUUID } from 'node:crypto';

import {
  activationGeneration,
  runtimeAssemblyIdentity
} from '../protocol/assemblyActivationLexical.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionRequestFrameHeader,
  type ConnectionResponseFrameHeader
} from '../protocol/envelope.js';
import { JsonRpc20TextProfile } from '../protocol/jsonRpc20TextProfile.js';
import type {
  ProfileId,
  WebSocketRpcProfileAdapter
} from '../protocol/jsonRpc20TextProfileContracts.js';
import type {
  RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  isCanonicalRuntimeAssemblyWebSocketBusinessIdentity,
  isCanonicalRuntimeAssemblyWebSocketConnectionId,
  isCanonicalRuntimeAssemblyWebSocketEntryId
} from '../protocol/runtimeAssemblyRequestMetadata.js';
import {
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeToRouterFrameHeader
} from '../protocol/runtimeProtocol.js';
import {
  GatewayError,
  RuntimeTimeoutError
} from '../router/errors.js';
import type {
  RuntimeAssemblyDeploymentRef
} from '../router/runtimeAssemblySnapshot.js';
import type {
  RuntimeAssemblyWebSocketMethodBinding
} from '../router/runtimeAssemblyWebSocketSnapshot.js';
import type {
  RuntimeAssemblyWebSocketJsonRpcDispatchResponse,
  RuntimeDispatchConnectionReceipt,
  RuntimeDispatcher
} from '../router/runtimeDispatcher.js';
import type {
  RuntimeConnectionRequestMessage,
  RuntimeConnectionRequestSource,
  RuntimeConnectionRequestSourceApi
} from '../router/runtimeEndpoint.js';
import {
  DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS,
  WebSocketRequestBroker,
  type BrokerConnectionGeneration,
  type BrokerRuntimeResponse,
  type BrokerRuntimeSource,
  type CapturedPeerWriter,
  type InboundDispatchAction,
  type InboundDispatchResult,
  type WebSocketRequestBrokerClock,
  type WebSocketRequestBrokerSnapshot
} from '../router/webSocketRequestBroker.js';

export type WebSocketRpcBridgeDispatcher = Pick<
  RuntimeDispatcher,
  | 'dispatchAssemblyWebSocketJsonRpc'
  | 'isRuntimeConnectionReceiptSender'
>;

export interface CapturedWebSocketRpcRuntimeOwner {
  readonly serviceId: string;
  readonly assemblyIdentity: string;
  readonly assemblyGeneration: number;
  readonly replicaId: string;
}

export interface CapturedWebSocketRpcConnection {
  readonly socketGeneration: string;
  readonly connectionId: string;
  readonly serviceId: string;
  readonly deployment: RuntimeAssemblyDeploymentRef;
  readonly assemblyIdentity: string;
  readonly assemblyGeneration: number;
  readonly websocketEntryId: string;
  readonly host: string;
  readonly path: string;
  readonly profile: ProfileId;
  readonly profileAdapter: WebSocketRpcProfileAdapter;
  readonly methodTable: ReadonlyMap<
    string,
    RuntimeAssemblyWebSocketMethodBinding
  >;
  readonly businessIdentity?: string;
  readonly writer: CapturedPeerWriter;
  readonly routerRequestTimeoutMs: number;
  readonly deploymentTimeoutMs?: number;
  readonly runtimeReceipt?: RuntimeDispatchConnectionReceipt;
  readonly runtimeReplicaId?: string;
  readonly runtimeOwner: (
    source: RuntimeConnectionRequestSource
  ) => CapturedWebSocketRpcRuntimeOwner | undefined;
  readonly releaseGeneration: () => void | Promise<void>;
}

export interface WebSocketRpcBridgeOptions {
  readonly endpoint: RuntimeConnectionRequestSourceApi;
  readonly dispatcher: WebSocketRpcBridgeDispatcher;
  readonly profiles?: readonly WebSocketRpcProfileAdapter[];
  readonly clock?: WebSocketRequestBrokerClock;
}

export interface WebSocketRpcBridgeConnectionHandle {
  handlePeerText(frame: string): void;
  handlePeerBinary(): void;
  handlePeerDisconnect(): Promise<void>;
  finalize(): Promise<void>;
  debugSnapshot(): WebSocketRpcBridgeDebugSnapshot;
}

export interface WebSocketRpcBridgeDebugSnapshot
  extends WebSocketRequestBrokerSnapshot {
  readonly attachedConnectionCount: number;
  readonly closed: boolean;
}

interface CapturedConnectionState {
  readonly context: CapturedWebSocketRpcConnection;
  readonly methodTable: ReadonlyMap<
    string,
    RuntimeAssemblyWebSocketMethodBinding
  >;
  readonly ownerToken: object;
  readonly brokerGeneration: BrokerConnectionGeneration;
  readonly inboundTimeoutMs: number;
  finalized: boolean;
  finalizePromise?: Promise<void>;
}

export class WebSocketRpcBridge {
  private readonly broker: WebSocketRequestBroker;
  private readonly connectionsById = new Map<string, CapturedConnectionState>();
  private readonly connectionsByGeneration =
    new Map<string, CapturedConnectionState>();
  private readonly endpointSourceByBrokerSource =
    new WeakMap<BrokerRuntimeSource, RuntimeConnectionRequestSource>();
  private readonly unsubscribeConnectionRequest: () => void;
  private readonly unsubscribeSourceDisconnect: () => void;
  private closed = false;
  private cleanupPromise?: Promise<void>;

  constructor(private readonly options: WebSocketRpcBridgeOptions) {
    const profiles =
      options.profiles ??
      [
        new JsonRpc20TextProfile(
          DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS.profileLimits
        )
      ];
    this.broker = new WebSocketRequestBroker({
      ...DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS,
      profiles,
      ...(options.clock === undefined ? {} : { clock: options.clock }),
      dispatchInbound: (action) => this.dispatchInbound(action),
      onRuntimeProtocolViolation: (source, reason) => {
        const endpointSource = this.endpointSourceByBrokerSource.get(source);
        if (endpointSource !== undefined) {
          this.options.endpoint.isolateConnectionRequestSource(
            endpointSource,
            reason
          );
        }
      }
    });
    this.unsubscribeConnectionRequest = options.endpoint.onConnectionRequest(
      (message, source) => this.handleRuntimeMessage(message, source)
    );
    this.unsubscribeSourceDisconnect =
      options.endpoint.onConnectionRequestSourceDisconnect((source) => {
        this.broker.handleRuntimeDisconnect(brokerSourceKey(source));
      });
  }

  attach(
    input: CapturedWebSocketRpcConnection
  ): WebSocketRpcBridgeConnectionHandle {
    if (this.closed) {
      throw new Error('WebSocket RPC bridge is closed');
    }
    const captured = captureConnection(input);
    if (this.connectionsById.has(captured.context.connectionId)) {
      throw new Error('external WebSocket connection id is already attached');
    }
    const identityKey = generationIdentityKey(
      captured.context.connectionId,
      captured.context.socketGeneration
    );
    if (this.connectionsByGeneration.has(identityKey)) {
      throw new Error('WebSocket connection generation is already attached');
    }

    const ownerToken = Object.freeze({});
    const brokerGeneration = this.broker.attachGeneration({
      connectionId: captured.context.connectionId,
      socketGeneration: captured.context.socketGeneration,
      serviceId: captured.context.serviceId,
      websocketEntryId: captured.context.websocketEntryId,
      ownerToken,
      profile: captured.context.profile,
      profileAdapter: captured.context.profileAdapter,
      inboundTimeoutMs: captured.inboundTimeoutMs,
      outboundIdPrefix: captured.context.socketGeneration,
      writer: captured.context.writer,
      acceptInboundMethod: (method) => captured.methodTable.has(method)
    });
    const state: CapturedConnectionState = {
      ...captured,
      ownerToken,
      brokerGeneration,
      finalized: false
    };
    this.connectionsById.set(state.context.connectionId, state);
    this.connectionsByGeneration.set(identityKey, state);

    return Object.freeze({
      handlePeerText: (frame: string) => {
        this.broker.handlePeerText(state.brokerGeneration, frame);
      },
      handlePeerBinary: () => {
        this.broker.handlePeerBinary(state.brokerGeneration);
      },
      handlePeerDisconnect: () => this.finalizeConnection(state),
      finalize: () => this.finalizeConnection(state),
      debugSnapshot: () => this.debugSnapshot()
    });
  }

  debugSnapshot(): WebSocketRpcBridgeDebugSnapshot {
    return {
      ...this.broker.debugSnapshot(),
      attachedConnectionCount: this.connectionsById.size,
      closed: this.closed
    };
  }

  cleanup(): Promise<void> {
    if (this.cleanupPromise !== undefined) {
      return this.cleanupPromise;
    }
    this.closed = true;
    this.unsubscribeConnectionRequest();
    this.unsubscribeSourceDisconnect();
    const finalizations = Array.from(this.connectionsById.values(), (state) =>
      this.finalizeConnection(state)
    );
    this.cleanupPromise = Promise.all(finalizations).then(() => undefined);
    return this.cleanupPromise;
  }

  private handleRuntimeMessage(
    message: RuntimeConnectionRequestMessage,
    source: RuntimeConnectionRequestSource
  ): void {
    const validation = validateRuntimeToRouterFrameHeader(message.header);
    if (!validation.ok) {
      if (
        message.kind === 'request' &&
        isCanonicalBoundedString(message.header.requestId, 1024)
      ) {
        this.sendRuntimeResponse(source, {
          requestId: message.header.requestId,
          outcome: 'protocolError'
        });
      }
      this.options.endpoint.isolateConnectionRequestSource(
        source,
        validation.error
      );
      return;
    }
    if (message.kind === 'cancel') {
      if (validation.envelope.type !== 'connection.request.cancel') {
        this.options.endpoint.isolateConnectionRequestSource(
          source,
          'connection request kind does not match its validated header'
        );
        return;
      }
      this.broker.handleRuntimeCancel(
        brokerSourceKey(source),
        validation.envelope.requestId
      );
      return;
    }
    if (validation.envelope.type !== 'connection.request') {
      this.sendRuntimeResponse(source, {
        requestId: message.header.requestId,
        outcome: 'protocolError'
      });
      this.options.endpoint.isolateConnectionRequestSource(
        source,
        'connection request kind does not match its validated header'
      );
      return;
    }
    this.handleRuntimeRequest(
      validation.envelope,
      message.payloadBytes,
      source
    );
  }

  private handleRuntimeRequest(
    header: ConnectionRequestFrameHeader,
    payloadBytes: Uint8Array,
    source: RuntimeConnectionRequestSource
  ): void {
    const state = this.connectionsById.get(header.connectionId);
    if (
      state === undefined ||
      header.serviceId !== state.context.serviceId ||
      header.websocketEntryId !== state.context.websocketEntryId
    ) {
      this.sendRuntimeResponse(source, {
        requestId: header.requestId,
        outcome: 'connectionUnavailable'
      });
      return;
    }
    if (header.profile !== state.context.profile) {
      this.rejectRuntimeProtocol(
        source,
        header.requestId,
        'connection.request profile does not match the captured connection'
      );
      return;
    }

    let runtimeOwner: CapturedWebSocketRpcRuntimeOwner | undefined;
    try {
      runtimeOwner = state.context.runtimeOwner(source);
    } catch {
      runtimeOwner = undefined;
    }
    if (!this.runtimeSourceMatches(state, source, runtimeOwner)) {
      this.rejectRuntimeProtocol(
        source,
        header.requestId,
        'connection.request source does not own the captured connection generation'
      );
      return;
    }

    const brokerSource = this.brokerSource(source);
    this.broker.handleRuntimeRequest(state.brokerGeneration, {
      source: brokerSource,
      requestId: header.requestId,
      serviceId: header.serviceId,
      websocketEntryId: header.websocketEntryId,
      ownerToken: state.ownerToken,
      profile: header.profile,
      method: header.method,
      payloadBytes,
      ...(header.deadline === undefined
        ? {}
        : {
            deadlineAtMs: Math.min(
              Date.parse(header.deadline.expiresAt),
              this.now() + header.deadline.timeoutMs
            )
          })
    });
  }

  private runtimeSourceMatches(
    state: CapturedConnectionState,
    source: RuntimeConnectionRequestSource,
    owner: CapturedWebSocketRpcRuntimeOwner | undefined
  ): boolean {
    const context = state.context;
    if (
      owner === undefined ||
      owner.serviceId !== context.serviceId ||
      owner.assemblyIdentity !== context.assemblyIdentity ||
      owner.assemblyGeneration !== context.assemblyGeneration
    ) {
      return false;
    }
    if (context.runtimeReceipt === undefined) {
      return state.methodTable.size === 0;
    }
    return (
      owner.replicaId === context.runtimeReplicaId &&
      this.options.dispatcher.isRuntimeConnectionReceiptSender(
        context.runtimeReceipt,
        source.sender
      )
    );
  }

  private rejectRuntimeProtocol(
    source: RuntimeConnectionRequestSource,
    requestId: string,
    reason: string
  ): void {
    this.sendRuntimeResponse(source, {
      requestId,
      outcome: 'protocolError'
    });
    this.options.endpoint.isolateConnectionRequestSource(source, reason);
  }

  private brokerSource(
    source: RuntimeConnectionRequestSource
  ): BrokerRuntimeSource {
    const brokerSource: BrokerRuntimeSource = {
      sender: source.sender,
      sessionToken: source.sessionToken,
      respond: (response) => this.sendRuntimeResponse(source, response)
    };
    this.endpointSourceByBrokerSource.set(brokerSource, source);
    return brokerSource;
  }

  private sendRuntimeResponse(
    source: RuntimeConnectionRequestSource,
    response: BrokerRuntimeResponse
  ): void {
    const header = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.response',
      requestId: response.requestId,
      outcome: response.outcome,
      ...(response.remote === undefined
        ? {}
        : { remote: { ...response.remote } })
    } satisfies ConnectionResponseFrameHeader;
    try {
      this.options.endpoint.sendConnectionResponse(
        source,
        header,
        response.payloadBytes
      );
    } catch {
      // The broker has already detached; a stale captured runtime cannot reopen it.
    }
  }

  private async dispatchInbound(
    action: InboundDispatchAction
  ): Promise<InboundDispatchResult> {
    const state = this.connectionsByGeneration.get(
      generationIdentityKey(action.connectionId, action.socketGeneration)
    );
    if (state === undefined || state.finalized) {
      return { kind: 'runtimeUnavailable' };
    }
    const binding = state.methodTable.get(action.method);
    const receipt = state.context.runtimeReceipt;
    if (binding === undefined || receipt === undefined) {
      return { kind: 'runtimeUnavailable' };
    }

    let payloadBytes: Uint8Array;
    let header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;
    try {
      payloadBytes = state.context.profileAdapter.toRuntimePayload(
        action.params,
        DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS.profileLimits
      );
      header = this.inboundRequestHeader(state, binding);
    } catch {
      return { kind: 'internalError' };
    }

    let response: RuntimeAssemblyWebSocketJsonRpcDispatchResponse;
    try {
      response =
        await this.options.dispatcher.dispatchAssemblyWebSocketJsonRpc(
          { header, payloadBytes },
          state.inboundTimeoutMs,
          receipt,
          { signal: action.signal }
        );
    } catch (error) {
      return error instanceof RuntimeTimeoutError
        ? { kind: 'deadlineExceeded' }
        : error instanceof GatewayError
          ? { kind: 'runtimeUnavailable' }
          : { kind: 'internalError' };
    }

    switch (response.header.websocketJsonRpc.outcome) {
      case 'success':
        try {
          return {
            kind: 'success',
            result: state.context.profileAdapter.fromRuntimePayload(
              response.payloadBytes,
              'inboundResult',
              DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS.profileLimits
            )
          };
        } catch {
          return { kind: 'internalError' };
        }
      case 'invalidParams':
        return { kind: 'invalidParams' };
      case 'internalError':
        return { kind: 'internalError' };
      case 'deadlineExceeded':
        return { kind: 'deadlineExceeded' };
    }
  }

  private inboundRequestHeader(
    state: CapturedConnectionState,
    binding: RuntimeAssemblyWebSocketMethodBinding
  ): RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
    const context = state.context;
    const candidate = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId: randomUUID(),
      mode: 'unary',
      caller: { kind: 'gateway' },
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity: context.assemblyIdentity,
        assemblyGeneration: context.assemblyGeneration,
        gatewayEntryIdentity: binding.gatewayEntryIdentity,
        ingress: {
          protocol: 'webSocket',
          host: context.host,
          method: binding.method,
          path: context.path
        }
      },
      deadline: {
        timeoutMs: state.inboundTimeoutMs,
        expiresAt: new Date(
          this.now() + state.inboundTimeoutMs
        ).toISOString()
      },
      trace: {
        traceId: randomUUID(),
        spanId: randomUUID()
      },
      websocketJsonRpc: {
        profile: context.profile,
        connectionId: context.connectionId,
        websocketEntryId: context.websocketEntryId,
        gatewayEntryIdentity: binding.gatewayEntryIdentity,
        ...(context.businessIdentity === undefined
          ? {}
          : { businessIdentity: context.businessIdentity })
      },
      testEffectsEnabled: false
    } as const;
    const validation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(candidate);
    if (
      !validation.ok ||
      validation.envelope.routing.ingress.protocol !== 'webSocket' ||
      validation.envelope.routing.ingress.method === null ||
      !('websocketJsonRpc' in validation.envelope)
    ) {
      throw new Error(
        validation.ok
          ? 'captured WebSocket RPC method normalized to the wrong wire branch'
          : validation.error
      );
    }
    return validation.envelope;
  }

  private finalizeConnection(
    state: CapturedConnectionState
  ): Promise<void> {
    if (state.finalizePromise !== undefined) {
      return state.finalizePromise;
    }
    state.finalized = true;
    this.broker.handlePeerDisconnect(state.brokerGeneration);
    this.connectionsById.delete(state.context.connectionId);
    this.connectionsByGeneration.delete(
      generationIdentityKey(
        state.context.connectionId,
        state.context.socketGeneration
      )
    );
    try {
      state.finalizePromise = Promise.resolve(
        state.context.releaseGeneration()
      );
    } catch (error) {
      state.finalizePromise = Promise.reject(error);
    }
    return state.finalizePromise;
  }

  private now(): number {
    return this.options.clock?.now() ?? Date.now();
  }
}

function captureConnection(input: CapturedWebSocketRpcConnection): {
  readonly context: CapturedWebSocketRpcConnection;
  readonly methodTable: ReadonlyMap<
    string,
    RuntimeAssemblyWebSocketMethodBinding
  >;
  readonly inboundTimeoutMs: number;
} {
  validateCapturedConnection(input);
  const methodTable = new Map<string, RuntimeAssemblyWebSocketMethodBinding>();
  for (const [method, binding] of input.methodTable) {
    if (
      method !== binding.method ||
      !isCanonicalBoundedString(method, 256) ||
      binding.profile !== input.profile ||
      binding.websocketEntryId !== input.websocketEntryId ||
      !sameDeployment(binding.deployment, input.deployment) ||
      !isCanonicalBoundedString(binding.gatewayEntryIdentity, 1024) ||
      !isCanonicalBoundedString(binding.gatewayEntryKey, 1024) ||
      !isCanonicalBoundedString(binding.handler, 1024) ||
      (binding.timeoutMs !== undefined &&
        (!Number.isSafeInteger(binding.timeoutMs) ||
          binding.timeoutMs <= 0))
    ) {
      throw new Error(
        'captured WebSocket RPC method does not match its physical connection'
      );
    }
    methodTable.set(method, freezeMethodBinding(binding));
  }
  if (methodTable.size > 0 && input.runtimeReceipt === undefined) {
    throw new Error(
      'method-bearing WebSocket connection requires a captured runtime receipt'
    );
  }
  if (
    (input.runtimeReceipt === undefined) !==
    (input.runtimeReplicaId === undefined)
  ) {
    throw new Error(
      'captured runtime receipt and replica owner must be present together'
    );
  }
  const inboundTimeoutMs =
    input.deploymentTimeoutMs === undefined
      ? input.routerRequestTimeoutMs
      : Math.min(
          input.routerRequestTimeoutMs,
          input.deploymentTimeoutMs
        );
  const context = Object.freeze({
    ...input,
    deployment: Object.freeze({ ...input.deployment }),
    methodTable,
    writer: input.writer
  });
  return { context, methodTable, inboundTimeoutMs };
}

function validateCapturedConnection(
  input: CapturedWebSocketRpcConnection
): void {
  if (!isCanonicalBoundedString(input.socketGeneration, 200)) {
    throw new Error('socketGeneration must be a bounded canonical token');
  }
  if (
    !isCanonicalRuntimeAssemblyWebSocketConnectionId(input.connectionId)
  ) {
    throw new Error('connectionId is not canonical');
  }
  if (
    !isCanonicalBoundedString(input.serviceId, 1024) ||
    input.deployment.serviceId !== input.serviceId
  ) {
    throw new Error('captured service/deployment owner does not match');
  }
  if (
    !isCanonicalBoundedString(input.deployment.contractVersion, 1024) ||
    !isCanonicalBoundedString(input.deployment.deploymentRevision, 1024) ||
    !isCanonicalBoundedString(
      input.deployment.deploymentArtifactIdentity,
      1024
    )
  ) {
    throw new Error('captured deployment owner is not canonical');
  }
  runtimeAssemblyIdentity(input.assemblyIdentity);
  activationGeneration(
    input.assemblyGeneration,
    'captured WebSocket assembly generation'
  );
  if (
    !isCanonicalRuntimeAssemblyWebSocketEntryId(input.websocketEntryId)
  ) {
    throw new Error('captured WebSocket entry identity is not canonical');
  }
  if (
    !isCanonicalBoundedString(input.host, 1024) ||
    !isCanonicalBoundedString(input.path, 4096) ||
    !input.path.startsWith('/')
  ) {
    throw new Error('captured WebSocket host/path is not canonical');
  }
  if (input.profileAdapter.profile !== input.profile) {
    throw new Error('captured profile adapter does not match its profile');
  }
  if (
    input.businessIdentity !== undefined &&
    !isCanonicalRuntimeAssemblyWebSocketBusinessIdentity(
      input.businessIdentity
    )
  ) {
    throw new Error('captured WebSocket business identity is not canonical');
  }
  for (const [label, value] of [
    ['routerRequestTimeoutMs', input.routerRequestTimeoutMs],
    ['deploymentTimeoutMs', input.deploymentTimeoutMs]
  ] as const) {
    if (
      value !== undefined &&
      (!Number.isSafeInteger(value) || value <= 0)
    ) {
      throw new Error(`${label} must be a positive safe integer`);
    }
  }
  if (
    input.runtimeReplicaId !== undefined &&
    !isCanonicalBoundedString(input.runtimeReplicaId, 1024)
  ) {
    throw new Error('captured runtime replica owner is not canonical');
  }
}

function freezeMethodBinding(
  binding: RuntimeAssemblyWebSocketMethodBinding
): RuntimeAssemblyWebSocketMethodBinding {
  return Object.freeze({
    ...binding,
    deployment: Object.freeze({ ...binding.deployment })
  });
}

function sameDeployment(
  left: RuntimeAssemblyDeploymentRef,
  right: RuntimeAssemblyDeploymentRef
): boolean {
  return (
    left.serviceId === right.serviceId &&
    left.contractVersion === right.contractVersion &&
    left.deploymentRevision === right.deploymentRevision &&
    left.deploymentArtifactIdentity === right.deploymentArtifactIdentity
  );
}

function generationIdentityKey(
  connectionId: string,
  socketGeneration: string
): string {
  return JSON.stringify([connectionId, socketGeneration]);
}

function brokerSourceKey(
  source: RuntimeConnectionRequestSource
): BrokerRuntimeSource {
  return {
    sender: source.sender,
    sessionToken: source.sessionToken,
    respond: () => undefined
  };
}

function isCanonicalBoundedString(
  value: unknown,
  maxBytes: number
): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.trim() === value &&
    !/\p{Cc}/u.test(value) &&
    Buffer.byteLength(value, 'utf8') <= maxBytes
  );
}
