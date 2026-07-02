import type { JsonSchema, OperationParameterManifest } from '../../src/manifest/types.js';
import { isRecord } from '../../src/protocol/envelope.js';

const MAGIC = Buffer.from('SKPV', 'ascii');
const VERSION = 2;

const TAG_NULL = 0;
const TAG_BOOL_FALSE = 1;
const TAG_BOOL_TRUE = 2;
const TAG_NUMBER = 3;
const TAG_STRING = 4;
const TAG_BYTES = 5;
const TAG_ARRAY = 6;
const TAG_OBJECT = 7;
const TAG_MAP = 8;
const TAG_DATE = 10;

export class RuntimePayloadCodecError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RuntimePayloadCodecError';
  }
}

export function operationArgsSchema(
  parameters: readonly OperationParameterManifest[]
): JsonSchema {
  const properties: Record<string, JsonSchema> = {};
  for (const parameter of parameters) {
    properties[parameter.name] = parameter.schema;
  }
  return {
    type: 'object',
    properties,
    required: parameters.map((parameter) => parameter.name),
    additionalProperties: false
  };
}

export function encodeOperationPayload(
  args: Record<string, unknown>,
  parameters: readonly OperationParameterManifest[]
): Buffer {
  return encodeRuntimePayload(args, operationArgsSchema(parameters));
}

export function decodeOperationPayload(
  payloadBytes: Uint8Array,
  parameters: readonly OperationParameterManifest[]
): Record<string, unknown> {
  const decoded = decodeRuntimePayload(payloadBytes, operationArgsSchema(parameters));
  if (!isRecord(decoded)) {
    throw new RuntimePayloadCodecError('operation payload must decode to an args object');
  }
  return decoded;
}

export function encodeRuntimePayload(value: unknown, schema: JsonSchema): Buffer {
  const writer = new PayloadWriter();
  writer.writeBytes(MAGIC);
  writer.writeU8(VERSION);
  encodeTyped(writer, value, schema, 'payload');
  return writer.toBuffer();
}

export function decodeRuntimePayload(payloadBytes: Uint8Array, schema: JsonSchema): unknown {
  const reader = new PayloadReader(payloadBytes);
  if (!reader.readBytes(MAGIC.byteLength).equals(MAGIC)) {
    throw new RuntimePayloadCodecError('runtime payload bytes missing SKPV magic');
  }
  const version = reader.readU8();
  if (version !== VERSION) {
    throw new RuntimePayloadCodecError(`unsupported runtime payload version ${version}`);
  }
  const value = decodeTyped(reader, schema, 'payload');
  if (!reader.done()) {
    throw new RuntimePayloadCodecError(
      `runtime payload has ${reader.remaining()} trailing byte(s)`
    );
  }
  return value;
}

function encodeTyped(
  writer: PayloadWriter,
  value: unknown,
  schema: JsonSchema,
  path: string
): void {
  if (schema.nullable) {
    if (value === null) {
      writer.writeU8(0);
      return;
    }
    writer.writeU8(1);
    encodeTyped(writer, value, withoutNullable(schema), path);
    return;
  }

  if ('oneOf' in schema) {
    if (schema.oneOf.length > 256) {
      throw new RuntimePayloadCodecError(
        `runtime payload union has ${schema.oneOf.length} branches; maximum is 256`
      );
    }
    const errors: string[] = [];
    for (const [index, branch] of schema.oneOf.entries()) {
      const branchWriter = new PayloadWriter();
      try {
        encodeTyped(branchWriter, value, branch, path);
        writer.writeU8(index);
        writer.writeBytes(branchWriter.toBuffer());
        return;
      } catch (error) {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    }
    throw new RuntimePayloadCodecError(
      `runtime payload union value did not match any branch: ${errors.join('; ')}`
    );
  }

  const literal = literalString(schema);
  if (literal !== undefined) {
    if (value !== literal) {
      throw new RuntimePayloadCodecError(`expected runtime literal string ${literal}`);
    }
    writeString(writer, literal);
    return;
  }

  const enumLiterals = stringEnumLiterals(schema);
  if (enumLiterals !== undefined && enumLiterals.length > 1) {
    encodeStringEnum(writer, value, enumLiterals, path);
    return;
  }

  if (isBytesSchema(schema)) {
    writer.writeU8(TAG_BYTES);
    writer.writeBuffer(bytesFromValue(value, path));
    return;
  }

  if (isDateSchema(schema)) {
    writeDate(writer, value, path);
    return;
  }

  switch (schema.type) {
    case 'any':
    case 'json':
      encodeAny(writer, value, path);
      return;
    case 'null':
      if (value !== null) {
        throw new RuntimePayloadCodecError(`expected null at ${path}`);
      }
      writer.writeU8(TAG_NULL);
      return;
    case 'string':
      if (typeof value !== 'string') {
        throw new RuntimePayloadCodecError(`expected string at ${path}`);
      }
      writeString(writer, value);
      return;
    case 'boolean':
      if (typeof value !== 'boolean') {
        throw new RuntimePayloadCodecError(`expected boolean at ${path}`);
      }
      writer.writeU8(value ? TAG_BOOL_TRUE : TAG_BOOL_FALSE);
      return;
    case 'number':
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        throw new RuntimePayloadCodecError(`expected number at ${path}`);
      }
      writeNumber(writer, value);
      return;
    case 'integer':
      if (typeof value !== 'number' || !Number.isInteger(value)) {
        throw new RuntimePayloadCodecError(`expected integer at ${path}`);
      }
      writeNumber(writer, value);
      return;
    case 'array':
      if (!Array.isArray(value)) {
        throw new RuntimePayloadCodecError(`expected array at ${path}`);
      }
      writer.writeU8(TAG_ARRAY);
      writer.writeLen(value.length);
      for (const [index, item] of value.entries()) {
        encodeTyped(writer, item, schema.items, `${path}[${index}]`);
      }
      return;
    case 'object':
      encodeObject(writer, value, schema, path);
      return;
    default:
      throw new RuntimePayloadCodecError(`unsupported runtime payload schema at ${path}`);
  }
}

