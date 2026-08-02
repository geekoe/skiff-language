import { performance } from 'node:perf_hooks';

import WebSocket from 'ws';

const DEFAULT_CONNECTION_LIMIT = 5_000;
const DEFAULT_ADMISSION_HIGH_WATER_RETENTION_MS = 120_000;
const DEFAULT_SLOW_CLIENT_BUDGET_BYTES = 16 * 1024 * 1024;
const DEFAULT_SHUTDOWN_TIMEOUT_MS = 1_000;
const MAX_WEBSOCKET_CLOSE_REASON_BYTES = 123;

export interface WebSocketConnectionPolicy {
  maxConnections: number;
  overflow: 'close-oldest' | 'reject-new';
  closeCode?: number;
  closeReason?: string;
}

export interface WebSocketLifecycleClose {
  code: number;
  reason: string;
}

export interface WebSocketLifecycleOutboundMessage {
  data: string | Uint8Array;
  binary: boolean;
}

export interface WebSocketLifecyclePeerWriter {
  writeText(frame: string): Promise<void>;
  close(code: number, reason: string): void;
}

export interface WebSocketConnectionLifecycleOptions {
  admissionHighWaterLimit?: number;
  admissionHighWaterRetentionMs?: number;
  connectionLimit?: number;
  now?: () => number;
  slowClientBudgetBytes?: number;
  shutdownTimeoutMs?: number;
}

export type WebSocketPolicyAdmission =
  | { accepted: true }
  | { accepted: false; close: WebSocketLifecycleClose };

type LifecycleState = 'reserved' | 'admitted' | 'closed';
export type WebSocketLifecyclePreAttachCloseDisposition =
  | 'abort-upgrade'
  | 'close-after-upgrade';

export interface WebSocketLifecyclePreAttachClose {
  close: WebSocketLifecycleClose;
  disposition: WebSocketLifecyclePreAttachCloseDisposition;
}

interface LifecycleConnection<TConnection, TRuntime> {
  admissionRank?: number;
  businessKey?: string;
  closeBeforeAttach?: (input: WebSocketLifecyclePreAttachClose) => void;
  id: string;
  observedWriteBytes: number;
  observedWrites: Set<ObservedWrite>;
  runtime?: TRuntime;
  socket?: WebSocket;
  state: LifecycleState;
  value: TConnection;
}

interface ObservedWrite {
  bytes: number;
  reject(error: Error): void;
  resolve(): void;
}

export class WebSocketConnectionLimitExceededError extends Error {
  constructor() {
    super('websocket gateway connection limit exceeded');
  }
}

export class WebSocketConnectionLifecycle<TConnection, TRuntime = unknown> {
  private readonly connectionLimit: number;
  private readonly admissionHighWaterLimit: number;
  private readonly admissionHighWaterRetentionMs: number;
  private readonly now: () => number;
  private readonly slowClientBudgetBytes: number;
  private readonly shutdownTimeoutMs: number;
  private readonly connectionsById = new Map<string, LifecycleConnection<TConnection, TRuntime>>();
  private readonly connectionsByBusinessKey = new Map<
    string,
    Set<LifecycleConnection<TConnection, TRuntime>>
  >();
  private readonly connectionsByRuntime = new Map<
    TRuntime,
    Set<LifecycleConnection<TConnection, TRuntime>>
  >();
  private readonly admissionHighWaterByBusinessKey = new Map<
    string,
    { rank: number; retainUntilMs: number }
  >();
  private newBusinessKeyQuarantineUntilMs: number | undefined;
  private shutdownPromise: Promise<void> | undefined;
  private shuttingDown = false;
  private readonly pendingFinalizations = new Set<Promise<void>>();
  private readonly finalizationFailures: unknown[] = [];

