import { randomUUID } from 'node:crypto';

import WebSocket from 'ws';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import {
  WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
  assertWebSocketGenerationLifecycleResponseMatches,
  type WebSocketGenerationAcquireControl,
  type WebSocketGenerationLifecycleControl,
  type WebSocketGenerationLifecycleRejectControl,
  type WebSocketGenerationLifecycleRequest,
  type WebSocketGenerationLifecycleResponse,
  type WebSocketGenerationLifecycleTuple,
  type WebSocketGenerationReleaseControl
} from '../protocol/webSocketGenerationLifecycle.js';
import type {
  RuntimeDispatchConnectionReceipt,
  RuntimeDispatcher
} from './runtimeDispatcher.js';

const DEFAULT_RELEASE_TIMEOUT_MS = 5_000;

export interface WebSocketGenerationLifecycleControlSender {
  sendWebSocketGenerationControl(
    ws: WebSocket,
    control: WebSocketGenerationLifecycleControl
  ): void;
}

export interface WebSocketGenerationPinExpectation {
  serviceId: string;
  assemblyIdentity: string;
  assemblyGeneration: number;
  websocketEntryId: string;
  connectionId: string;
}

interface ExpectedConnection extends WebSocketGenerationPinExpectation {
  acquired?: AcquiredConnection;
}

interface AcquiredConnection {
  tuple: WebSocketGenerationLifecycleTuple;
  ws: WebSocket;
}

interface PendingRelease {
  request: WebSocketGenerationReleaseControl;
  ws: WebSocket;
  promise: Promise<void>;
  resolve(): void;
  reject(error: unknown): void;
  timeout: NodeJS.Timeout;
}

interface CachedAcquire {
  request: WebSocketGenerationAcquireControl;
  response: WebSocketGenerationLifecycleResponse;
  ws: WebSocket;
}

export class WebSocketGenerationLifecycleRouter {
  private readonly expectedByConnectionId = new Map<string, ExpectedConnection>();
  private readonly pendingReleaseByConnectionId = new Map<string, PendingRelease>();
  private readonly pendingReleaseByRequestId = new Map<string, PendingRelease>();
  private readonly cachedAcquireByRequestId = new Map<string, CachedAcquire>();
  private readonly routerSessionByRuntime = new Map<WebSocket, string>();
  private readonly runtimeByRouterSession = new Map<string, WebSocket>();
  private readonly releaseAckCountByRuntime = new Map<WebSocket, number>();
  private readonly disconnectHandlers =
    new Set<(connectionId: string) => void>();
  private readonly releaseFailures: unknown[] = [];

  constructor(
    private readonly options: {
      dispatcher: RuntimeDispatcher;
      sender: WebSocketGenerationLifecycleControlSender;
      releaseTimeoutMs?: number;
    }
  ) {}

  expectConnection(expectation: WebSocketGenerationPinExpectation): void {
    if (this.expectedByConnectionId.has(expectation.connectionId)) {
      throw new Error(`duplicate WebSocket generation pin ${expectation.connectionId}`);
    }
    this.expectedByConnectionId.set(expectation.connectionId, { ...expectation });
  }

  requireAcquired(
    connectionId: string,
    receipt: RuntimeDispatchConnectionReceipt
  ): WebSocketGenerationLifecycleTuple {
    const expected = this.expectedByConnectionId.get(connectionId);
    const acquired = expected?.acquired;
    if (expected === undefined || acquired === undefined) {
      throw new Error(
        `runtime did not acquire the WebSocket generation pin for ${connectionId}`
      );
    }
    if (!this.options.dispatcher.isRuntimeConnectionReceiptSender(receipt, acquired.ws)) {
      throw new Error(
        `WebSocket generation pin ${connectionId} was acquired by a different runtime`
      );
    }
    return { ...acquired.tuple };
  }