function decodeTyped(reader: PayloadReader, schema: JsonSchema, path: string): unknown {
  if (schema.nullable) {
    const discriminant = reader.readU8();
    if (discriminant === 0) {
      return null;
    }
    if (discriminant !== 1) {
      throw new RuntimePayloadCodecError(
        `runtime payload nullable discriminant must be 0 or 1, got ${discriminant}`
      );
    }
    return decodeTyped(reader, withoutNullable(schema), path);
  }

  if ('oneOf' in schema) {
    const branch = reader.readU8();
    const branchSchema = schema.oneOf[branch];
    if (branchSchema === undefined) {
      throw new RuntimePayloadCodecError(
        `runtime payload union branch ${branch} is out of range`
      );
    }
    return decodeTyped(reader, branchSchema, path);
  }

  const literal = literalString(schema);
  if (literal !== undefined) {
    const value = decodeString(reader, path);
    if (value !== literal) {
      throw new RuntimePayloadCodecError(`expected runtime literal string ${literal}`);
    }
    return value;
  }

  const enumLiterals = stringEnumLiterals(schema);
  if (enumLiterals !== undefined && enumLiterals.length > 1) {
    return decodeStringEnum(reader, enumLiterals, path);
  }

  if (isBytesSchema(schema)) {
    reader.expectTag(TAG_BYTES, path);
    return reader.readBuffer();
  }

  if (isDateSchema(schema)) {
    return decodeDate(reader, path);
  }

  switch (schema.type) {
    case 'any':
    case 'json':
      return decodeAny(reader, path);
    case 'null':
      reader.expectTag(TAG_NULL, path);
      return null;
    case 'string':
      return decodeString(reader, path);
    case 'boolean':
      switch (reader.readU8()) {
        case TAG_BOOL_FALSE:
          return false;
        case TAG_BOOL_TRUE:
          return true;
        default:
          throw new RuntimePayloadCodecError(`expected runtime bool tag at ${path}`);
      }
    case 'number':
      return decodeNumber(reader, path);
    case 'integer': {
      const number = decodeNumber(reader, path);
      if (!Number.isInteger(number)) {
        throw new RuntimePayloadCodecError(`expected runtime integer at ${path}`);
      }
      return number;
    }
    case 'array': {
      reader.expectTag(TAG_ARRAY, path);
      const len = reader.readLen();
      const items: unknown[] = [];
      for (let index = 0; index < len; index += 1) {
        items.push(decodeTyped(reader, schema.items, `${path}[${index}]`));
      }
      return items;
    }
    case 'object':
      return decodeObject(reader, schema, path);
    default:
      throw new RuntimePayloadCodecError(`unsupported runtime payload schema at ${path}`);
  }
}

