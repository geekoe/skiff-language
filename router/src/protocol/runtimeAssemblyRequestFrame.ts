import { TextDecoder } from "node:util";

import {
  decodeBinaryFrame,
  type BinaryFrame,
} from "./envelope.js";
import { validateRuntimeAssemblyRequestStartFrameHeader } from "./runtimeProtocol.js";
import type { RuntimeAssemblyRequestStartFrameHeader } from "./runtimeAssemblyRequest.js";

const FIXED_HEADER_BYTES = 14;
const MAGIC = Buffer.from("SKBF", "ascii");
const VERSION = 1;
const JSON_ENCODING = 1;
const fatalUtf8Decoder = new TextDecoder("utf-8", { fatal: true });

export function decodeRuntimeAssemblyRequestStartFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<RuntimeAssemblyRequestStartFrameHeader & Record<string, unknown>> {
  const bytes = rawDataToBuffer(input);
  rejectCanonicalHeaderDuplicates(bytes);
  const frame = decodeBinaryFrame(bytes);
  const result = validateRuntimeAssemblyRequestStartFrameHeader(frame.header);
  if (!result.ok) {
    throw new Error(result.error);
  }
  return {
    header: result.envelope as RuntimeAssemblyRequestStartFrameHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}

function rejectCanonicalHeaderDuplicates(frame: Buffer): void {
  if (
    frame.byteLength < FIXED_HEADER_BYTES ||
    !frame.subarray(0, 4).equals(MAGIC) ||
    frame.readUInt8(4) !== VERSION ||
    frame.readUInt8(5) !== JSON_ENCODING
  ) {
    return;
  }
  const headerLength = frame.readUInt32BE(6);
  const payloadLength = frame.readUInt32BE(10);
  if (
    headerLength === 0 ||
    frame.byteLength !== FIXED_HEADER_BYTES + headerLength + payloadLength
  ) {
    return;
  }
  const source = fatalUtf8Decoder.decode(
    frame.subarray(FIXED_HEADER_BYTES, FIXED_HEADER_BYTES + headerLength),
  );
  new DuplicateRejectingJsonScanner(source).scan();
}

class DuplicateRejectingJsonScanner {
  private offset = 0;
  private readonly source: string;

  constructor(source: string) {
    this.source = source;
  }

  scan(): void {
    this.skipWhitespace();
    this.scanValue();
    this.skipWhitespace();
    if (this.offset !== this.source.length) this.fail("unexpected trailing input");
  }

  private scanValue(): void {
    const token = this.source[this.offset];
    if (token === "{") return this.scanObject();
    if (token === "[") return this.scanArray();
    if (token === '"') {
      this.scanString();
      return;
    }
    if (token === "t") return this.scanKeyword("true");
    if (token === "f") return this.scanKeyword("false");
    if (token === "n") return this.scanKeyword("null");
    if (token === "-" || (token !== undefined && token >= "0" && token <= "9")) {
      return this.scanNumber();
    }
    this.fail("expected a JSON value");
  }

  private scanObject(): void {
    this.offset += 1;
    this.skipWhitespace();
    const keys = new Set<string>();
    if (this.consume("}")) return;
    while (true) {
      if (this.source[this.offset] !== '"') this.fail("object keys must be strings");
      const key = this.scanString();
      if (keys.has(key)) this.fail(`duplicate JSON object key ${key}`);
      keys.add(key);
      this.skipWhitespace();
      this.expect(":");
      this.skipWhitespace();
      this.scanValue();
      this.skipWhitespace();
      if (this.consume("}")) return;
      this.expect(",");
      this.skipWhitespace();
    }
  }

  private scanArray(): void {
    this.offset += 1;
    this.skipWhitespace();
    if (this.consume("]")) return;
    while (true) {
      this.scanValue();
      this.skipWhitespace();
      if (this.consume("]")) return;
      this.expect(",");
      this.skipWhitespace();
    }
  }

  private scanString(): string {
    const start = this.offset;
    this.offset += 1;
    while (this.offset < this.source.length) {
      const code = this.source.charCodeAt(this.offset);
      if (code === 0x22) {
        this.offset += 1;
        const raw = this.source.slice(start, this.offset);
        try {
          return JSON.parse(raw) as string;
        } catch {
          this.fail("invalid JSON string");
        }
      }
      if (code <= 0x1f) this.fail("unescaped control character in string");
      if (code === 0x5c) {
        this.offset += 1;
        const escaped = this.source[this.offset];
        if (escaped === "u") {
          const hex = this.source.slice(this.offset + 1, this.offset + 5);
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) this.fail("invalid Unicode escape");
          this.offset += 5;
          continue;
        }
        if (!['"', "\\", "/", "b", "f", "n", "r", "t"].includes(escaped ?? "")) {
          this.fail("invalid string escape");
        }
      }
      this.offset += 1;
    }
    this.fail("unterminated string");
  }

  private scanNumber(): void {
    const match = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(
      this.source.slice(this.offset),
    );
    if (match === null) this.fail("invalid JSON number");
    this.offset += match[0].length;
  }

  private scanKeyword(keyword: string): void {
    if (this.source.slice(this.offset, this.offset + keyword.length) !== keyword) {
      this.fail(`expected ${keyword}`);
    }
    this.offset += keyword.length;
  }

  private skipWhitespace(): void {
    while (/^[\u0009\u000a\u000d\u0020]$/.test(this.source[this.offset] ?? "")) {
      this.offset += 1;
    }
  }

  private expect(token: string): void {
    if (!this.consume(token)) this.fail(`expected ${token}`);
  }

  private consume(token: string): boolean {
    if (this.source[this.offset] !== token) return false;
    this.offset += 1;
    return true;
  }

  private fail(message: string): never {
    throw new Error(
      `invalid runtimeAssembly request.start frame header: ${message} at JSON offset ${this.offset}`,
    );
  }
}

function rawDataToBuffer(
  data: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): Buffer {
  if (Array.isArray(data)) return Buffer.concat(data);
  if (typeof data === "string") return Buffer.from(data, "utf8");
  if (data instanceof ArrayBuffer) return Buffer.from(new Uint8Array(data));
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}