  constructor(
    options: WebSocketConnectionLifecycleOptions = {},
    private readonly onFinish?: (
      value: TConnection,
      runtime: TRuntime | undefined
    ) => void | Promise<void>
  ) {
    this.connectionLimit = positiveInteger(
      options.connectionLimit ?? DEFAULT_CONNECTION_LIMIT,
      'connectionLimit'
    );
    this.admissionHighWaterLimit = positiveInteger(
      options.admissionHighWaterLimit ?? this.connectionLimit,
      'admissionHighWaterLimit'
    );
    this.admissionHighWaterRetentionMs = positiveInteger(
      options.admissionHighWaterRetentionMs ?? DEFAULT_ADMISSION_HIGH_WATER_RETENTION_MS,
      'admissionHighWaterRetentionMs'
    );
    this.now = options.now ?? (() => Math.floor(performance.now()));
    this.slowClientBudgetBytes = nonNegativeInteger(
      options.slowClientBudgetBytes ?? DEFAULT_SLOW_CLIENT_BUDGET_BYTES,
      'slowClientBudgetBytes'
    );
    this.shutdownTimeoutMs = nonNegativeInteger(
      options.shutdownTimeoutMs ?? DEFAULT_SHUTDOWN_TIMEOUT_MS,
      'shutdownTimeoutMs'
    );
  }

  reserve(
    id: string,
    value: TConnection,
    runtime?: TRuntime,
    closeBeforeAttach?: (input: WebSocketLifecyclePreAttachClose) => void
  ): void {
    if (this.shuttingDown) {
      throw new Error('websocket connection lifecycle is shutting down');
    }
    if (this.connectionsById.has(id)) {
      throw new Error(`duplicate websocket connection id ${id}`);
    }
    if (this.connectionsById.size >= this.connectionLimit) {
      throw new WebSocketConnectionLimitExceededError();
    }
    const connection: LifecycleConnection<TConnection, TRuntime> = {
      id,
      observedWriteBytes: 0,
      observedWrites: new Set(),
      state: 'reserved',
      value,
      ...(runtime !== undefined ? { runtime } : {}),
      ...(closeBeforeAttach !== undefined ? { closeBeforeAttach } : {})
    };
    this.connectionsById.set(id, connection);
    if (runtime !== undefined) {
      addToIndex(this.connectionsByRuntime, runtime, connection);
    }
  }

  release(id: string): boolean {
    const connection = this.connectionsById.get(id);
    if (connection === undefined) {
      return false;
    }
    this.finishConnection(connection, undefined, false);
    return true;
  }

  admit(
    id: string,
    input: {
      businessKey?: string;
      policy?: WebSocketConnectionPolicy;
      admissionRank?: number;
    }
  ): WebSocketPolicyAdmission {
    const connection = this.requireConnection(id);
    if (connection.state !== 'reserved') {
      throw new Error(`websocket connection ${id} is already ${connection.state}`);
    }
    if (input.policy !== undefined && input.businessKey === undefined) {
      throw new Error('websocket connection policy requires a business key');
    }
    if (
      input.admissionRank !== undefined &&
      (!Number.isSafeInteger(input.admissionRank) || input.admissionRank <= 0)
    ) {
      throw new Error('websocket admission rank must be a positive safe integer');
    }
    if (input.admissionRank !== undefined && input.businessKey === undefined) {
      throw new Error('websocket admission rank requires a business key');
    }
    if (
      input.admissionRank !== undefined &&
      (input.policy?.overflow !== 'close-oldest' ||
        input.policy.maxConnections !== 1)
    ) {
      throw new Error(
        'websocket admission rank requires a single-connection close-oldest policy'
      );
    }

    const existing =
      input.businessKey === undefined
        ? []
        : Array.from(this.connectionsByBusinessKey.get(input.businessKey) ?? []);
    const now = this.monotonicNow();
    this.pruneAdmissionHighWater(now);
    const highWater =
      input.businessKey === undefined
        ? undefined
        : this.admissionHighWaterByBusinessKey.get(input.businessKey);
    if (
      highWater !== undefined &&
      (input.admissionRank === undefined || input.admissionRank <= highWater.rank)
    ) {
      const close = admissionSupersededClose();
      this.finishConnection(connection, undefined, false);
      return { accepted: false, close };
    }
    if (
      input.businessKey !== undefined &&
      highWater === undefined &&
      this.isNewBusinessKeyQuarantined(now)
    ) {
      return this.rejectForAdmissionHighWaterCapacity(
        connection,
        now,
        input.admissionRank === undefined ? [] : existing
      );
    }
    if (input.admissionRank !== undefined) {
      if (
        highWater === undefined &&
        this.admissionHighWaterByBusinessKey.size >= this.admissionHighWaterLimit
      ) {
        return this.rejectForAdmissionHighWaterCapacity(
          connection,
          now,
          existing
        );
      }
      this.admissionHighWaterByBusinessKey.set(
        input.businessKey!,
        {
          rank: input.admissionRank,
          // An earlier lower-rank dispatch is quarantined by its timeout before
          // this fence may be reclaimed. Active keys remain fenced separately.
          retainUntilMs: safeRetentionDeadline(
            now,
            this.admissionHighWaterRetentionMs
          )
        }
      );
      this.finishSupersededConnections(existing);
    }
    if (
      input.policy?.overflow === 'reject-new' &&
      existing.length >= input.policy.maxConnections
    ) {
      const close = policyOverflowClose(input.policy);
      this.finishConnection(connection, undefined, false);
      return { accepted: false, close };
    }

    if (input.policy?.overflow === 'close-oldest') {
      const overflowCount = existing.length + 1 - input.policy.maxConnections;
      for (const candidate of existing.slice(0, Math.max(0, overflowCount))) {
        this.finishConnection(candidate, policyOverflowClose(input.policy), true);
      }
    }

    connection.state = 'admitted';
    if (input.admissionRank !== undefined) {
      connection.admissionRank = input.admissionRank;
    }
    if (input.businessKey !== undefined) {
      connection.businessKey = input.businessKey;
      addToIndex(this.connectionsByBusinessKey, input.businessKey, connection);
    }
    return { accepted: true };
  }