function encodeObject(
  writer: PayloadWriter,
  value: unknown,
  schema: Extract<JsonSchema, { type: 'object' }>,
  path: string
): void {
  if (!isRecord(value)) {
    throw new RuntimePayloadCodecError(`expected object at ${path}`);
  }
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const known = new Set(Object.keys(properties));
  for (const name of required) {
    if (!(name in value) || value[name] === undefined) {
      throw new RuntimePayloadCodecError(`missing required field ${path}.${name}`);
    }
  }
  if (schema.additionalProperties === false) {
    for (const name of Object.keys(value)) {
      if (!known.has(name)) {
        throw new RuntimePayloadCodecError(`unexpected field ${path}.${name}`);
      }
    }
  }

  const presentFields = Object.keys(properties)
    .filter((name) => value[name] !== undefined)
    .sort();
  writer.writeU8(TAG_OBJECT);
  writer.writeLen(presentFields.length);
  for (const name of presentFields) {
    const fieldSchema = properties[name];
    if (fieldSchema === undefined) {
      throw new RuntimePayloadCodecError(`missing schema for ${path}.${name}`);
    }
    writer.writeStringRaw(name);
    encodeTyped(writer, value[name], fieldSchema, `${path}.${name}`);
  }
}

function decodeObject(
  reader: PayloadReader,
  schema: Extract<JsonSchema, { type: 'object' }>,
  path: string
): Record<string, unknown> {
  reader.expectTag(TAG_OBJECT, path);
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const len = reader.readLen();
  const output: Record<string, unknown> = {};
  for (let index = 0; index < len; index += 1) {
    const name = reader.readStringRaw();
    const fieldSchema = properties[name];
    if (fieldSchema === undefined) {
      throw new RuntimePayloadCodecError(`unexpected field ${path}.${name}`);
    }
    output[name] = decodeTyped(reader, fieldSchema, `${path}.${name}`);
  }
  for (const name of required) {
    if (!(name in output)) {
      throw new RuntimePayloadCodecError(`missing required field ${path}.${name}`);
    }
  }
  return output;
}

function encodeAny(writer: PayloadWriter, value: unknown, path: string): void {
  if (value === null || value === undefined) {
    writer.writeU8(TAG_NULL);
    return;
  }
  if (typeof value === 'boolean') {
    writer.writeU8(value ? TAG_BOOL_TRUE : TAG_BOOL_FALSE);
    return;
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new RuntimePayloadCodecError(`cannot encode non-finite number at ${path}`);
    }
    writeNumber(writer, value);
    return;
  }
  if (typeof value === 'string') {
    writeString(writer, value);
    return;
  }
  if (value instanceof Uint8Array) {
    writer.writeU8(TAG_BYTES);
    writer.writeBuffer(value);
    return;
  }
  if (value instanceof Date) {
    writeDate(writer, value, path);
    return;
  }
  if (Array.isArray(value)) {
    writer.writeU8(TAG_ARRAY);
    writer.writeLen(value.length);
    for (const [index, item] of value.entries()) {
      encodeAny(writer, item, `${path}[${index}]`);
    }
    return;
  }
  if (isRecord(value)) {
    writer.writeU8(TAG_OBJECT);
    const keys = Object.keys(value)
      .filter((name) => value[name] !== undefined)
      .sort();
    writer.writeLen(keys.length);
    for (const name of keys) {
      writer.writeStringRaw(name);
      encodeAny(writer, value[name], `${path}.${name}`);
    }
    return;
  }
  throw new RuntimePayloadCodecError(`unsupported value at ${path}`);
}

function decodeAny(reader: PayloadReader, path: string): unknown {
  const tag = reader.readU8();
  switch (tag) {
    case TAG_NULL:
      return null;
    case TAG_BOOL_FALSE:
      return false;
    case TAG_BOOL_TRUE:
      return true;
    case TAG_NUMBER:
      return reader.readF64();
    case TAG_STRING:
      return reader.readStringRaw();
    case TAG_DATE:
      return readDate(reader, path);
    case TAG_BYTES:
      return reader.readBuffer();
    case TAG_ARRAY: {
      const len = reader.readLen();
      const items: unknown[] = [];
      for (let index = 0; index < len; index += 1) {
        items.push(decodeAny(reader, `${path}[${index}]`));
      }
      return items;
    }
    case TAG_OBJECT: {
      const len = reader.readLen();
      const output: Record<string, unknown> = {};
      for (let index = 0; index < len; index += 1) {
        const name = reader.readStringRaw();
        output[name] = decodeAny(reader, `${path}.${name}`);
      }
      return output;
    }
    case TAG_MAP: {
      const len = reader.readLen();
      const output: Record<string, unknown> = {};
      for (let index = 0; index < len; index += 1) {
        const keyTag = reader.readU8();
        if (keyTag !== 0) {
          throw new RuntimePayloadCodecError('unsupported runtime payload map key type');
        }
        output[reader.readStringRaw()] = decodeAny(reader, path);
      }
      return output;
    }
    default:
      throw new RuntimePayloadCodecError(`unknown runtime payload tag ${tag}`);
  }
}

