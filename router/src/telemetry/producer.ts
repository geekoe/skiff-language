import { randomUUID } from 'node:crypto';

import WebSocket from 'ws';

import {
  TELEMETRY_PROTOCOL,
  type TelemetryBatchEnvelope,
  type TelemetryControlConfig,
  type TelemetryEvent,
  type TelemetryRegisterEnvelope
} from '../protocol/envelope.js';

export interface RouterTelemetryEventSink {
  emit(event: TelemetryEvent): void;
}

export class RouterTelemetryProducer implements RouterTelemetryEventSink {
  private readonly producerId: string;
  private readonly queue: TelemetryEvent[] = [];
  private ws: WebSocket | undefined;
  private flushTimer: NodeJS.Timeout | undefined;
  private seq = 0;
  private registered = false;
  private stopped = false;

  constructor(private readonly config: TelemetryControlConfig) {
    this.producerId = `router-${randomUUID()}`;
  }

  start(): void {
    if (this.stopped || this.ws !== undefined) {
      return;
    }
    this.connect();
    this.flushTimer = setInterval(() => {
      void this.flush();
    }, this.config.flushIntervalMs);
    this.flushTimer.unref();
  }

  emit(event: TelemetryEvent): void {
    if (this.stopped || !this.config.topics.includes(event.topic)) {
      return;
    }
    if (this.queue.length >= this.config.queueMaxEvents) {
      this.queue.shift();
    }
    this.queue.push(event);
    if (this.queue.length >= this.config.batchMaxEvents) {
      void this.flush();
    }
  }

  async flush(): Promise<void> {
    if (!this.registered || this.ws?.readyState !== WebSocket.OPEN || this.queue.length === 0) {
      return;
    }
    const events = this.takeBatch();
    if (events.length === 0) {
      return;
    }
    const batch: TelemetryBatchEnvelope = {
      type: 'telemetry.batch',
      producerId: this.producerId,
      seq: (this.seq += 1),
      events
    };
    try {
      this.ws.send(JSON.stringify(batch));
    } catch {
      this.queue.unshift(...events);
      this.trimQueue();
    }
  }

  async shutdown(): Promise<void> {
    if (this.flushTimer !== undefined) {
      clearInterval(this.flushTimer);
      this.flushTimer = undefined;
    }
    await this.flush();
    this.stopped = true;
    await new Promise<void>((resolve) => {
      const ws = this.ws;
      if (ws === undefined || ws.readyState === WebSocket.CLOSED) {
        resolve();
        return;
      }
      ws.once('close', () => resolve());
      ws.close();
      setTimeout(resolve, 250).unref();
    });
    this.ws = undefined;
  }

  private connect(): void {
    if (this.stopped) {
      return;
    }
    const ws = new WebSocket(this.config.endpoint);
    this.ws = ws;
    this.registered = false;

    ws.on('open', () => {
      const register: TelemetryRegisterEnvelope = {
        type: 'telemetry.register',
        protocol: TELEMETRY_PROTOCOL,
        producerId: this.producerId,
        source: 'router',
        topics: this.config.topics
      };
      try {
        ws.send(JSON.stringify(register));
      } catch {
        ws.close();
      }
    });

    ws.on('message', (data) => {
      const parsed = parseJson(data.toString());
      if (
        isRecord(parsed) &&
        parsed.type === 'telemetry.registered' &&
        parsed.producerId === this.producerId
      ) {
        this.registered = true;
        void this.flush();
      }
    });

    ws.on('close', () => {
      if (this.ws === ws) {
        this.ws = undefined;
        this.registered = false;
      }
      if (!this.stopped) {
        setTimeout(() => this.connect(), this.config.flushIntervalMs).unref();
      }
    });

    ws.on('error', () => {
      ws.close();
    });
  }

  private takeBatch(): TelemetryEvent[] {
    const events: TelemetryEvent[] = [];
    let bytes = 0;
    while (events.length < this.config.batchMaxEvents && this.queue.length > 0) {
      const event = this.queue[0]!;
      const nextBytes = Buffer.byteLength(JSON.stringify(event));
      if (events.length > 0 && bytes + nextBytes > this.config.batchMaxBytes) {
        break;
      }
      bytes += nextBytes;
      events.push(this.queue.shift()!);
    }
    return events;
  }

  private trimQueue(): void {
    while (this.queue.length > this.config.queueMaxEvents) {
      this.queue.shift();
    }
  }
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
