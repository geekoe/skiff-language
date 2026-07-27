export interface LosslessJsonLimits {
  readonly maxJsonDepth: number;
  readonly maxJsonNodes: number;
  readonly maxStringBytes: number;
}

export type LosslessJsonNode =
  | LosslessJsonObjectNode
  | LosslessJsonArrayNode
  | LosslessJsonStringNode
  | LosslessJsonNumberNode
  | LosslessJsonLiteralNode;

interface LosslessJsonNodeBase {
  readonly start: number;
  readonly end: number;
}

export interface LosslessJsonObjectNode extends LosslessJsonNodeBase {
  readonly kind: 'object';
  readonly members: readonly LosslessJsonMember[];
}

export interface LosslessJsonMember {
  readonly key: string;
  readonly value: LosslessJsonNode;
}

export interface LosslessJsonArrayNode extends LosslessJsonNodeBase {
  readonly kind: 'array';
  readonly items: readonly LosslessJsonNode[];
}

export interface LosslessJsonStringNode extends LosslessJsonNodeBase {
  readonly kind: 'string';
  readonly value: string;
}

export interface LosslessJsonNumberNode extends LosslessJsonNodeBase {
  readonly kind: 'number';
  readonly lexeme: string;
}

export interface LosslessJsonLiteralNode extends LosslessJsonNodeBase {
  readonly kind: 'boolean' | 'null';
  readonly value: boolean | null;
}

export class LosslessJsonLimitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'LosslessJsonLimitError';
  }
}

export class LosslessJsonSyntaxError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'LosslessJsonSyntaxError';
  }
}

export function parseLosslessJson(
  source: string,
  limits: LosslessJsonLimits
): LosslessJsonNode {
  return new LosslessJsonParser(source, limits).parse();
}

export function losslessJsonSlice(
  source: string,
  node: LosslessJsonNode
): string {
  return source.slice(node.start, node.end);
}

export function uniqueObjectMembers(
  node: LosslessJsonObjectNode
): ReadonlyMap<string, LosslessJsonNode> | undefined {
  const members = new Map<string, LosslessJsonNode>();
  for (const member of node.members) {
    if (members.has(member.key)) {
      return undefined;
    }
    members.set(member.key, member.value);
  }
  return members;
}

class LosslessJsonParser {
  private nodeCount = 0;
  private offset = 0;

  constructor(
    private readonly source: string,
    private readonly limits: LosslessJsonLimits
  ) {
    if (
      !Number.isSafeInteger(limits.maxJsonDepth) ||
      limits.maxJsonDepth <= 0 ||
      !Number.isSafeInteger(limits.maxJsonNodes) ||
      limits.maxJsonNodes <= 0 ||
      !Number.isSafeInteger(limits.maxStringBytes) ||
      limits.maxStringBytes <= 0
    ) {
      throw new Error('lossless JSON limits must be positive safe integers');
    }
  }

  parse(): LosslessJsonNode {
    this.skipWhitespace();
    const value = this.parseValue(1);
    this.skipWhitespace();
    if (this.offset !== this.source.length) {
      this.fail('unexpected trailing input');
    }
    return value;
  }

  private parseValue(depth: number): LosslessJsonNode {
    if (depth > this.limits.maxJsonDepth) {
      this.limit('JSON depth exceeds the profile limit');
    }
    this.nodeCount += 1;
    if (this.nodeCount > this.limits.maxJsonNodes) {
      this.limit('JSON node count exceeds the profile limit');
    }

    const token = this.source[this.offset];
    if (token === '{') {
      return this.parseObject(depth);
    }
    if (token === '[') {
      return this.parseArray(depth);
    }
    if (token === '"') {
      return this.parseStringNode();
    }
    if (token === 't') {
      return this.parseLiteral('true', 'boolean', true);
    }
    if (token === 'f') {
      return this.parseLiteral('false', 'boolean', false);
    }
    if (token === 'n') {
      return this.parseLiteral('null', 'null', null);
    }
    if (token === '-' || isDigit(token)) {
      return this.parseNumber();
    }
    this.fail('expected a JSON value');
  }