  attach(id: string, socket: WebSocket): void {
    const connection = this.requireConnection(id);
    if (connection.state !== 'admitted') {
      throw new Error(`websocket connection ${id} is not admitted`);
    }
    if (connection.socket !== undefined) {
      throw new Error(`websocket connection ${id} already has a transport`);
    }
    connection.socket = socket;
    delete connection.closeBeforeAttach;
    socket.once('close', () => {
      if (connection.socket === socket) {
        this.finishConnection(connection, undefined, false);
      }
    });
    socket.once('error', () => {
      if (connection.socket === socket) {
        this.finishConnection(
          connection,
          { code: 1011, reason: 'websocket transport failed' },
          true
        );
      }
    });
    if (socket.readyState === WebSocket.CLOSED) {
      this.finishConnection(connection, undefined, false);
    }
  }

  bindRuntime(id: string, runtime: TRuntime): void {
    const connection = this.requireConnection(id);
    if (connection.runtime === runtime) {
      return;
    }
    if (connection.runtime !== undefined) {
      removeFromIndex(this.connectionsByRuntime, connection.runtime, connection);
    }
    connection.runtime = runtime;
    addToIndex(this.connectionsByRuntime, runtime, connection);
  }

  runtimeDisconnected(
    runtime: TRuntime,
    close: WebSocketLifecycleClose = {
      code: 1011,
      reason: 'websocket runtime disconnected'
    }
  ): number {
    const connections = Array.from(this.connectionsByRuntime.get(runtime) ?? []);
    for (const connection of connections) {
      this.finishConnection(connection, close, true);
    }
    return connections.length;
  }

  connection(id: string): TConnection | undefined {
    const connection = this.connectionsById.get(id);
    return connection?.state === 'admitted' ? connection.value : undefined;
  }

  connectionsForBusinessKey(businessKey: string): TConnection[] {
    return Array.from(this.connectionsByBusinessKey.get(businessKey) ?? [], ({ value }) => value);
  }

  sendToConnection(id: string, message: WebSocketLifecycleOutboundMessage): boolean {
    const connection = this.connectionsById.get(id);
    return connection === undefined ? false : this.send(connection, message);
  }

  capturePeerWriter(id: string): WebSocketLifecyclePeerWriter | undefined {
    const connection = this.connectionsById.get(id);
    if (
      connection === undefined ||
      connection.state !== 'admitted' ||
      connection.socket === undefined
    ) {
      return undefined;
    }
    return Object.freeze({
      writeText: (frame: string) => this.writeObservedText(connection, frame),
      close: (code: number, reason: string) => {
        this.finishConnection(connection, { code, reason }, true);
      }
    });
  }

  sendToBusinessKey(
    businessKey: string,
    message: WebSocketLifecycleOutboundMessage
  ): number {
    let sent = 0;
    for (const connection of Array.from(this.connectionsByBusinessKey.get(businessKey) ?? [])) {
      if (this.send(connection, message)) {
        sent += 1;
      }
    }
    return sent;
  }

