import type { ServerResponse } from 'node:http';

import type { HttpResponseFrameMetadata } from '../protocol/envelope.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import { GatewayError } from './errors.js';
import type {
  PendingTerminal,
  PendingTerminalSource,
  RuntimeBinaryDispatchChunk,
  RuntimeBinaryDispatchResponse,
  RuntimeBinaryDispatchStart,
  RuntimeStreamRequestTerminal
} from './runtimeDispatcher.js';

export const DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS = 10_000;

export interface HttpStreamLifecycleCounters {
  activeWriters: number;
  backpressureWaiters: number;
  backpressureCancels: number;
}

export class HttpStreamResponseWriter {
  private closed = false;
  private endReceived = false;
  private queue: Promise<void> = Promise.resolve();
  private requestTerminalCallback: RuntimeStreamRequestTerminal | undefined;
  private terminalRequested = false;

  constructor(
    private readonly input: {
      response: ServerResponse;
      clientDisconnectSignal: AbortSignal;
      backpressureDrainTimeoutMs: number;
      counters: HttpStreamLifecycleCounters;
      writeHeaders(
        response: ServerResponse,
        headers: HttpResponseFrameMetadata['headers']
      ): void;
    }
  ) {
    this.input.counters.activeWriters += 1;
  }

  enqueueStart(
    runtimeResponse: RuntimeBinaryDispatchStart,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', () => {
      if (this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.start received after HTTP response headers were sent'
        );
      }
      const httpResponse = runtimeResponse.header.httpResponse;
      this.input.response.statusCode = httpResponse.status;
      this.input.writeHeaders(this.input.response, httpResponse.headers);
      this.input.response.flushHeaders();
    });
  }

  enqueueChunk(
    runtimeResponse: RuntimeBinaryDispatchChunk,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', async () => {
      if (!this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.chunk received before response.start'
        );
      }
      await this.writeBuffer(Buffer.from(runtimeResponse.payloadBytes));
    });
  }

  enqueueEnd(
    runtimeResponse: RuntimeBinaryDispatchResponse,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', async () => {
      if (!this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.end received before response.start'
        );
      }
      if (runtimeResponse.payloadBytes.byteLength !== 0) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'streaming response.end must not include a payload'
        );
      }
      await this.endResponse();
      this.requestTerminal('runtime_response_end');
    });
  }

  markEndReceived(): void {
    this.endReceived = true;
  }

  requestTerminal(source: PendingTerminalSource, error?: unknown): void {
    if (this.terminalRequested) return;
    if (source === 'runtime_response_end' && !this.endReceived) return;
    this.terminalRequested = true;
    if (source === 'backpressure') {
      this.input.counters.backpressureCancels += 1;
    }
    this.requestTerminalCallback?.(httpStreamPendingTerminal(source, error));
  }

  closeFromPendingTerminal(_terminal: PendingTerminal): void {
    if (this.closed) return;
    this.closed = true;
    this.input.counters.activeWriters = Math.max(
      0,
      this.input.counters.activeWriters - 1
    );
  }

  dispose(): void {
    this.closeFromPendingTerminal({
      source: 'router_shutdown',
      kind: 'cancelled'
    });
  }

  private bindRequestTerminal(requestTerminal: RuntimeStreamRequestTerminal): void {
    this.requestTerminalCallback ??= requestTerminal;
  }

  private enqueue(
    source: PendingTerminalSource,
    write: () => void | Promise<void>
  ): void {
    this.queue = this.queue.then(async () => {
      if (this.closed || this.terminalRequested) return;
      try {
        await write();
      } catch (error) {
        this.requestTerminal(source, error);
      }
    });
    void this.queue.catch((error: unknown) => {
      this.requestTerminal(source, error);
    });
  }

  private async writeBuffer(buffer: Buffer): Promise<void> {
    if (this.closed || this.terminalRequested) return;
    if (
      this.input.response.destroyed ||
      this.input.clientDisconnectSignal.aborted
    ) {
      this.requestTerminal('client_disconnect');
      return;
    }
    if (!this.input.response.write(buffer)) {
      await this.waitForDrain();
    }
  }

  private async waitForDrain(): Promise<void> {
    if (
      this.input.clientDisconnectSignal.aborted ||
      this.input.response.destroyed
    ) {
      this.requestTerminal('client_disconnect');
      return;
    }
    this.input.counters.backpressureWaiters += 1;
    try {
      await new Promise<void>((resolve, reject) => {
        let timeout: NodeJS.Timeout | undefined;
        const cleanup = () => {
          if (timeout !== undefined) clearTimeout(timeout);
          this.input.response.off('drain', onDrain);
          this.input.response.off('error', onError);
          this.input.clientDisconnectSignal.removeEventListener('abort', onAbort);
        };
        const finish = (action: () => void) => {
          cleanup();
          action();
        };
        const onDrain = () => finish(resolve);
        const onError = (error: Error) => finish(() => {
          this.requestTerminal('callback_error', error);
          reject(error);
        });
        const onAbort = () => finish(() => {
          this.requestTerminal('client_disconnect');
          reject(new Error('HTTP client disconnected while waiting for drain'));
        });
        timeout = setTimeout(() => finish(() => {
          this.requestTerminal('backpressure');
          reject(new Error('HTTP response drain timed out'));
        }), this.input.backpressureDrainTimeoutMs);
        this.input.response.once('drain', onDrain);
        this.input.response.once('error', onError);
        this.input.clientDisconnectSignal.addEventListener('abort', onAbort, {
          once: true
        });
      });
    } finally {
      this.input.counters.backpressureWaiters = Math.max(
        0,
        this.input.counters.backpressureWaiters - 1
      );
    }
  }

  private async endResponse(): Promise<void> {
    if (
      this.closed ||
      this.terminalRequested ||
      this.input.response.writableEnded
    ) {
      return;
    }
    if (
      this.input.clientDisconnectSignal.aborted ||
      this.input.response.destroyed
    ) {
      this.requestTerminal('client_disconnect');
      return;
    }
    await new Promise<void>((resolve, reject) => {
      const cleanup = () => {
        this.input.response.off('error', onError);
        this.input.clientDisconnectSignal.removeEventListener('abort', onAbort);
      };
      const onError = (error: Error) => {
        cleanup();
        this.requestTerminal('callback_error', error);
        reject(error);
      };
      const onAbort = () => {
        cleanup();
        this.requestTerminal('client_disconnect');
        reject(new Error('HTTP client disconnected while ending stream'));
      };
      this.input.response.once('error', onError);
      this.input.clientDisconnectSignal.addEventListener('abort', onAbort, {
        once: true
      });
      this.input.response.end(() => {
        cleanup();
        resolve();
      });
    });
  }
}

function httpStreamPendingTerminal(
  source: PendingTerminalSource,
  error: unknown
): PendingTerminal {
  switch (source) {
    case 'runtime_response_end':
      return { source, kind: 'completed' };
    case 'client_disconnect':
    case 'backpressure':
    case 'timeout':
    case 'caller_abort':
    case 'runtime_disconnect':
    case 'router_shutdown':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(
          source === 'client_disconnect'
            ? REQUEST_CANCEL_SITUATION.clientDisconnect
            : source === 'backpressure'
              ? REQUEST_CANCEL_SITUATION.backpressure
              : source === 'timeout'
                ? REQUEST_CANCEL_SITUATION.timeout
                : source === 'caller_abort'
                  ? REQUEST_CANCEL_SITUATION.callerAbort
                  : source === 'runtime_disconnect'
                    ? REQUEST_CANCEL_SITUATION.runtimeDisconnect
                    : REQUEST_CANCEL_SITUATION.routerShutdown
        )
      };
    case 'runtime_response_error':
    case 'runtime_request_cancel':
    case 'protocol_error':
    case 'callback_error':
      return {
        source,
        kind: 'failed',
        error: error ?? new Error(`HTTP stream ${source}`)
      };
  }
}
