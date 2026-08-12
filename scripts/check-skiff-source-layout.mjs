import { readdir, readFile, stat } from 'node:fs/promises';
import { basename, dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const failures = [];

const requiredStdSurface = {
  http: {
    types: [
      'HttpHeader',
      'HttpQueryParam',
      'HttpRequest',
      'HttpResponse',
      'HttpResponseStreamEvent',
      'HttpClientRequest',
      'HttpClientResponse',
      'HttpClientStreamHandle',
      'HttpSseEvent',
      'HttpError',
      'RequestTimeoutError',
    ],
    nativeFunctions: [
      'request',
      'stream',
      'sse',
      'header',
      'headers',
      'query',
      'cookie',
      'json',
      'jsonWithHeaders',
      'errorResponse',
      'noContent',
      'methodNotAllowed',
      'decodeJson',
      'requireMethod',
      'forwardableHeaders',
      'sseHeaders',
      'streamStart',
      'streamChunk',
      'streamEnd',
      'emitResponseStream',
    ],
    sourceFunctions: [],
  },
  service: {
    types: ['ProviderUnavailableError', 'ProtocolError', 'InternalError'],
    nativeFunctions: [],
    sourceFunctions: [],
  },
  websocket: {
    types: [
      'WebSocketConnectRequest',
      'WebSocketConnectionPolicy',
      'WebSocketConnectResult',
      'WebSocketRequestError',
    ],
    nativeFunctions: [
      'sendTextToConnection',
      'sendBinaryToConnection',
      'sendTextToBusinessIdentity',
      'sendBinaryToBusinessIdentity',
      'requestJsonToConnection',
    ],
    sourceFunctions: ['sendJsonToConnection', 'sendJsonToBusinessIdentity'],
  },
  actor: {
    types: ['ActivationTimeoutError', 'MethodInvocationTimeoutError'],
    nativeFunctions: ['get'],
    sourceFunctions: [],
  },
};

const removedWebSocketFunctions = ['receive', 'sendText', 'sendBinary', 'sendJson'];

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
  const compilerBuiltinRegistryPath = join(root, 'compiler', 'core', 'src', 'prelude_registry.rs');

  const legacyRoot = join(root, 'stdlib');
  expect(!(await pathExists(legacyRoot)), 'root legacy standard library dir must not remain');

  const preludeManifestPath = join(preludeRoot, 'prelude.yml');
  expect(
    !(await pathExists(preludeManifestPath)),
    'prelude.yml must not exist; builtin type identity is owned by the compiler registry',
  );

  const compilerBuiltinRegistry = await readText(compilerBuiltinRegistryPath);
  expectContains(
    compilerBuiltinRegistry,
    'pub const COMPILER_BUILTIN_TYPES:',
    'compiler/core prelude registry must define COMPILER_BUILTIN_TYPES',
  );
  for (const builtin of [
    'bytes',
    'Array',
    'Map',
    'Config',
    'Date',
    'Json',
    'JsonObject',
    'Stream',
    'Exception',
    'CatchResult',
    'SourceLocation',
    'StackTrace',
    'StackFrame',
    'TimeoutError',
    'ClientSessionRef',
    'ClientCapability',
  ]) {
    expectMatches(
      compilerBuiltinRegistry,
      compilerBuiltinNamePattern(builtin),
      `compiler builtin registry must own ${builtin}`,
    );
  }
  for (const removed of ['ActorRef', 'CancelError']) {
    expectNotMatches(
      compilerBuiltinRegistry,
      compilerBuiltinNamePattern(removed),
      `compiler builtin registry must not expose ${removed}`,
    );
  }

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

  const stdApiExports = parseStdApiExports(await readText(join(stdRoot, 'api.yml')));
  for (const [moduleName, surface] of Object.entries(requiredStdSurface)) {
    for (const name of [...surface.types, ...surface.nativeFunctions, ...surface.sourceFunctions]) {
      expect(
        stdApiExports.get(`${moduleName}.${name}`) === `${moduleName}.${name}`,
        `std/api.yml must export ${moduleName}.${name}`,
      );
    }
  }
  for (const removed of removedWebSocketFunctions) {
    expect(
      !stdApiExports.has(`websocket.${removed}`),
      `std/api.yml must not export obsolete websocket.${removed}`,
    );
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
    if (relPath.startsWith('prelude/')) {
      expectNotMatches(
        source,
        removedNativeTypeDeclarationPattern(),
        `${relPath} must not declare native types; builtin types are compiler-owned`,
      );
    }
    expectNotMatches(
      source,
      /\bActorRef\b/,
      `${relPath} must not expose legacy ActorRef in Skiff source`,
    );

    checkKnownSource(relPath, source);
  }
}