  close(
    id: string,
    close: WebSocketLifecycleClose = { code: 1000, reason: '' }
  ): boolean {
    const connection = this.connectionsById.get(id);
    if (connection === undefined) {
      return false;
    }
    this.finishConnection(connection, close, true);
    return true;
  }

  connectionCount(): number {
    return this.connectionsById.size;
  }

  admissionHighWaterSize(): number {
    return this.admissionHighWaterByBusinessKey.size;
  }

  observedWriteCount(): number {
    let count = 0;
    for (const connection of this.connectionsById.values()) {
      count += connection.observedWrites.size;
    }
    return count;
  }

  shutdown(
    close: WebSocketLifecycleClose = {
      code: 1001,
      reason: 'websocket gateway shutting down'
    }
  ): Promise<void> {
    if (this.shutdownPromise !== undefined) {
      return this.shutdownPromise;
    }
    this.shuttingDown = true;
    this.shutdownPromise = this.performShutdown(close);
    return this.shutdownPromise;
  }

  private async performShutdown(close: WebSocketLifecycleClose): Promise<void> {
    const connections = Array.from(this.connectionsById.values());
    const sockets = connections.flatMap((connection) =>
      connection.socket === undefined ? [] : [connection.socket]
    );
    const socketsClosed = waitForSocketsOrTimeout(sockets, this.shutdownTimeoutMs);
    for (const connection of connections) {
      this.finishConnection(connection, close, true);
    }
    await socketsClosed;
    for (const socket of sockets) {
      if (socket.readyState !== WebSocket.CLOSED) {
        socket.terminate();
      }
    }
    await Promise.all(Array.from(this.pendingFinalizations));
    if (this.finalizationFailures.length > 0) {
      throw new AggregateError(
        this.finalizationFailures,
        'websocket connection finalization failed'
      );
    }
  }

  private requireConnection(id: string): LifecycleConnection<TConnection, TRuntime> {
    const connection = this.connectionsById.get(id);
    if (connection === undefined) {
      throw new Error(`unknown websocket connection id ${id}`);
    }
    return connection;
  }

