import WebSocket from 'ws';

const DEFAULT_CONNECTION_LIMIT = 5_000;
const DEFAULT_RECEIVE_QUEUE_LIMIT = 100;
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

export interface WebSocketConnectionLifecycleOptions {
  connectionLimit?: number;
  receiveQueueLimit?: number;
  slowClientBudgetBytes?: number;
  shutdownTimeoutMs?: number;
}

export interface WebSocketReceiveLifecycleCounters {
  inFlight: number;
  queued: number;
  abortOnClose: number;
}

export type WebSocketReceiveScheduleResult = 'started' | 'queued' | 'closed' | 'ignored';

export type WebSocketPolicyAdmission =
  | { accepted: true }
  | { accepted: false; close: WebSocketLifecycleClose };

interface PendingReceive {
  run(signal: AbortSignal): Promise<void>;
  onError(error: unknown): void;
}

interface ActiveReceive extends PendingReceive {
  controller: AbortController;
  abortCounted: boolean;
}

type LifecycleState = 'reserved' | 'admitted' | 'closed';

interface LifecycleConnection<TConnection, TRuntime> {
  activeReceive?: ActiveReceive;
  businessKey?: string;
  closeBeforeAttach?: (close: WebSocketLifecycleClose) => void;
  id: string;
  receiveQueue: PendingReceive[];
  runtime?: TRuntime;
  socket?: WebSocket;
  state: LifecycleState;
  value: TConnection;
}

export class WebSocketConnectionLimitExceededError extends Error {
  constructor() {
    super('websocket gateway connection limit exceeded');
  }
}

export class WebSocketConnectionLifecycle<TConnection, TRuntime = unknown> {
  private readonly connectionLimit: number;
  private readonly receiveQueueLimit: number;
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
  private readonly receiveCountersValue: WebSocketReceiveLifecycleCounters = {
    inFlight: 0,
    queued: 0,
    abortOnClose: 0
  };
  private shutdownPromise: Promise<void> | undefined;
  private shuttingDown = false;

  constructor(options: WebSocketConnectionLifecycleOptions = {}) {
    this.connectionLimit = positiveInteger(
      options.connectionLimit ?? DEFAULT_CONNECTION_LIMIT,
      'connectionLimit'
    );
    this.receiveQueueLimit = nonNegativeInteger(
      options.receiveQueueLimit ?? DEFAULT_RECEIVE_QUEUE_LIMIT,
      'receiveQueueLimit'
    );
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
    closeBeforeAttach?: (close: WebSocketLifecycleClose) => void
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
      receiveQueue: [],
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
    input: { businessKey?: string; policy?: WebSocketConnectionPolicy }
  ): WebSocketPolicyAdmission {
    const connection = this.requireConnection(id);
    if (connection.state !== 'reserved') {
      throw new Error(`websocket connection ${id} is already ${connection.state}`);
    }
    if (input.policy !== undefined && input.businessKey === undefined) {
      throw new Error('websocket connection policy requires a business key');
    }

    const existing =
      input.businessKey === undefined
        ? []
        : Array.from(this.connectionsByBusinessKey.get(input.businessKey) ?? []);
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

  scheduleReceive(
    id: string,
    receive: {
      run(signal: AbortSignal): Promise<void>;
      onError(error: unknown): void;
    }
  ): WebSocketReceiveScheduleResult {
    const connection = this.connectionsById.get(id);
    if (connection === undefined || connection.state !== 'admitted') {
      return 'ignored';
    }
    if (connection.activeReceive === undefined) {
      this.startReceive(connection, receive);
      return 'started';
    }
    if (connection.receiveQueue.length >= this.receiveQueueLimit) {
      this.finishConnection(
        connection,
        { code: 1008, reason: 'websocket receive queue is full' },
        true
      );
      return 'closed';
    }
    connection.receiveQueue.push(receive);
    this.receiveCountersValue.queued += 1;
    return 'queued';
  }

  sendToConnection(id: string, message: WebSocketLifecycleOutboundMessage): boolean {
    const connection = this.connectionsById.get(id);
    return connection === undefined ? false : this.send(connection, message);
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

  receiveCounters(): WebSocketReceiveLifecycleCounters {
    return { ...this.receiveCountersValue };
  }

  connectionCount(): number {
    return this.connectionsById.size;
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
  }

  private requireConnection(id: string): LifecycleConnection<TConnection, TRuntime> {
    const connection = this.connectionsById.get(id);
    if (connection === undefined) {
      throw new Error(`unknown websocket connection id ${id}`);
    }
    return connection;
  }

  private startReceive(
    connection: LifecycleConnection<TConnection, TRuntime>,
    receive: PendingReceive
  ): void {
    const active: ActiveReceive = {
      ...receive,
      controller: new AbortController(),
      abortCounted: false
    };
    connection.activeReceive = active;
    this.receiveCountersValue.inFlight += 1;

    let receivePromise: Promise<void>;
    try {
      receivePromise = active.run(active.controller.signal);
    } catch (error) {
      receivePromise = Promise.reject(error);
    }
    void receivePromise
      .catch((error: unknown) => {
        if (connection.state === 'admitted') {
          active.onError(error);
        }
      })
      .finally(() => {
        if (connection.activeReceive !== active) {
          return;
        }
        delete connection.activeReceive;
        this.receiveCountersValue.inFlight = Math.max(
          0,
          this.receiveCountersValue.inFlight - 1
        );
        if (active.abortCounted) {
          this.receiveCountersValue.abortOnClose = Math.max(
            0,
            this.receiveCountersValue.abortOnClose - 1
          );
        }
        this.drainReceiveQueue(connection);
      })
      .catch(() => undefined);
  }

  private drainReceiveQueue(connection: LifecycleConnection<TConnection, TRuntime>): void {
    if (connection.state !== 'admitted' || connection.activeReceive !== undefined) {
      return;
    }
    const receive = connection.receiveQueue.shift();
    if (receive === undefined) {
      return;
    }
    this.receiveCountersValue.queued = Math.max(0, this.receiveCountersValue.queued - 1);
    this.startReceive(connection, receive);
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

  private finishConnection(
    connection: LifecycleConnection<TConnection, TRuntime>,
    close: WebSocketLifecycleClose | undefined,
    closeTransport: boolean
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

    if (connection.receiveQueue.length > 0) {
      this.receiveCountersValue.queued = Math.max(
        0,
        this.receiveCountersValue.queued - connection.receiveQueue.length
      );
      connection.receiveQueue = [];
    }
    const active = connection.activeReceive;
    if (active !== undefined && !active.controller.signal.aborted) {
      active.abortCounted = true;
      this.receiveCountersValue.abortOnClose += 1;
      active.controller.abort();
    }

    if (closeTransport && close !== undefined) {
      if (connection.socket !== undefined) {
        closeSocket(connection.socket, close);
      } else {
        try {
          connection.closeBeforeAttach?.(close);
        } catch {
          // The lifecycle is already deindexed; a transport cleanup failure cannot restore it.
        }
      }
    }
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