function checkKnownSource(relPath, source) {
  switch (relPath) {
    case 'std/http.skiff':
      for (const typeName of requiredStdSurface.http.types) {
        expectExportedType(source, typeName, relPath);
      }
      for (const removed of ['HttpBody', 'HttpClientHeader']) {
        expectNotMatches(source, exportedTypePattern(removed), `${relPath} must not export ${removed}`);
      }
      for (const name of requiredStdSurface.http.nativeFunctions) {
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
      for (const typeName of requiredStdSurface.websocket.types) {
        expectExportedType(source, typeName, relPath);
      }
      for (const name of requiredStdSurface.websocket.nativeFunctions) {
        expectExportedNativeFunction(source, name, relPath);
      }
      for (const name of requiredStdSurface.websocket.sourceFunctions) {
        expectExportedSourceFunction(source, name, relPath);
      }
      for (const removed of removedWebSocketFunctions) {
        expectNotMatches(
          source,
          exportedFunctionPattern(removed),
          `${relPath} must not export obsolete function ${removed}`,
        );
      }
      return;
    case 'std/json.skiff':
      expectExportedType(source, 'DecodeError', relPath);
      for (const name of ['encode', 'decode']) {
        expectExportedNativeFunction(source, name, relPath);
      }
      for (const name of ['parse', 'stringify', 'from', 'at', 'asString', 'asNumber', 'asBool', 'asArray']) {
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
      for (const typeName of requiredStdSurface.service.types) {
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
      expectExportedType(source, 'DecodeError', relPath);
      expectNotMatches(source, exportedNativeFunctionPattern('get'), `${relPath} must not export native get`);
      expectNotMatches(source, exportedFunctionPattern('get'), `${relPath} must not expose config.get`);
      return;
    case 'prelude/date.skiff':
      expectContains(source, 'impl Date', `${relPath} must define impl Date`);
      for (const name of ['now', 'fromEpochMilliseconds', 'parse', 'requireParse']) {
        expectMatches(source, staticNativeFunctionPattern(name), `${relPath} must export native static ${name}`);
      }
      for (const name of ['toEpochMilliseconds', 'toISOString', 'addMilliseconds', 'diffMilliseconds', 'compare', 'isBefore', 'isAfter']) {
        expectMatches(source, receiverNativeFunctionPattern(name), `${relPath} must export native receiver ${name}`);
      }
      return;
    case 'prelude/collection.skiff':
      for (const typeName of [
        'ArrayIndexOutOfBoundsError',
        'MapKeyNotFoundError',
        'JsonObjectPropertyNotFoundError',
      ]) {
        expectExportedType(source, typeName, relPath);
      }
      for (const typeName of ['Array', 'Map']) {
        expectContains(source, `impl ${typeName}`, `${relPath} must define impl ${typeName}`);
      }
      return;
    case 'prelude/stream.skiff':
      return;
    case 'prelude/actor.skiff':
      for (const typeName of requiredStdSurface.actor.types) {
        expectExportedType(source, typeName, relPath);
      }
      expectMatches(source, nativeFunctionPattern('get'), `${relPath} must define native function get`);
      return;
    case 'prelude/session.skiff':
      return;
    case 'prelude/error.skiff':
      for (const typeName of ['TimeoutError', 'InstructionLimitExceededError']) {
        expectExportedType(source, typeName, relPath);
      }
      return;
    case 'prelude/json.skiff':
      return;
    case 'prelude/number.skiff':
      expectContains(source, 'impl number', `${relPath} must define impl number`);
      for (const name of ['isInteger', 'isSafeInteger', 'assertSafeInteger']) {
        expectMatches(source, staticNativeFunctionPattern(name), `${relPath} must export native static ${name}`);
      }
      return;
    case 'prelude/bytes.skiff':
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

function removedNativeTypeDeclarationPattern(name) {
  const typeName = name === undefined ? '[A-Za-z_][A-Za-z0-9_]*' : escapeRegExp(name);
  return new RegExp(`\\b(?:export\\s+)?native\\s+type\\s+${typeName}\\b`);
}

function exportedFunctionPattern(name) {
  return new RegExp(`\\b(?:export\\s+)?(?:native\\s+)?function\\s+${escapeRegExp(name)}\\b`);
}

function exportedNativeFunctionPattern(name) {
  return new RegExp(`\\b(?:export\\s+)?native\\s+function\\s+${escapeRegExp(name)}\\b`);
}

function nativeFunctionPattern(name) {
  return new RegExp(`\\bnative\\s+function\\s+${escapeRegExp(name)}\\b`);
}

function compilerBuiltinNamePattern(name) {
  return new RegExp(`\\bname:\\s*"${escapeRegExp(name)}"\\s*,`);
}

function parseStdApiExports(source) {
  const exports = new Map();
  let moduleName;
  for (const line of source.split(/\r?\n/)) {
    const moduleMatch = /^([A-Za-z_][A-Za-z0-9_]*):\s*$/.exec(line);
    if (moduleMatch) {
      moduleName = moduleMatch[1];
      continue;
    }

    const exportMatch = /^  ([A-Za-z_][A-Za-z0-9_]*):\s*(\S+)\s*$/.exec(line);
    if (moduleName !== undefined && exportMatch) {
      exports.set(`${moduleName}.${exportMatch[1]}`, exportMatch[2]);
    }
  }
  return exports;
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