  releaseConnection(connectionId: string): Promise<void> {
    const existingRelease = this.pendingReleaseByConnectionId.get(connectionId);
    if (existingRelease !== undefined) {
      return existingRelease.promise;
    }
    const expected = this.expectedByConnectionId.get(connectionId);
    this.expectedByConnectionId.delete(connectionId);
    const acquired = expected?.acquired;
    this.forgetCachedAcquire(connectionId);
    if (acquired === undefined || acquired.ws.readyState !== WebSocket.OPEN) {
      return Promise.resolve();
    }

    const request: WebSocketGenerationReleaseControl = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'release',
      requestId:
        `skiff-websocket-lifecycle-request-v1:opaque:${randomUUID()}`,
      sender: 'router',
      tuple: { ...acquired.tuple }
    };
    let resolve!: () => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<void>((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    });
    void promise.catch((error: unknown) => {
      this.releaseFailures.push(error);
    });
    const pending: PendingRelease = {
      request,
      ws: acquired.ws,
      promise,
      resolve,
      reject,
      timeout: setTimeout(() => {
        this.finishRelease(
          request.requestId,
          new Error(`WebSocket generation release timed out for ${connectionId}`)
        );
        acquired.ws.close(1008, 'websocket generation release timed out');
      }, this.options.releaseTimeoutMs ?? DEFAULT_RELEASE_TIMEOUT_MS)
    };
    this.pendingReleaseByConnectionId.set(connectionId, pending);
    this.pendingReleaseByRequestId.set(request.requestId, pending);
    try {
      this.options.sender.sendWebSocketGenerationControl(acquired.ws, request);
    } catch (error) {
      this.finishRelease(request.requestId, error);
    }
    return promise;
  }

  handleRuntimeControl(
    ws: WebSocket,
    control: WebSocketGenerationLifecycleControl
  ): void {
    if (control.action === 'acquire') {
      this.options.sender.sendWebSocketGenerationControl(
        ws,
        this.handleAcquire(ws, control)
      );
      return;
    }
    if (
      (control.action === 'ack' || control.action === 'reject') &&
      control.operation === 'release'
    ) {
      this.handleReleaseResponse(ws, control);
      return;
    }
    throw new Error(
      `runtime sent unsupported WebSocket generation lifecycle ${control.action}`
    );
  }

  handleRuntimeDisconnect(ws: WebSocket): void {
    this.releaseAckCountByRuntime.delete(ws);
    const disconnectedIds: string[] = [];
    for (const [connectionId, expected] of this.expectedByConnectionId) {
      if (expected.acquired?.ws === ws) {
        this.expectedByConnectionId.delete(connectionId);
        disconnectedIds.push(connectionId);
      }
    }
    for (const pending of [...this.pendingReleaseByRequestId.values()]) {
      if (pending.ws === ws) {
        this.finishRelease(pending.request.requestId);
      }
    }
    const sessionId = this.routerSessionByRuntime.get(ws);
    if (sessionId !== undefined) {
      this.routerSessionByRuntime.delete(ws);
      if (this.runtimeByRouterSession.get(sessionId) === ws) {
        this.runtimeByRouterSession.delete(sessionId);
      }
    }
    for (const [requestId, cached] of this.cachedAcquireByRequestId) {
      if (cached.ws === ws) {
        this.cachedAcquireByRequestId.delete(requestId);
      }
    }
    for (const connectionId of disconnectedIds) {
      for (const handler of this.disconnectHandlers) {
        handler(connectionId);
      }
    }
  }

  onConnectionLost(handler: (connectionId: string) => void): () => void {
    this.disconnectHandlers.add(handler);
    return () => this.disconnectHandlers.delete(handler);
  }

  connectionPinCount(ws: WebSocket): number {
    let count = 0;
    for (const expected of this.expectedByConnectionId.values()) {
      if (expected.acquired?.ws === ws) count += 1;
    }
    for (const pending of this.pendingReleaseByRequestId.values()) {
      if (pending.ws === ws) count += 1;
    }
    return count;
  }

  connectionReleaseAckCount(ws: WebSocket): number {
    return this.releaseAckCountByRuntime.get(ws) ?? 0;
  }

  async flush(): Promise<void> {
    await Promise.allSettled(
      [...this.pendingReleaseByRequestId.values()].map((pending) => pending.promise)
    );
    if (this.releaseFailures.length > 0) {
      const failures = this.releaseFailures.splice(0);
      throw new AggregateError(failures, 'WebSocket generation release failed');
    }
  }

  private handleAcquire(
    ws: WebSocket,
    request: WebSocketGenerationAcquireControl
  ): WebSocketGenerationLifecycleResponse {
    const cached = this.cachedAcquireByRequestId.get(request.requestId);
    if (cached !== undefined) {
      if (
        cached.ws === ws &&
        tuplesEqual(cached.request.tuple, request.tuple)
      ) {
        return cached.response;
      }
      return rejection(request, 'request-conflict', 'acquire request id was reused');
    }

    const response = this.acquireResponse(ws, request);
    this.cachedAcquireByRequestId.set(request.requestId, {
      request,
      response,
      ws
    });
    return response;
  }

  private acquireResponse(
    ws: WebSocket,
    request: WebSocketGenerationAcquireControl
  ): WebSocketGenerationLifecycleResponse {
    const tuple = request.tuple;
    const sessionRuntime = this.runtimeByRouterSession.get(tuple.routerSessionId);
    const runtimeSession = this.routerSessionByRuntime.get(ws);
    if (
      (sessionRuntime !== undefined && sessionRuntime !== ws) ||
      (runtimeSession !== undefined && runtimeSession !== tuple.routerSessionId)
    ) {
      return rejection(
        request,
        'sender-mismatch',
        'router session does not belong to the runtime sender'
      );
    }
    const expected = this.expectedByConnectionId.get(tuple.connectionId);
    if (expected === undefined) {
      return rejection(request, 'not-acquired', 'connection is not pending admission');
    }
    if (!matchesExpectation(tuple, expected)) {
      return rejection(
        request,
        'tuple-mismatch',
        'acquire tuple does not match the selected RuntimeAssembly ingress'
      );
    }
    if (
      !this.options.dispatcher.isPendingWebSocketAcquireSender(ws, tuple)
    ) {
      return rejection(
        request,
        'sender-mismatch',
        'acquire sender does not own the pending WebSocket connect request'
      );
    }
    if (
      expected.acquired !== undefined &&
      (expected.acquired.ws !== ws ||
        !tuplesEqual(expected.acquired.tuple, tuple))
    ) {
      return rejection(
        request,
        'tuple-mismatch',
        'connection already has a different generation pin'
      );
    }
    this.routerSessionByRuntime.set(ws, tuple.routerSessionId);
    this.runtimeByRouterSession.set(tuple.routerSessionId, ws);
    expected.acquired ??= { tuple: { ...tuple }, ws };
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'ack',
      operation: 'acquire',
      requestId: request.requestId,
      sender: 'router',
      tuple: { ...tuple }
    };
  }

  private handleReleaseResponse(
    ws: WebSocket,
    response: WebSocketGenerationLifecycleResponse
  ): void {
    const pending = this.pendingReleaseByRequestId.get(response.requestId);
    if (pending === undefined) {
      throw new Error('unexpected WebSocket generation release response');
    }
    if (pending.ws !== ws) {
      throw new Error('WebSocket generation release response sender mismatch');
    }
    assertWebSocketGenerationLifecycleResponseMatches(
      pending.request as WebSocketGenerationLifecycleRequest,
      response
    );
    if (response.action === 'reject') {
      this.finishRelease(
        response.requestId,
        new Error(
          `runtime rejected WebSocket generation release: ${response.code}: ${response.reason}`
        )
      );
      ws.close(1008, 'websocket generation release rejected');
      return;
    }
    this.releaseAckCountByRuntime.set(
      ws,
      this.connectionReleaseAckCount(ws) + 1
    );
    this.finishRelease(response.requestId);
  }

  private finishRelease(requestId: string, error?: unknown): void {
    const pending = this.pendingReleaseByRequestId.get(requestId);
    if (pending === undefined) return;
    clearTimeout(pending.timeout);
    this.pendingReleaseByRequestId.delete(requestId);
    this.pendingReleaseByConnectionId.delete(
      pending.request.tuple.connectionId
    );
    if (error === undefined) pending.resolve();
    else pending.reject(error);
  }

  private forgetCachedAcquire(connectionId: string): void {
    for (const [requestId, cached] of this.cachedAcquireByRequestId) {
      if (cached.request.tuple.connectionId === connectionId) {
        this.cachedAcquireByRequestId.delete(requestId);
      }
    }
  }
}

