import { TextDecoder } from "node:util";

const fatalUtf8Decoder = new TextDecoder("utf-8", {
  fatal: true,
  ignoreBOM: true,
});
const MAX_SAFE_INTEGER_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);

export function decodeRuntimeAssemblyRequestJson(input: Uint8Array): unknown {
  return new StrictRuntimeAssemblyRequestJsonParser(
    fatalUtf8Decoder.decode(input),
  ).parse();
}

class StrictRuntimeAssemblyRequestJsonParser {
  private offset = 0;
  private readonly source: string;

  constructor(source: string) {
    this.source = source;
  }

  parse(): unknown {
    this.skipWhitespace();
    const value = this.parseValue();
    this.skipWhitespace();
    if (this.offset !== this.source.length) this.fail("unexpected trailing input");
    return value;
  }

  private parseValue(): unknown {
    const token = this.source[this.offset];
    if (token === "{") return this.parseObject();
    if (token === "[") return this.parseArray();
    if (token === '"') return this.parseString();
    if (token === "t") return this.parseKeyword("true", true);
    if (token === "f") return this.parseKeyword("false", false);
    if (token === "n") return this.parseKeyword("null", null);
    if (token === "-" || isDigit(token)) return this.parseNumber();
    this.fail("expected a JSON value");
  }

  private parseObject(): Record<string, unknown> {
    this.offset += 1;
    this.skipWhitespace();
    const result: Record<string, unknown> = {};
    const keys = new Set<string>();
    if (this.consume("}")) return result;
    while (true) {
      if (this.source[this.offset] !== '"') this.fail("object keys must be strings");
      const key = this.parseString();
      if (keys.has(key)) this.fail(`duplicate JSON object key ${key}`);
      keys.add(key);
      this.skipWhitespace();
      this.expect(":");
      this.skipWhitespace();
      Object.defineProperty(result, key, {
        value: this.parseValue(),
        enumerable: true,
        configurable: true,
        writable: true,
      });
      this.skipWhitespace();
      if (this.consume("}")) return result;
      this.expect(",");
      this.skipWhitespace();
    }
  }

  private parseArray(): unknown[] {
    this.offset += 1;
    this.skipWhitespace();
    const result: unknown[] = [];
    if (this.consume("]")) return result;
    while (true) {
      result.push(this.parseValue());
      this.skipWhitespace();
      if (this.consume("]")) return result;
      this.expect(",");
      this.skipWhitespace();
    }
  }

  private parseString(): string {
    this.expect('"');
    let result = "";
    while (this.offset < this.source.length) {
      const codeUnit = this.source.charCodeAt(this.offset);
      if (codeUnit === 0x22) {
        this.offset += 1;
        return result;
      }
      if (codeUnit === 0x5c) {
        result += this.parseEscape();
        continue;
      }
      if (codeUnit <= 0x1f) this.fail("unescaped control character in string");
      if (isHighSurrogate(codeUnit)) {
        const low = this.source.charCodeAt(this.offset + 1);
        if (!isLowSurrogate(low)) this.fail("lone high surrogate in string");
        result += this.source.slice(this.offset, this.offset + 2);
        this.offset += 2;
        continue;
      }
      if (isLowSurrogate(codeUnit)) this.fail("lone low surrogate in string");
      result += this.source[this.offset];
      this.offset += 1;
    }
    this.fail("unterminated string");
  }

  private parseEscape(): string {
    this.offset += 1;
    const escaped = this.source[this.offset];
    this.offset += 1;
    switch (escaped) {
      case '"':
      case "\\":
      case "/":
        return escaped;
      case "b":
        return "\b";
      case "f":
        return "\f";
      case "n":
        return "\n";
      case "r":
        return "\r";
      case "t":
        return "\t";
      case "u":
        return this.parseUnicodeEscape();
      default:
        this.fail("invalid string escape");
    }
  }

  private parseUnicodeEscape(): string {
    const first = this.readHexCodeUnit();
    if (isLowSurrogate(first)) this.fail("lone low surrogate escape");
    if (!isHighSurrogate(first)) return String.fromCharCode(first);
    if (this.source.slice(this.offset, this.offset + 2) !== "\\u") {
      this.fail("lone high surrogate escape");
    }
    this.offset += 2;
    const second = this.readHexCodeUnit();
    if (!isLowSurrogate(second)) this.fail("invalid surrogate pair escape");
    return String.fromCodePoint(
      0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00),
    );
  }

  private readHexCodeUnit(): number {
    const hex = this.source.slice(this.offset, this.offset + 4);
    if (!/^[0-9a-fA-F]{4}$/.test(hex)) this.fail("invalid Unicode escape");
    this.offset += 4;
    return Number.parseInt(hex, 16);
  }

  private parseNumber(): number {
    const match = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(
      this.source.slice(this.offset),
    );
    if (match === null) this.fail("invalid JSON number");
    const lexeme = match[0];
    this.offset += lexeme.length;
    if (/^-?(?:0|[1-9][0-9]*)$/.test(lexeme)) {
      const exact = BigInt(lexeme);
      if (exact > MAX_SAFE_INTEGER_BIGINT || exact < -MAX_SAFE_INTEGER_BIGINT) {
        this.fail("JSON integer exceeds Number.MAX_SAFE_INTEGER");
      }
    }
    const value = Number(lexeme);
    if (!Number.isFinite(value)) this.fail("JSON number must be finite");
    if (Object.is(value, -0)) return value;
    if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
      this.fail("JSON integer exceeds Number.MAX_SAFE_INTEGER");
    }
    return value;
  }

  private parseKeyword<T>(keyword: string, value: T): T {
    if (this.source.slice(this.offset, this.offset + keyword.length) !== keyword) {
      this.fail(`expected ${keyword}`);
    }
    this.offset += keyword.length;
    return value;
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
      `invalid runtimeAssembly request.start JSON: ${message} at offset ${this.offset}`,
    );
  }
}

function isDigit(value: string | undefined): boolean {
  return value !== undefined && value >= "0" && value <= "9";
}

function isHighSurrogate(value: number): boolean {
  return value >= 0xd800 && value <= 0xdbff;
}

function isLowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}
