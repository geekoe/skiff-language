import { TextDecoder } from "node:util";

export type ActivationJsonInput = string | Uint8Array;

const fatalUtf8Decoder = new TextDecoder("utf-8", {
  fatal: true,
  ignoreBOM: true,
});
const MAX_SAFE_GENERATION_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);

export function parseStrictActivationJson(input: ActivationJsonInput): unknown {
  const source =
    typeof input === "string" ? input : fatalUtf8Decoder.decode(input);
  return new StrictActivationJsonParser(source).parse();
}

class StrictActivationJsonParser {
  private offset = 0;
  private readonly source: string;

  constructor(source: string) {
    this.source = source;
  }

  parse(): unknown {
    this.skipWhitespace();
    const value = this.parseValue();
    this.skipWhitespace();
    if (this.offset !== this.source.length) {
      this.fail("unexpected trailing input");
    }
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
    if (token !== undefined && token >= "0" && token <= "9") {
      return this.parseGenerationNumber();
    }
    this.fail("expected a JSON value");
  }

  private parseObject(): Record<string, unknown> {
    this.offset += 1;
    this.skipWhitespace();
    const result = Object.create(null) as Record<string, unknown>;
    const keys = new Set<string>();
    if (this.consume("}")) return result;
    while (true) {
      if (this.source[this.offset] !== '"') {
        this.fail("object keys must be strings");
      }
      const key = this.parseString();
      if (keys.has(key)) this.fail(`duplicate JSON object key ${key}`);
      keys.add(key);
      this.skipWhitespace();
      this.expect(":");
      this.skipWhitespace();
      result[key] = this.parseValue();
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
      result += this.source.charAt(this.offset);
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
    return String.fromCharCode(first, second);
  }

  private readHexCodeUnit(): number {
    const hex = this.source.slice(this.offset, this.offset + 4);
    if (!/^[0-9a-fA-F]{4}$/.test(hex)) this.fail("invalid Unicode escape");
    this.offset += 4;
    return Number.parseInt(hex, 16);
  }

  private parseGenerationNumber(): number {
    const start = this.offset;
    if (this.source[this.offset] === "0") {
      this.offset += 1;
      const next = this.source[this.offset];
      if (next !== undefined && next >= "0" && next <= "9") {
        this.fail("generation number has a leading zero");
      }
    } else {
      while (true) {
        const digit = this.source[this.offset];
        if (digit === undefined || digit < "0" || digit > "9") break;
        this.offset += 1;
      }
    }
    const next = this.source[this.offset];
    if (next === "." || next === "e" || next === "E") {
      this.fail("generation number must use canonical unsigned integer syntax");
    }
    const lexeme = this.source.slice(start, this.offset);
    const exact = BigInt(lexeme);
    if (exact > MAX_SAFE_GENERATION_BIGINT) {
      this.fail("generation number exceeds Number.MAX_SAFE_INTEGER");
    }
    return Number(exact);
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
    throw new Error(`${message} at JSON offset ${this.offset}`);
  }
}

function isHighSurrogate(value: number): boolean {
  return value >= 0xd800 && value <= 0xdbff;
}

function isLowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}