function matchesExpectation(
  tuple: WebSocketGenerationLifecycleTuple,
  expected: WebSocketGenerationPinExpectation
): boolean {
  return (
    tuple.serviceId === expected.serviceId &&
    tuple.assemblyIdentity === expected.assemblyIdentity &&
    tuple.assemblyGeneration === expected.assemblyGeneration &&
    tuple.websocketEntryId === expected.websocketEntryId &&
    tuple.connectionId === expected.connectionId
  );
}

function rejection(
  request: WebSocketGenerationAcquireControl,
  code: WebSocketGenerationLifecycleRejectControl['code'],
  reason: string
): WebSocketGenerationLifecycleRejectControl {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    action: 'reject',
    operation: 'acquire',
    requestId: request.requestId,
    sender: 'router',
    tuple: { ...request.tuple },
    code,
    reason
  };
}

function tuplesEqual(
  left: WebSocketGenerationLifecycleTuple,
  right: WebSocketGenerationLifecycleTuple
): boolean {
  return (
    left.routerSessionId === right.routerSessionId &&
    left.serviceId === right.serviceId &&
    left.assemblyIdentity === right.assemblyIdentity &&
    left.assemblyGeneration === right.assemblyGeneration &&
    left.websocketEntryId === right.websocketEntryId &&
    left.connectionId === right.connectionId
  );
}
