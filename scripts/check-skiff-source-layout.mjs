import { readdir, readFile, stat } from 'node:fs/promises';
import { basename, dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const failures = [];

await checkManifests();
await checkSources();

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`FAIL ${failure}`);
  }
  process.exit(1);
}

console.log('Skiff source layout checks passed.');

async function checkManifests() {
  const preludeRoot = join(root, 'prelude');
  const stdRoot = join(root, 'std');

  const legacyRoot = join(root, 'stdlib');
  expect(!(await pathExists(legacyRoot)), 'root legacy standard library dir must not remain');

  const preludeManifestPath = join(preludeRoot, 'prelude.yml');
  expect(!(await pathExists(preludeManifestPath)), 'prelude.yml must not exist; native types are declared via export native type in .skiff source files');

  for (const required of ['collection', 'stream', 'actor', 'session', 'error', 'date', 'bytes', 'json', 'config']) {
    const skiffPath = join(preludeRoot, `${required}.skiff`);
    expect(await pathExists(skiffPath), `prelude/${required}.skiff must exist`);
  }

  const stdRegistry = await readText(join(stdRoot, 'registry.yml'));
  expectContains(stdRegistry, 'schemaVersion: skiff-std-registry-v1', 'std registry schema');
  expectContains(stdRegistry, 'id: skiff.run/std', 'std registry id');
  for (const legacy of [
    'id: ext',
    'path: ../ext',
    'id: std.json',
    'id: std.http',
    'id: skiff.run/llm',
    'std.values',
    'std.core',
  ]) {
    expectNotContains(stdRegistry, legacy, `std registry must not contain ${legacy}`);
  }

  const stdManifest = await readText(join(stdRoot, 'package.yml'));
  for (const expected of [
    'id: skiff.run/std',
    'version: 1.0.0',
  ]) {
    expectContains(stdManifest, expected, `std package.yml must contain ${expected}`);
  }
  for (const legacy of ['- llm', 'valuesRequirements', 'path: dashscopeApiKey']) {
    expectNotContains(stdManifest, legacy, `std package.yml must not contain ${legacy}`);
  }

  for (const oldManifest of ['json/package.yml', 'http/package.yml', 'llm/package.yml']) {
    const path = join(stdRoot, oldManifest);
    expect(!(await pathExists(path)), `old std module manifest must not remain: ${oldManifest}`);
  }
}

async function checkSources() {
  const sources = [];
  await collectSkiffSources(join(root, 'prelude'), sources);
  await collectSkiffSources(join(root, 'std'), sources);

  expect(sources.length > 0, 'prelude/std must contain Skiff sources');

  for (const path of sources) {
    if (basename(path).endsWith('.test.skiff')) {
      continue;
    }

    const relPath = toPosix(relative(root, path));
    const source = await readText(path);

    expect(!basename(path).includes('v1'), `source identities must not include v1: ${relPath}`);
    for (const legacy of ['SecretString', 'std.values', 'values.']) {
      expectNotContains(source, legacy, `${relPath} must not contain legacy values surface ${legacy}`);
    }

    checkKnownSource(relPath, source);
  }
}