function writeString(writer: PayloadWriter, value: string): void {
  writer.writeU8(TAG_STRING);
  writer.writeStringRaw(value);
}

function decodeString(reader: PayloadReader, path: string): string {
  reader.expectTag(TAG_STRING, path);
  try {
    return reader.readStringRaw();
  } catch (error) {
    throw new RuntimePayloadCodecError(
      `runtime payload string at ${path} is not UTF-8: ${
        error instanceof Error ? error.message : String(error)
      }`
    );
  }
}

function writeNumber(writer: PayloadWriter, value: number): void {
  writer.writeU8(TAG_NUMBER);
  writer.writeF64(value);
}

function decodeNumber(reader: PayloadReader, path: string): number {
  reader.expectTag(TAG_NUMBER, path);
  const value = reader.readF64();
  if (!Number.isFinite(value)) {
    throw new RuntimePayloadCodecError(`runtime payload number at ${path} must be finite`);
  }
  return value;
}

function writeDate(writer: PayloadWriter, value: unknown, path: string): void {
  const epochMillis = dateEpochMillis(value, path);
  writer.writeU8(TAG_DATE);
  writer.writeI64(epochMillis);
}

function decodeDate(reader: PayloadReader, path: string): Date {
  reader.expectTag(TAG_DATE, path);
  return readDate(reader, path);
}

function readDate(reader: PayloadReader, path: string): Date {
  const epochMillis = reader.readI64();
  if (!Number.isSafeInteger(epochMillis)) {
    throw new RuntimePayloadCodecError(`runtime payload Date at ${path} is not a safe integer`);
  }
  const value = new Date(epochMillis);
  if (!Number.isFinite(value.getTime())) {
    throw new RuntimePayloadCodecError(`runtime payload Date at ${path} is out of range`);
  }
  return value;
}

function dateEpochMillis(value: unknown, path: string): number {
  const epochMillis =
    value instanceof Date
      ? value.getTime()
      : typeof value === 'string'
        ? Date.parse(value)
        : typeof value === 'number'
          ? value
          : Number.NaN;
  if (!Number.isSafeInteger(epochMillis)) {
    throw new RuntimePayloadCodecError(`expected Date at ${path}`);
  }
  const date = new Date(epochMillis);
  if (!Number.isFinite(date.getTime())) {
    throw new RuntimePayloadCodecError(`Date at ${path} is out of range`);
  }
  return epochMillis;
}

function literalString(schema: JsonSchema): string | undefined {
  const literals = stringEnumLiterals(schema);
  return literals?.length === 1 ? literals[0] : undefined;
}

function stringEnumLiterals(schema: JsonSchema): readonly string[] | undefined {
  if ('oneOf' in schema || schema.type !== 'string' || schema.enum === undefined) {
    return undefined;
  }
  return schema.enum.every((value): value is string => typeof value === 'string')
    ? schema.enum
    : undefined;
}

function encodeStringEnum(
  writer: PayloadWriter,
  value: unknown,
  literals: readonly string[],
  path: string
): void {
  if (literals.length > 256) {
    throw new RuntimePayloadCodecError(
      `runtime payload union has ${literals.length} branches; maximum is 256`
    );
  }
  if (typeof value !== 'string') {
    throw new RuntimePayloadCodecError(`expected string at ${path}`);
  }
  const branch = literals.indexOf(value);
  if (branch === -1) {
    throw new RuntimePayloadCodecError(`expected runtime enum string at ${path}`);
  }
  writer.writeU8(branch);
  writeString(writer, value);
}

function decodeStringEnum(
  reader: PayloadReader,
  literals: readonly string[],
  path: string
): string {
  if (literals.length > 256) {
    throw new RuntimePayloadCodecError(
      `runtime payload union has ${literals.length} branches; maximum is 256`
    );
  }
  const branch = reader.readU8();
  const literal = literals[branch];
  if (literal === undefined) {
    throw new RuntimePayloadCodecError(
      `runtime payload union branch ${branch} is out of range at ${path}`
    );
  }
  const value = decodeString(reader, path);
  if (value !== literal) {
    throw new RuntimePayloadCodecError(`expected runtime literal string ${literal} at ${path}`);
  }
  return value;
}