  private monotonicNow(): number {
    const value = this.now();
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new Error('websocket lifecycle clock must return a non-negative safe integer');
    }
    return value;
  }

  private pruneAdmissionHighWater(now: number): void {
    for (const [businessKey, highWater] of this.admissionHighWaterByBusinessKey) {
      if (
        // Never evict an unexpired or active fence merely to admit a new key.
        highWater.retainUntilMs < now &&
        (this.connectionsByBusinessKey.get(businessKey)?.size ?? 0) === 0
      ) {
        this.admissionHighWaterByBusinessKey.delete(businessKey);
      }
    }
  }

  private isNewBusinessKeyQuarantined(now: number): boolean {
    return (
      this.newBusinessKeyQuarantineUntilMs !== undefined &&
      now <= this.newBusinessKeyQuarantineUntilMs
    );
  }

  private rejectForAdmissionHighWaterCapacity(
    connection: LifecycleConnection<TConnection, TRuntime>,
    now: number,
    supersededConnections: readonly LifecycleConnection<TConnection, TRuntime>[]
  ): WebSocketPolicyAdmission {
    this.finishSupersededConnections(supersededConnections);
    const nextDeadline = safeRetentionDeadline(
      now,
      this.admissionHighWaterRetentionMs
    );
    this.newBusinessKeyQuarantineUntilMs = Math.max(
      this.newBusinessKeyQuarantineUntilMs ?? 0,
      nextDeadline
    );
    const close = admissionHighWaterCapacityClose();
    this.finishConnection(connection, undefined, false);
    return { accepted: false, close };
  }

  private finishSupersededConnections(
    connections: readonly LifecycleConnection<TConnection, TRuntime>[]
  ): void {
    for (const superseded of connections) {
      this.finishConnection(
        superseded,
        admissionSupersededClose(),
        true,
        superseded.admissionRank === undefined
          ? 'abort-upgrade'
          : 'close-after-upgrade'
      );
    }
  }

  private send(
    connection: LifecycleConnection<TConnection, TRuntime>,
    message: WebSocketLifecycleOutboundMessage
  ): boolean {
    const socket = connection.socket;
    if (connection.state !== 'admitted' || socket?.readyState !== WebSocket.OPEN) {
      return false;
    }
    const messageBytes =
      typeof message.data === 'string'
        ? Buffer.byteLength(message.data, 'utf8')
        : message.data.byteLength;
    if (socket.bufferedAmount + messageBytes > this.slowClientBudgetBytes) {
      this.finishConnection(
        connection,
        { code: 1011, reason: 'websocket client is too slow' },
        true
      );
      return false;
    }
    try {
      socket.send(message.data, { binary: message.binary });
      return true;
    } catch {
      this.finishConnection(
        connection,
        { code: 1011, reason: 'websocket client send failed' },
        true
      );
      return false;
    }
  }

  private writeObservedText(
    connection: LifecycleConnection<TConnection, TRuntime>,
    frame: string
  ): Promise<void> {
    const socket = connection.socket;
    if (
      connection.state !== 'admitted' ||
      socket === undefined ||
      socket.readyState !== WebSocket.OPEN
    ) {
      if (connection.state === 'admitted') {
        this.finishConnection(connection, undefined, false);
      }
      return Promise.reject(
        new Error('websocket connection is not open')
      );
    }
    const messageBytes = Buffer.byteLength(frame, 'utf8');
    if (
      socket.bufferedAmount +
        connection.observedWriteBytes +
        messageBytes >
      this.slowClientBudgetBytes
    ) {
      const error = new Error('websocket client is too slow');
      this.finishConnection(
        connection,
        { code: 1011, reason: error.message },
        true
      );
      return Promise.reject(error);
    }

    return new Promise<void>((resolve, reject) => {
      const write: ObservedWrite = {
        bytes: messageBytes,
        reject,
        resolve
      };
      connection.observedWrites.add(write);
      connection.observedWriteBytes += messageBytes;
      try {
        socket.send(frame, { binary: false }, (error?: Error | null) => {
          if (error == null) {
            if (
              connection.state === 'admitted' &&
              connection.socket === socket &&
              socket.readyState === WebSocket.OPEN
            ) {
              this.settleObservedWrite(connection, write);
              return;
            }
            if (
              this.settleObservedWrite(
                connection,
                write,
                new Error(
                  'websocket connection closed before send completed'
                )
              )
            ) {
              this.finishConnection(connection, undefined, false);
            }
            return;
          }
          if (this.settleObservedWrite(connection, write, error)) {
            this.finishConnection(
              connection,
              { code: 1011, reason: 'websocket client send failed' },
              true
            );
          }
        });
      } catch (error) {
        const sendError = asError(error, 'websocket client send failed');
        if (this.settleObservedWrite(connection, write, sendError)) {
          this.finishConnection(
            connection,
            { code: 1011, reason: 'websocket client send failed' },
            true
          );
        }
      }
    });
  }

  private settleObservedWrite(
    connection: LifecycleConnection<TConnection, TRuntime>,
    write: ObservedWrite,
    error?: Error
  ): boolean {
    if (!connection.observedWrites.delete(write)) {
      return false;
    }
    connection.observedWriteBytes -= write.bytes;
    if (error === undefined) {
      write.resolve();
    } else {
      write.reject(error);
    }
    return true;
  }

  private finishConnection(
    connection: LifecycleConnection<TConnection, TRuntime>,
    close: WebSocketLifecycleClose | undefined,
    closeTransport: boolean,
    preAttachDisposition: WebSocketLifecyclePreAttachCloseDisposition =
      'abort-upgrade'
  ): void {
    if (connection.state === 'closed') {
      return;
    }
    connection.state = 'closed';
    this.connectionsById.delete(connection.id);
    if (connection.businessKey !== undefined) {
      removeFromIndex(this.connectionsByBusinessKey, connection.businessKey, connection);
    }
    if (connection.runtime !== undefined) {
      removeFromIndex(this.connectionsByRuntime, connection.runtime, connection);
    }
    for (const write of Array.from(connection.observedWrites)) {
      this.settleObservedWrite(
        connection,
        write,
        new Error('websocket connection closed before send completed')
      );
    }
    this.trackFinalization(connection);

    if (closeTransport && close !== undefined) {
      if (connection.socket !== undefined) {
        closeSocket(connection.socket, close);
      } else {
        try {
          connection.closeBeforeAttach?.({
            close,
            disposition: preAttachDisposition
          });
        } catch {
          // The lifecycle is already deindexed; a transport cleanup failure cannot restore it.
        }
      }
    }
  }

  private trackFinalization(
    connection: LifecycleConnection<TConnection, TRuntime>
  ): void {
    let result: void | Promise<void>;
    try {
      result = this.onFinish?.(connection.value, connection.runtime);
    } catch (error) {
      this.finalizationFailures.push(error);
      return;
    }
    if (result === undefined) {
      return;
    }
    const completion = Promise.resolve(result)
      .then(
        () => undefined,
        (error: unknown) => {
          this.finalizationFailures.push(error);
        }
      )
      .finally(() => {
        this.pendingFinalizations.delete(completion);
      });
    this.pendingFinalizations.add(completion);
  }
}