function checkKnownSource(relPath, source) {
  switch (relPath) {
    case 'std/http.skiff':
      for (const typeName of [
        'HttpRequest',
        'HttpResponse',
        'HttpClientRequest',
        'HttpClientResponse',
        'HttpClientStreamHandle',
        'HttpSseEvent',
      ]) {
        expectExportedType(source, typeName, relPath);
      }
      for (const removed of ['HttpBody', 'HttpClientHeader']) {
        expectNotMatches(source, exportedTypePattern(removed), `${relPath} must not export ${removed}`);
      }
      for (const name of ['request', 'stream', 'sse', 'emitResponseStream']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      return;
    case 'std/file.skiff':
      expectExportedType(source, 'FileError', relPath);
      for (const typeName of ['ImmutableFile', 'CreateOptions', 'FileInfo']) {
        expectExportedType(source, typeName, relPath);
      }
      for (const name of ['create', 'createText', 'read', 'readText', 'info', 'delete', 'createFromStream']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      return;
    case 'std/websocket.skiff':
      for (const typeName of ['WebSocketConnectRequest']) {
        expectExportedType(source, typeName, relPath);
      }
      for (const name of [
        'sendTextToConnection',
        'sendBinaryToConnection',
        'sendTextToBusinessIdentity',
        'sendBinaryToBusinessIdentity',
      ]) {
        expectExportedNativeFunction(source, name, relPath);
      }
      for (const name of ['sendJsonToConnection', 'sendJsonToBusinessIdentity']) {
        expectExportedSourceFunction(source, name, relPath);
      }
      return;
    case 'std/json.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      for (const name of ['encode', 'decode']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      for (const name of ['parse', 'stringify', 'from', 'get', 'at', 'asString', 'asNumber', 'asBool', 'asArray']) {
        expectNotMatches(source, exportedNativeFunctionPattern(name), `${relPath} must not export native ${name}`);
      }
      return;
    case 'std/bytes.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      return;
    case 'std/db.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      return;
    case 'std/number.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      return;
    case 'std/service.skiff':
      for (const typeName of ['ProviderUnavailableError', 'ProtocolError']) {
        expectExportedType(source, typeName, relPath);
      }
      return;
    case 'std/log.skiff':
      for (const name of ['debug', 'info', 'warn', 'error']) {
        expectExportedSourceFunction(source, name, relPath);
        expectMatches(
          source,
          new RegExp(`\\b(?:export\\s+)?function\\s+${escapeRegExp(name)}\\s*\\([^)]*attrs\\s*:\\s*JsonObject\\?`, 's'),
          `${relPath} ${name} must accept attrs: JsonObject?`,
        );
      }
      expectContains(source, 'telemetry.emit', `${relPath} log wrappers must call telemetry.emit`);
      return;
    case 'std/string.skiff':
      for (const name of ['split', 'isAsciiDigits', 'encodeQueryComponent', 'encodePath']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      return;
    case 'std/crypto.skiff':
      for (const name of ['hmacSha1Base64', 'sha256', 'randomToken', 'uuid', 'uuidSimple']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      return;
    case 'std/time.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      expectExportedNativeFunction(source, 'sleep', relPath);
      return;
    case 'prelude/config.skiff':
      expectExportedNativeType(source, 'Config', relPath);
      expectNotMatches(source, exportedNativeFunctionPattern('get'), `${relPath} must not export native get`);
      expectNotMatches(source, exportedFunctionPattern('get'), `${relPath} must not expose config.get`);
      return;
    case 'prelude/date.skiff':
      expectExportedNativeType(source, 'Date', relPath);
      expectContains(source, 'impl Date', `${relPath} must define impl Date`);
      for (const name of ['now', 'fromEpochMilliseconds', 'parse', 'requireParse']) {
        expectMatches(source, staticNativeFunctionPattern(name), `${relPath} must export native static ${name}`);
      }
      for (const name of ['toEpochMilliseconds', 'toISOString', 'addMilliseconds', 'diffMilliseconds', 'compare', 'isBefore', 'isAfter']) {
        expectMatches(source, receiverNativeFunctionPattern(name), `${relPath} must export native receiver ${name}`);
      }
      return;
    case 'prelude/collection.skiff':
      for (const typeName of ['Array', 'Map']) {
        expectExportedNativeType(source, typeName, relPath);
      }
      return;
    case 'prelude/stream.skiff':
      expectExportedNativeType(source, 'Stream', relPath);
      return;
    case 'prelude/actor.skiff':
      expectExportedNativeType(source, 'ActorRef', relPath);
      return;
    case 'prelude/session.skiff':
      for (const typeName of ['ClientSessionRef', 'ClientCapability']) {
        expectExportedNativeType(source, typeName, relPath);
      }
      return;
    case 'prelude/error.skiff':
      for (const typeName of [
        'ErrorPayload',
        'Exception',
        'CatchResult',
        'SourceLocation',
        'StackTrace',
        'StackFrame',
        'TimeoutError',
        'CancelError',
        'InternalError',
      ]) {
        expectExportedNativeType(source, typeName, relPath);
      }
      return;
    case 'prelude/json.skiff':
      for (const typeName of ['Json', 'JsonObject']) {
        expectExportedNativeType(source, typeName, relPath);
      }
      return;
    case 'prelude/number.skiff':
      expectContains(source, 'impl number', `${relPath} must define impl number`);
      for (const name of ['isInteger', 'isSafeInteger', 'assertSafeInteger']) {
        expectMatches(source, staticNativeFunctionPattern(name), `${relPath} must export native static ${name}`);
      }
      return;
    case 'prelude/bytes.skiff':
      expectExportedNativeType(source, 'bytes', relPath);
      expectContains(source, 'impl bytes', `${relPath} must define impl bytes`);
      expectMatches(source, staticNativeFunctionPattern('concat'), `${relPath} must export native static concat`);
      return;
    default:
      return;
  }
}

async function collectSkiffSources(directory, results) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!shouldSkipDirectory(entry.name)) {
        await collectSkiffSources(path, results);
      }
      continue;
    }
    if (entry.isFile() && entry.name.endsWith('.skiff')) {
      results.push(path);
    }
  }
}