function withoutNullable(schema: JsonSchema): JsonSchema {
  const { nullable: _nullable, ...rest } = schema;
  return rest as JsonSchema;
}

function isBytesSchema(schema: JsonSchema): boolean {
  const raw = schema as Record<string, unknown>;
  return (
    raw.type === 'bytes' ||
    raw.xSkiffSymbol === 'std.bytes.bytes' ||
    raw.xSkiffSymbol === 'bytes' ||
    (raw.type === 'string' && raw.contentEncoding === 'base64')
  );
}

function isDateSchema(schema: JsonSchema): boolean {
  const raw = schema as Record<string, unknown>;
  return raw.xSkiffSymbol === 'Date';
}

function bytesFromValue(value: unknown, path: string): Uint8Array {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (typeof value === 'string') {
    return Buffer.from(value, 'base64');
  }
  throw new RuntimePayloadCodecError(`expected bytes at ${path}`);
}

class PayloadWriter {
  private chunks: Buffer[] = [];

  writeU8(value: number): void {
    const buffer = Buffer.allocUnsafe(1);
    buffer.writeUInt8(value);
    this.chunks.push(buffer);
  }

  writeLen(value: number): void {
    if (!Number.isInteger(value) || value < 0 || value > 0xffffffff) {
      throw new RuntimePayloadCodecError('runtime payload length exceeds u32');
    }
    const buffer = Buffer.allocUnsafe(4);
    buffer.writeUInt32LE(value);
    this.chunks.push(buffer);
  }

  writeF64(value: number): void {
    const buffer = Buffer.allocUnsafe(8);
    buffer.writeDoubleLE(value);
    this.chunks.push(buffer);
  }

  writeI64(value: number): void {
    const buffer = Buffer.allocUnsafe(8);
    buffer.writeBigInt64LE(BigInt(value));
    this.chunks.push(buffer);
  }

  writeStringRaw(value: string): void {
    this.writeBuffer(Buffer.from(value, 'utf8'));
  }

  writeBuffer(value: Uint8Array): void {
    this.writeLen(value.byteLength);
    this.writeBytes(value);
  }

  writeBytes(value: Uint8Array): void {
    this.chunks.push(Buffer.from(value.buffer, value.byteOffset, value.byteLength));
  }

  toBuffer(): Buffer {
    return Buffer.concat(this.chunks);
  }
}

class PayloadReader {
  private offset = 0;
  private readonly buffer: Buffer;

  constructor(payloadBytes: Uint8Array) {
    this.buffer = Buffer.from(
      payloadBytes.buffer,
      payloadBytes.byteOffset,
      payloadBytes.byteLength
    );
  }

  done(): boolean {
    return this.offset === this.buffer.byteLength;
  }

  remaining(): number {
    return this.buffer.byteLength - this.offset;
  }

  expectTag(expected: number, path: string): void {
    const actual = this.readU8();
    if (actual !== expected) {
      throw new RuntimePayloadCodecError(
        `runtime payload expected tag ${expected}, got ${actual} at ${path}`
      );
    }
  }

  readU8(): number {
    this.ensure(1);
    const value = this.buffer.readUInt8(this.offset);
    this.offset += 1;
    return value;
  }

  readLen(): number {
    this.ensure(4);
    const value = this.buffer.readUInt32LE(this.offset);
    this.offset += 4;
    return value;
  }

  readF64(): number {
    this.ensure(8);
    const value = this.buffer.readDoubleLE(this.offset);
    this.offset += 8;
    return value;
  }

  readI64(): number {
    this.ensure(8);
    const value = this.buffer.readBigInt64LE(this.offset);
    this.offset += 8;
    return Number(value);
  }

  readStringRaw(): string {
    return this.readBuffer().toString('utf8');
  }

  readBuffer(): Buffer {
    const len = this.readLen();
    return this.readBytes(len);
  }

  readBytes(len: number): Buffer {
    this.ensure(len);
    const bytes = this.buffer.subarray(this.offset, this.offset + len);
    this.offset += len;
    return bytes;
  }

  private ensure(len: number): void {
    if (this.offset + len > this.buffer.byteLength) {
      throw new RuntimePayloadCodecError('runtime payload ended early');
    }
  }
}