export function closePolicyOverflowSocket(
  socket: WebSocket,
  policy: WebSocketConnectionPolicy
): void {
  closeSocket(socket, policyOverflowClose(policy));
}

export function truncateWebSocketCloseReason(
  reason: string,
  maxBytes = MAX_WEBSOCKET_CLOSE_REASON_BYTES
): string {
  const bytes = Buffer.from(reason, 'utf8');
  if (bytes.byteLength <= maxBytes) {
    return bytes.toString('utf8');
  }
  let end = maxBytes;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) {
    end -= 1;
  }
  return bytes.subarray(0, end).toString('utf8');
}

function policyOverflowClose(policy: WebSocketConnectionPolicy): WebSocketLifecycleClose {
  return {
    code: policy.closeCode ?? 1008,
    reason: policy.closeReason ?? 'websocket connection limit exceeded'
  };
}

function admissionSupersededClose(): WebSocketLifecycleClose {
  return {
    code: 4009,
    reason: 'websocket connection superseded by a higher admission rank'
  };
}

function admissionHighWaterCapacityClose(): WebSocketLifecycleClose {
  return {
    code: 1013,
    reason: 'websocket admission high-water capacity exceeded'
  };
}

function safeRetentionDeadline(now: number, retentionMs: number): number {
  const deadline = now + retentionMs;
  if (!Number.isSafeInteger(deadline)) {
    throw new Error('websocket admission high-water retention deadline overflowed');
  }
  return deadline;
}

function closeSocket(socket: WebSocket, close: WebSocketLifecycleClose): void {
  if (socket.readyState === WebSocket.CLOSED) {
    return;
  }
  if (socket.readyState === WebSocket.CONNECTING) {
    try {
      socket.terminate();
    } catch {
      // The connection is already deindexed.
    }
    return;
  }
  if (socket.readyState === WebSocket.OPEN) {
    try {
      socket.close(close.code, truncateWebSocketCloseReason(close.reason));
    } catch {
      try {
        socket.terminate();
      } catch {
        // The connection is already deindexed.
      }
    }
  }
}

function addToIndex<TKey, TConnection>(
  index: Map<TKey, Set<TConnection>>,
  key: TKey,
  connection: TConnection
): void {
  const connections = index.get(key) ?? new Set<TConnection>();
  connections.add(connection);
  index.set(key, connections);
}

function removeFromIndex<TKey, TConnection>(
  index: Map<TKey, Set<TConnection>>,
  key: TKey,
  connection: TConnection
): void {
  const connections = index.get(key);
  if (connections === undefined) {
    return;
  }
  connections.delete(connection);
  if (connections.size === 0) {
    index.delete(key);
  }
}

function positiveInteger(value: number, name: string): number {
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

function nonNegativeInteger(value: number, name: string): number {
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return value;
}

function asError(error: unknown, fallback: string): Error {
  return error instanceof Error ? error : new Error(fallback);
}

function waitForSocketsOrTimeout(sockets: WebSocket[], timeoutMs: number): Promise<void> {
  return new Promise((resolve) => {
    let completed = false;
    let remaining = sockets.filter((socket) => socket.readyState !== WebSocket.CLOSED).length;
    const cleanups: Array<() => void> = [];
    const timer = setTimeout(finish, timeoutMs);

    function finish(): void {
      if (completed) {
        return;
      }
      completed = true;
      clearTimeout(timer);
      for (const cleanup of cleanups) {
        cleanup();
      }
      resolve();
    }

    if (remaining === 0) {
      finish();
      return;
    }
    for (const socket of sockets) {
      if (socket.readyState === WebSocket.CLOSED) {
        continue;
      }
      const onClose = () => {
        remaining -= 1;
        if (remaining === 0) {
          finish();
        }
      };
      socket.once('close', onClose);
      cleanups.push(() => socket.off('close', onClose));
    }
  });
}