function expectExportedType(source, name, relPath) {
  expectMatches(source, exportedTypePattern(name), `${relPath} must export type ${name}`);
}

function expectExportedNativeType(source, name, relPath) {
  expectMatches(source, exportedNativeTypePattern(name), `${relPath} must export native type ${name}`);
}

function expectExportedNativeFunction(source, name, relPath) {
  expectMatches(source, exportedNativeFunctionPattern(name), `${relPath} must export native function ${name}`);
}

function expectExportedSourceFunction(source, name, relPath) {
  expectMatches(source, exportedFunctionPattern(name), `${relPath} must export function ${name}`);
  expectNotMatches(source, exportedNativeFunctionPattern(name), `${relPath} ${name} must not be native`);
}

function exportedTypePattern(name) {
  return new RegExp(`\\b(?:export\\s+)?type\\s+${escapeRegExp(name)}\\b`);
}

function exportedNativeTypePattern(name) {
  return new RegExp(`\\b(?:export\\s+)?native\\s+type\\s+${escapeRegExp(name)}\\b`);
}

function exportedFunctionPattern(name) {
  return new RegExp(`\\b(?:export\\s+)?(?:native\\s+)?function\\s+${escapeRegExp(name)}\\b`);
}

function exportedNativeFunctionPattern(name) {
  return new RegExp(`\\b(?:export\\s+)?native\\s+function\\s+${escapeRegExp(name)}\\b`);
}

function staticNativeFunctionPattern(name) {
  return new RegExp(`\\bnative\\s+static\\s+function\\s+${escapeRegExp(name)}\\b`);
}

function receiverNativeFunctionPattern(name) {
  return new RegExp(`\\bnative\\s+function\\s+${escapeRegExp(name)}\\b`);
}

async function readText(path) {
  return readFile(path, 'utf8');
}

async function pathExists(path) {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function expectContains(text, needle, message) {
  expect(text.includes(needle), message);
}

function expectNotContains(text, needle, message) {
  expect(!text.includes(needle), message);
}

function expectMatches(text, pattern, message) {
  expect(pattern.test(text), message);
}

function expectNotMatches(text, pattern, message) {
  expect(!pattern.test(text), message);
}

function expect(condition, message) {
  if (!condition) {
    failures.push(message);
  }
}

function shouldSkipDirectory(name) {
  return name === 'target' || name === 'node_modules' || name.startsWith('.');
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function toPosix(path) {
  return path.split(sep).join('/');
}
