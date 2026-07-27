import type {
  WebSocketRequestBrokerClock,
  WebSocketRequestBrokerOptions
} from './webSocketRequestBrokerTypes.js';

interface Tombstone {
  readonly key: string;
  readonly generationUid: number;
  readonly expiresAt: number;
  readonly sequence: number;
}

export class BrokerTombstoneStore {
  private readonly entries = new Map<string, Tombstone>();
  private queue: Tombstone[] = [];
  private nextSequence = 1;

  constructor(
    private readonly capacity: number,
    private readonly ttlMs: number,
    private readonly now: () => number
  ) {}

  has(key: string): boolean {
    this.sweep();
    return this.entries.has(key);
  }

  add(key: string, generationUid: number): void {
    this.sweep();
    const tombstone: Tombstone = {
      key,
      generationUid,
      expiresAt: this.now() + this.ttlMs,
      sequence: this.nextSequence++
    };
    this.entries.set(key, tombstone);
    this.queue.push(tombstone);
    this.evictToCapacity();
  }

  removeGeneration(generationUid: number): void {
    for (const [key, tombstone] of this.entries) {
      if (tombstone.generationUid === generationUid) {
        this.entries.delete(key);
      }
    }
    this.queue = this.queue.filter(
      (tombstone) => tombstone.generationUid !== generationUid
    );
  }

  get size(): number {
    this.sweep();
    return this.entries.size;
  }

  sweep(): void {
    const now = this.now();
    while (true) {
      const expired = this.queue[0];
      if (expired === undefined || expired.expiresAt > now) {
        return;
      }
      this.queue.shift();
      if (this.entries.get(expired.key) === expired) {
        this.entries.delete(expired.key);
      }
    }
  }

  private evictToCapacity(): void {
    while (this.entries.size > this.capacity) {
      const oldest = this.queue.shift();
      if (oldest === undefined) {
        throw new Error('tombstone FIFO lost its oldest entry');
      }
      if (this.entries.get(oldest.key) === oldest) {
        this.entries.delete(oldest.key);
      }
    }
  }
}

export const SYSTEM_BROKER_CLOCK: WebSocketRequestBrokerClock = {
  now: () => Date.now(),
  setTimeout(callback, delayMs) {
    return setTimeout(callback, delayMs);
  },
  clearTimeout(handle) {
    clearTimeout(handle as ReturnType<typeof setTimeout>);
  }
};

export function validateBrokerOptions(
  options: WebSocketRequestBrokerOptions
): void {
  for (const [name, value] of [
    ['outboundGlobalCapacity', options.outboundGlobalCapacity],
    ['outboundPerGenerationCapacity', options.outboundPerGenerationCapacity],
    ['inboundGlobalCapacity', options.inboundGlobalCapacity],
    ['inboundPerGenerationCapacity', options.inboundPerGenerationCapacity],
    ['outboundTombstoneCapacity', options.outboundTombstoneCapacity],
    ['inboundTombstoneCapacity', options.inboundTombstoneCapacity],
    ['outboundTombstoneTtlMs', options.outboundTombstoneTtlMs],
    ['inboundTombstoneTtlMs', options.inboundTombstoneTtlMs],
    ['inboundTimeoutMs', options.inboundTimeoutMs]
  ] as const) {
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new Error(`${name} must be a positive safe integer`);
    }
  }
}