  private parseObject(depth: number): LosslessJsonObjectNode {
    const start = this.offset;
    this.offset += 1;
    this.skipWhitespace();
    const members: LosslessJsonMember[] = [];
    if (this.consume('}')) {
      return { kind: 'object', start, end: this.offset, members };
    }
    while (true) {
      if (this.source[this.offset] !== '"') {
        this.fail('object keys must be strings');
      }
      const key = this.parseString();
      this.skipWhitespace();
      this.expect(':');
      this.skipWhitespace();
      const value = this.parseValue(depth + 1);
      members.push({ key, value });
      this.skipWhitespace();
      if (this.consume('}')) {
        return { kind: 'object', start, end: this.offset, members };
      }
      this.expect(',');
      this.skipWhitespace();
    }
  }

  private parseArray(depth: number): LosslessJsonArrayNode {
    const start = this.offset;
    this.offset += 1;
    this.skipWhitespace();
    const items: LosslessJsonNode[] = [];
    if (this.consume(']')) {
      return { kind: 'array', start, end: this.offset, items };
    }
    while (true) {
      items.push(this.parseValue(depth + 1));
      this.skipWhitespace();
      if (this.consume(']')) {
        return { kind: 'array', start, end: this.offset, items };
      }
      this.expect(',');
      this.skipWhitespace();
    }
  }

  private parseStringNode(): LosslessJsonStringNode {
    const start = this.offset;
    const value = this.parseString();
    return { kind: 'string', start, end: this.offset, value };
  }

  private parseString(): string {
    const start = this.offset;
    this.expect('"');
    while (this.offset < this.source.length) {
      const codeUnit = this.source.charCodeAt(this.offset);
      if (codeUnit === 0x22) {
        this.offset += 1;
        const raw = this.source.slice(start, this.offset);
        let value: string;
        try {
          value = JSON.parse(raw) as string;
        } catch {
          this.fail('invalid JSON string');
        }
        if (Buffer.byteLength(value, 'utf8') > this.limits.maxStringBytes) {
          this.limit('JSON string exceeds the profile limit');
        }
        return value;
      }
      if (codeUnit === 0x5c) {
        this.offset += 1;
        const escaped = this.source[this.offset];
        if (
          escaped === '"' ||
          escaped === '\\' ||
          escaped === '/' ||
          escaped === 'b' ||
          escaped === 'f' ||
          escaped === 'n' ||
          escaped === 'r' ||
          escaped === 't'
        ) {
          this.offset += 1;
          continue;
        }
        if (escaped !== 'u') {
          this.fail('invalid string escape');
        }
        this.offset += 1;
        const hex = this.source.slice(this.offset, this.offset + 4);
        if (!/^[0-9a-fA-F]{4}$/.test(hex)) {
          this.fail('invalid Unicode escape');
        }
        this.offset += 4;
        continue;
      }
      if (codeUnit <= 0x1f) {
        this.fail('unescaped control character in string');
      }
      this.offset += 1;
    }
    this.fail('unterminated string');
  }

  private parseNumber(): LosslessJsonNumberNode {
    const start = this.offset;
    const match =
      /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(
        this.source.slice(this.offset)
      );
    if (match === null) {
      this.fail('invalid JSON number');
    }
    const lexeme = match[0];
    this.offset += lexeme.length;
    return { kind: 'number', start, end: this.offset, lexeme };
  }

  private parseLiteral(
    spelling: string,
    kind: 'boolean' | 'null',
    value: boolean | null
  ): LosslessJsonLiteralNode {
    const start = this.offset;
    if (this.source.slice(start, start + spelling.length) !== spelling) {
      this.fail(`expected ${spelling}`);
    }
    this.offset += spelling.length;
    return { kind, value, start, end: this.offset };
  }

  private skipWhitespace(): void {
    while (
      this.source[this.offset] === ' ' ||
      this.source[this.offset] === '\t' ||
      this.source[this.offset] === '\n' ||
      this.source[this.offset] === '\r'
    ) {
      this.offset += 1;
    }
  }

  private expect(token: string): void {
    if (!this.consume(token)) {
      this.fail(`expected ${token}`);
    }
  }

  private consume(token: string): boolean {
    if (this.source[this.offset] !== token) {
      return false;
    }
    this.offset += 1;
    return true;
  }

  private fail(message: string): never {
    throw new LosslessJsonSyntaxError(
      `${message} at JSON offset ${this.offset}`
    );
  }

  private limit(message: string): never {
    throw new LosslessJsonLimitError(message);
  }
}

function isDigit(value: string | undefined): boolean {
  return value !== undefined && value >= '0' && value <= '9';
}
