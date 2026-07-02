#!/usr/bin/env node

import { access, readFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const nightlyProbeTimeoutMs = 10_000;

const standardCrates = ['std', 'core', 'alloc'];
const approvedExternalValueCrates = ['serde', 'serde_json'];

const defaultConfigs = new Map([
  [
    'skiff-compiler-input-model',
    {
      allowedCrates: [
        'skiff-compiler-input-model',
        'skiff-compiler-core',
        'skiff-artifact-model',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'input-model public API excludes skiff-syntax/parser/AST unless explicitly allowed later',
    },
  ],
  [
    'skiff-compiler-input',
    {
      allowedCrates: [
        'skiff-compiler-input',
        'skiff-compiler-core',
        'skiff-compiler-input-model',
        'skiff-artifact-model',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'input public API allows only self/core/input-model/artifact-model/std and approved value crates',
    },
  ],
  [
    'skiff-compiler-projection-input',
    {
      allowedCrates: [
        'skiff-compiler-projection-input',
        'skiff-compiler-core',
        'skiff-artifact-model',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'projection-input public API allows only self/core/artifact-model/std and approved value crates',
    },
  ],
  [
    'skiff-compiler-source',
    {
      allowedCrates: [
        'skiff-compiler-source',
        'skiff-compiler-core',
        'skiff-compiler-input-model',
        'skiff-artifact-model',
        'skiff-syntax',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'source public API allows only self/core/input-model/artifact-model/syntax/std and approved value crates',
    },
  ],
  [
    'skiff-compiler-lowering',
    {
      allowedCrates: [
        'skiff-compiler-lowering',
        'skiff-compiler-core',
        'skiff-compiler-source',
        'skiff-artifact-model',
        'skiff-syntax',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'lowering public API allows only self/core/source/artifact-model/syntax/std and approved value crates',
    },
  ],
  [
    'skiff-compiler-compiled',
    {
      allowedCrates: [
        'skiff-compiler-compiled',
        'skiff-compiler-core',
        'skiff-compiler-source',
        'skiff-compiler-lowering',
        'skiff-compiler-projection-input',
        'skiff-artifact-model',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'compiled public API allows only self/core/source/lowering/projection-input/artifact-model/std and approved value crates',
    },
  ],
  [
    'skiff-compiler-projection',
    {
      allowedCrates: [
        'skiff-compiler-projection',
        'skiff-compiler-core',
        'skiff-compiler-projection-input',
        'skiff-artifact-model',
        ...standardCrates,
        ...approvedExternalValueCrates,
      ],
      note: 'projection public API allows only self/core/projection-input/artifact-model/std and approved value crates',
    },
  ],
]);

const typeVariantKeys = new Set([
  'array',
  'borrowed_ref',
  'dyn_trait',
  'function_pointer',
  'generic',
  'impl_trait',
  'infer',
  'never',
  'pat',
  'primitive',
  'qualified_path',
  'raw_pointer',
  'resolved_path',
  'slice',
  'tuple',
]);

const innerVariantKeys = new Set([
  'assoc_const',
  'assoc_type',
  'constant',
  'enum',
  'extern_crate',
  'function',
  'impl',
  'module',
  'static',
  'struct',
  'struct_field',
  'trait',
  'trait_alias',
  'type_alias',
  'union',
  'use',
  'variant',
]);

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});

async function main() {
  const options = parseArgs(process.argv.slice(2));

  if (options.help) {
    printUsage();
    return;
  }

  if (options.selfTest) {
    runSelfTest();
    return;
  }

  if (!options.crateName) {
    throw new Error('missing crate name; run with --help for usage');
  }

  const metadata = await cargoMetadata();
  const packageInfo = metadata.packages.find((pkg) => pkg.name === options.crateName);
  if (!packageInfo) {
    console.log(
      `SKIP public API check for ${options.crateName}: package is not present in this workspace yet.`,
    );
    return;
  }

  const config = configForCrate(options.crateName, options.extraAllowedCrates);
  await runRustdocJson(options.crateName);

  const rustdocPath = rustdocJsonPath(metadata, packageInfo);
  await assertReadable(rustdocPath);
  const rustdocJson = JSON.parse(await readFile(rustdocPath, 'utf8'));
  const result = checkPublicApi(rustdocJson, {
    crateName: options.crateName,
    allowedCrates: config.allowedCrates,
  });

  printConfig(options.crateName, config);
  printResult(result);
  if (result.violations.length > 0) {
    process.exitCode = 1;
  }
}

function parseArgs(argv) {
  const options = {
    crateName: undefined,
    extraAllowedCrates: [],
    help: false,
    selfTest: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--self-test' || arg === '--test') {
      options.selfTest = true;
      continue;
    }
    if (arg === '--allow-crate' || arg === '--allow') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${arg} requires a crate name`);
      }
      options.extraAllowedCrates.push(value);
      index += 1;
      continue;
    }
    if (arg.startsWith('--allow-crate=')) {
      options.extraAllowedCrates.push(arg.slice('--allow-crate='.length));
      continue;
    }
    if (arg.startsWith('--allow=')) {
      options.extraAllowedCrates.push(arg.slice('--allow='.length));
      continue;
    }
    if (arg === '--allow-list') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--allow-list requires a comma-separated crate list');
      }
      options.extraAllowedCrates.push(...splitCrateList(value));
      index += 1;
      continue;
    }
    if (arg.startsWith('--allow-list=')) {
      options.extraAllowedCrates.push(...splitCrateList(arg.slice('--allow-list='.length)));
      continue;
    }
    if (arg.startsWith('--')) {
      throw new Error(`unknown option: ${arg}`);
    }
    if (options.crateName) {
      throw new Error(`unexpected extra crate name: ${arg}`);
    }
    options.crateName = arg;
  }

  return options;
}

function printUsage() {
  console.log(`Usage:
  node scripts/check-crate-public-api.mjs <crate> [--allow-crate <crate> ...]
  node scripts/check-crate-public-api.mjs --self-test

Checks exported public API types with rustdoc JSON:
  cargo +nightly rustdoc -p <crate> --lib -- -Z unstable-options --output-format json
  RUSTC_BOOTSTRAP=1 cargo rustdoc -p <crate> --lib -- -Z unstable-options --output-format json

Default gated crates:
  skiff-compiler-input-model
  skiff-compiler-input
  skiff-compiler-source
  skiff-compiler-lowering
  skiff-compiler-compiled
  skiff-compiler-projection
  skiff-compiler-projection-input`);
}

function splitCrateList(value) {
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function configForCrate(crateName, extraAllowedCrates) {
  const base = defaultConfigs.get(crateName) ?? {
    allowedCrates: [crateName, ...standardCrates],
    note: 'no default allow-list exists; using self plus std/core/alloc',
  };

  return {
    allowedCrates: uniqueCrates([...base.allowedCrates, ...extraAllowedCrates]),
    note: base.note,
  };
}

function printConfig(crateName, config) {
  console.log(`Public API allow-list for ${crateName}: ${config.allowedCrates.join(', ')}`);
  console.log(`Policy: ${config.note}`);
}

function printResult(result) {
  if (result.violations.length === 0) {
    console.log(`Public API check passed for ${result.crateName}.`);
    return;
  }

  console.error(
    `Public API check failed for ${result.crateName}: ${result.violations.length} forbidden reference(s).`,
  );
  for (const violation of result.violations) {
    console.error(
      `DENY ${violation.site} references ${violation.referencedPath} from forbidden crate ${violation.crateName}`,
    );
  }
}

async function cargoMetadata() {
  const result = await runCommand('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
    cwd: root,
  });
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`failed to parse cargo metadata JSON: ${error.message}`);
  }
}

async function runRustdocJson(crateName) {
  const nightlyProbe = await probeCargoNightly();
  const attempts = [];
  if (nightlyProbe.available) {
    attempts.push(rustdocJsonCommand(crateName, { nightly: true }));
  } else {
    console.warn(
      `Nightly Rust toolchain is unavailable; falling back to current toolchain with RUSTC_BOOTSTRAP=1.`,
    );
  }
  attempts.push(rustdocJsonCommand(crateName, { nightly: false }));

  const failures = [];
  for (const attempt of attempts) {
    try {
      await runCommand(attempt.command, attempt.args, attempt.options);
      if (failures.length > 0) {
        console.warn(`Built rustdoc JSON for ${crateName} with ${attempt.label}.`);
      }
      return;
    } catch (error) {
      failures.push({ attempt, error });
    }
  }

  const detailParts = [];
  if (!nightlyProbe.available && nightlyProbe.error) {
    detailParts.push(
      `nightly probe failed; skipped cargo +nightly rustdoc:\n${commandFailureDetail(
        nightlyProbe.error,
      )}`,
    );
  }
  for (const failure of failures) {
    detailParts.push(`${failure.attempt.label} failed:\n${commandFailureDetail(failure.error)}`);
  }
  throw new Error(
    `failed to build rustdoc JSON for ${crateName}. This crate exists, so rustdoc JSON support is a blocking failure.\n${detailParts.join(
      '\n\n',
    )}`,
  );
}

async function probeCargoNightly() {
  try {
    await runCommand('cargo', ['+nightly', '--version'], {
      cwd: root,
      timeoutMs: nightlyProbeTimeoutMs,
    });
    return { available: true };
  } catch (error) {
    return { available: false, error };
  }
}

function rustdocJsonCommand(crateName, { nightly }) {
  const args = [
    ...(nightly ? ['+nightly'] : []),
    'rustdoc',
    '-p',
    crateName,
    '--lib',
    '--',
    '-Z',
    'unstable-options',
    '--output-format',
    'json',
  ];
  const options = { cwd: root };
  if (!nightly) {
    options.env = { ...process.env, RUSTC_BOOTSTRAP: '1' };
  }
  return {
    args,
    command: 'cargo',
    label: nightly ? 'cargo +nightly rustdoc' : 'RUSTC_BOOTSTRAP=1 cargo rustdoc',
    options,
  };
}

function rustdocJsonPath(metadata, packageInfo) {
  const libTarget = packageInfo.targets.find((target) => target.kind.includes('lib'));
  if (!libTarget) {
    throw new Error(`${packageInfo.name} exists but has no lib target to document`);
  }
  return join(metadata.target_directory, 'doc', `${rustdocFileStem(libTarget.name)}.json`);
}

function rustdocFileStem(targetName) {
  return targetName.replaceAll('-', '_');
}

async function assertReadable(path) {
  try {
    await access(path, constants.R_OK);
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      throw new Error(`rustdoc JSON was not produced at ${path}`);
    }
    throw error;
  }
}

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    let settled = false;
    let timedOut = false;
    let timeout;

    if (options.timeoutMs) {
      timeout = setTimeout(() => {
        timedOut = true;
        child.kill('SIGKILL');
      }, options.timeoutMs);
    }

    function complete(callback) {
      if (settled) {
        return;
      }
      settled = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      callback();
    }

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      error.command = formatCommand(command, args);
      error.stdout = stdout;
      error.stderr = stderr;
      complete(() => reject(error));
    });
    child.on('close', (code, signal) => {
      if (code === 0) {
        complete(() => resolve({ stdout, stderr }));
        return;
      }
      const message = timedOut
        ? `${formatCommand(command, args)} timed out after ${options.timeoutMs}ms`
        : `${formatCommand(command, args)} exited with ${code ?? signal}`;
      const error = new Error(message);
      error.command = formatCommand(command, args);
      error.exitCode = code;
      error.signal = signal;
      error.stdout = stdout;
      error.stderr = stderr;
      error.timedOut = timedOut;
      error.timeoutMs = timedOut ? options.timeoutMs : undefined;
      complete(() => reject(error));
    });
  });
}

function commandFailureDetail(error) {
  const parts = [`command: ${error.command ?? 'unknown'}`];
  if (error.message) {
    parts.push(`error: ${error.message}`);
  }
  if (error.stderr) {
    parts.push(`stderr:\n${error.stderr.trimEnd()}`);
  }
  if (error.stdout) {
    parts.push(`stdout:\n${error.stdout.trimEnd()}`);
  }
  return parts.join('\n');
}

function formatCommand(command, args) {
  return [command, ...args].join(' ');
}

function checkPublicApi(rustdocJson, config) {
  const context = createContext(rustdocJson, config);
  const rootId = rustdocJson.root;
  if (!rootId) {
    throw new Error('rustdoc JSON is missing root item id');
  }
  inspectItem(context, rootId, { site: context.crateName, forcePublic: true });
  context.violations.sort((left, right) => {
    const leftKey = `${left.site}\0${left.crateName}\0${left.referencedPath}`;
    const rightKey = `${right.site}\0${right.crateName}\0${right.referencedPath}`;
    return leftKey.localeCompare(rightKey);
  });
  return {
    crateName: context.crateName,
    violations: context.violations,
  };
}

function createContext(rustdocJson, config) {
  return {
    allowedCrates: new Set(config.allowedCrates.map(normalizeCrateName)),
    crateName: config.crateName,
    externalCrates: rustdocJson.external_crates ?? {},
    index: rustdocJson.index ?? {},
    paths: rustdocJson.paths ?? {},
    seenItems: new Set(),
    seenViolations: new Set(),
    violations: [],
  };
}

function inspectItem(context, id, exposure) {
  const item = context.index[id];
  if (!item) {
    recordReferenceById(context, id, exposure.site);
    return;
  }
  if (!exposure.forcePublic && !isPublicVisibility(item.visibility)) {
    return;
  }

  const visitKey = `${id}`;
  if (context.seenItems.has(visitKey)) {
    return;
  }
  context.seenItems.add(visitKey);

  const itemLabel = itemLabelFor(context, id, item, exposure.site);
  const inner = unwrapInner(item.inner);
  if (!inner) {
    return;
  }

  switch (inner.kind) {
    case 'module':
      inspectModule(context, inner.value, itemLabel);
      break;
    case 'use':
      inspectUse(context, inner.value, itemLabel);
      break;
    case 'struct':
    case 'union':
      inspectStructLike(context, inner.value, itemLabel);
      break;
    case 'struct_field':
      inspectType(context, inner.value, `${itemLabel} field type`);
      break;
    case 'enum':
      inspectEnum(context, inner.value, itemLabel);
      break;
    case 'variant':
      inspectVariant(context, inner.value, itemLabel);
      break;
    case 'function':
      inspectFunction(context, inner.value, itemLabel);
      break;
    case 'type_alias':
      inspectTypeAlias(context, inner.value, itemLabel);
      break;
    case 'impl':
      inspectImpl(context, inner.value, itemLabel);
      break;
    case 'trait':
      inspectTrait(context, inner.value, itemLabel);
      break;
    case 'trait_alias':
      inspectTraitAlias(context, inner.value, itemLabel);
      break;
    case 'assoc_type':
      inspectAssocType(context, inner.value, itemLabel);
      break;
    case 'assoc_const':
    case 'constant':
    case 'static':
      inspectTypedItem(context, inner.value, itemLabel);
      break;
    case 'extern_crate':
      inspectExternCrate(context, inner.value, itemLabel);
      break;
    default:
      inspectSignatureNode(context, inner.value, itemLabel);
      break;
  }
}

function inspectModule(context, module, site) {
  for (const childId of module.items ?? []) {
    inspectItem(context, childId, { site, forcePublic: false });
  }
}

function inspectUse(context, useItem, site) {
  const targetId = useItem.id ?? useItem.target;
  if (targetId) {
    recordReferenceById(context, targetId, `${site} re-export`);
    if (context.index[targetId]) {
      inspectItem(context, targetId, { site: `${site} re-export`, forcePublic: true });
    }
  }
  inspectSignatureNode(context, useItem, `${site} re-export`);
}

function inspectStructLike(context, structItem, site) {
  inspectGenerics(context, structItem.generics, `${site} generics`);
  inspectStructKind(context, structItem.kind, site, false);
  for (const implId of structItem.impls ?? []) {
    inspectItem(context, implId, { site: `${site} impl`, forcePublic: true });
  }
}

function inspectStructKind(context, kind, site, forcePublicFields) {
  if (!kind || typeof kind !== 'object') {
    return;
  }
  const variant = unwrapVariant(kind, ['plain', 'tuple', 'unit']);
  if (!variant) {
    inspectSignatureNode(context, kind, `${site} fields`);
    return;
  }

  if (variant.kind === 'plain') {
    for (const fieldId of variant.value.fields ?? []) {
      inspectItem(context, fieldId, { site: `${site} field`, forcePublic: forcePublicFields });
    }
    return;
  }

  if (variant.kind === 'tuple') {
    const fields = Array.isArray(variant.value) ? variant.value : variant.value.fields ?? [];
    for (const fieldId of fields) {
      if (fieldId) {
        inspectItem(context, fieldId, { site: `${site} field`, forcePublic: forcePublicFields });
      }
    }
  }
}

function inspectEnum(context, enumItem, site) {
  inspectGenerics(context, enumItem.generics, `${site} generics`);
  for (const variantId of enumItem.variants ?? []) {
    inspectItem(context, variantId, { site: `${site} variant`, forcePublic: true });
  }
  for (const implId of enumItem.impls ?? []) {
    inspectItem(context, implId, { site: `${site} impl`, forcePublic: true });
  }
}

function inspectVariant(context, variant, site) {
  if (!variant || typeof variant !== 'object') {
    return;
  }
  const kind = variant.kind ?? variant;
  inspectStructKind(context, kind, site, true);
  inspectSignatureNode(context, variant, site);
}

function inspectFunction(context, functionItem, site) {
  inspectGenerics(context, functionItem.generics, `${site} generics`);
  inspectFunctionSignature(context, functionItem.sig ?? functionItem.decl, `${site} signature`);
}

function inspectFunctionSignature(context, signature, site) {
  if (!signature || typeof signature !== 'object') {
    return;
  }
  for (const input of signature.inputs ?? []) {
    if (Array.isArray(input)) {
      inspectType(context, input[1], `${site} input ${input[0]}`);
    } else {
      inspectType(context, input, `${site} input`);
    }
  }
  if (signature.output) {
    inspectType(context, signature.output, `${site} output`);
  }
}

function inspectTypeAlias(context, typeAlias, site) {
  inspectGenerics(context, typeAlias.generics, `${site} generics`);
  inspectType(context, typeAlias.type, `${site} target`);
}

function inspectImpl(context, implItem, site) {
  inspectGenerics(context, implItem.generics, `${site} generics`);
  const isTraitImpl = Boolean(implItem.trait);
  if (implItem.trait) {
    inspectTypeOrPath(context, implItem.trait, `${site} trait`);
  }
  inspectType(context, implItem.for, `${site} for type`);
  for (const itemId of implItem.items ?? []) {
    inspectItem(context, itemId, { site, forcePublic: isTraitImpl });
  }
}

function inspectTrait(context, traitItem, site) {
  inspectGenerics(context, traitItem.generics, `${site} generics`);
  inspectBounds(context, traitItem.bounds, `${site} bounds`);
  for (const itemId of traitItem.items ?? []) {
    inspectItem(context, itemId, { site, forcePublic: true });
  }
}

function inspectTraitAlias(context, traitAlias, site) {
  inspectGenerics(context, traitAlias.generics, `${site} generics`);
  inspectBounds(context, traitAlias.params ?? traitAlias.bounds, `${site} bounds`);
}

function inspectAssocType(context, assocType, site) {
  inspectGenerics(context, assocType.generics, `${site} generics`);
  inspectBounds(context, assocType.bounds, `${site} bounds`);
  inspectType(context, assocType.type, `${site} default`);
}

function inspectTypedItem(context, typedItem, site) {
  inspectType(context, typedItem.type, `${site} type`);
}

function inspectExternCrate(context, externCrate, site) {
  if (externCrate?.id) {
    recordReferenceById(context, externCrate.id, site);
  }
}

function inspectGenerics(context, generics, site) {
  if (!generics || typeof generics !== 'object') {
    return;
  }
  for (const param of generics.params ?? []) {
    inspectSignatureNode(context, param.kind, `${site} parameter ${param.name ?? ''}`.trim());
  }
  for (const predicate of generics.where_predicates ?? generics.wherePredicates ?? []) {
    inspectSignatureNode(context, predicate, `${site} where predicate`);
  }
}

function inspectBounds(context, bounds, site) {
  if (!bounds) {
    return;
  }
  inspectSignatureNode(context, bounds, site);
}

function inspectType(context, type, site) {
  if (!type || typeof type !== 'object') {
    return;
  }

  const variant = unwrapVariant(type, typeVariantKeys);
  if (!variant) {
    inspectSignatureNode(context, type, site);
    return;
  }

  switch (variant.kind) {
    case 'resolved_path':
      inspectPath(context, variant.value, site);
      break;
    case 'qualified_path':
      inspectQualifiedPath(context, variant.value, site);
      break;
    case 'borrowed_ref':
    case 'raw_pointer':
    case 'slice':
    case 'array':
    case 'pat':
      inspectType(context, variant.value.type ?? variant.value, site);
      if (variant.value.length) {
        inspectSignatureNode(context, variant.value.length, site);
      }
      break;
    case 'tuple':
      for (const innerType of variant.value ?? []) {
        inspectType(context, innerType, site);
      }
      break;
    case 'function_pointer':
      inspectFunctionSignature(context, variant.value.sig ?? variant.value, `${site} function pointer`);
      break;
    case 'dyn_trait':
    case 'impl_trait':
      inspectSignatureNode(context, variant.value, site);
      break;
    case 'generic':
    case 'primitive':
    case 'infer':
    case 'never':
      break;
    default:
      inspectSignatureNode(context, variant.value, site);
      break;
  }
}

function inspectQualifiedPath(context, qualifiedPath, site) {
  inspectType(context, qualifiedPath.self_type, `${site} self type`);
  inspectTypeOrPath(context, qualifiedPath.trait, `${site} trait`);
  inspectGenericArgs(context, qualifiedPath.args, `${site} args`);
}

function inspectTypeOrPath(context, value, site) {
  if (!value || typeof value !== 'object') {
    return;
  }
  if (isTypeObject(value)) {
    inspectType(context, value, site);
    return;
  }
  if (looksLikePath(value)) {
    inspectPath(context, value, site);
    return;
  }
  inspectSignatureNode(context, value, site);
}

function inspectPath(context, path, site) {
  if (!path || typeof path !== 'object') {
    return;
  }
  if (path.id) {
    recordReferenceById(context, path.id, site);
  }
  inspectGenericArgs(context, path.args, `${site} args`);
}

function inspectGenericArgs(context, args, site) {
  if (!args || typeof args !== 'object') {
    return;
  }
  const variant = unwrapVariant(args, ['angle_bracketed', 'parenthesized']);
  if (!variant) {
    inspectSignatureNode(context, args, site);
    return;
  }

  if (variant.kind === 'angle_bracketed') {
    for (const arg of variant.value.args ?? []) {
      inspectGenericArg(context, arg, site);
    }
    for (const constraint of variant.value.constraints ?? []) {
      inspectSignatureNode(context, constraint, `${site} constraint`);
    }
    return;
  }

  if (variant.kind === 'parenthesized') {
    for (const input of variant.value.inputs ?? []) {
      inspectType(context, input, `${site} input`);
    }
    if (variant.value.output) {
      inspectType(context, variant.value.output, `${site} output`);
    }
  }
}

function inspectGenericArg(context, arg, site) {
  if (!arg || typeof arg !== 'object') {
    return;
  }
  if (arg.type) {
    inspectType(context, arg.type, site);
    return;
  }
  inspectSignatureNode(context, arg, site);
}

function inspectSignatureNode(context, node, site) {
  if (!node || typeof node !== 'object') {
    return;
  }

  if (Array.isArray(node)) {
    for (const entry of node) {
      inspectSignatureNode(context, entry, site);
    }
    return;
  }

  if (isTypeObject(node)) {
    inspectType(context, node, site);
    return;
  }

  if (looksLikePath(node)) {
    inspectPath(context, node, site);
    return;
  }

  for (const [key, value] of Object.entries(node)) {
    if (key === 'id' || key === 'name' || key === 'span' || key === 'docs' || key === 'attrs') {
      continue;
    }
    if (key === 'type') {
      inspectType(context, value, `${site} type`);
      continue;
    }
    if (key === 'trait') {
      inspectTypeOrPath(context, value, `${site} trait`);
      continue;
    }
    if (key === 'args') {
      inspectGenericArgs(context, value, `${site} args`);
      continue;
    }
    inspectSignatureNode(context, value, site);
  }
}

function recordReferenceById(context, id, site) {
  const summary = context.paths[id];
  if (!summary && context.index[id]) {
    return;
  }

  const crateName = crateNameForReference(context, id, summary);
  if (!crateName) {
    return;
  }

  if (context.allowedCrates.has(normalizeCrateName(crateName))) {
    return;
  }

  const referencedPath = referencePath(context, id, summary);
  const key = `${site}\0${crateName}\0${referencedPath}`;
  if (context.seenViolations.has(key)) {
    return;
  }
  context.seenViolations.add(key);
  context.violations.push({
    crateName,
    referencedPath,
    site,
  });
}

function crateNameForReference(context, id, summary) {
  if (!summary) {
    return undefined;
  }

  const external = context.externalCrates[String(summary.crate_id)];
  if (external?.name) {
    return external.name;
  }

  if (context.index[id]) {
    return context.crateName;
  }

  if (summary.path?.[0]) {
    return summary.path[0];
  }

  return undefined;
}

function referencePath(context, id, summary) {
  if (summary?.path?.length > 0) {
    return summary.path.join('::');
  }
  const item = context.index[id];
  if (item?.name) {
    return item.name;
  }
  return id;
}

function itemLabelFor(context, id, item, fallback) {
  const summary = context.paths[id];
  if (summary?.path?.length > 0) {
    return summary.path.join('::');
  }
  if (item.name) {
    return `${fallback}::${item.name}`;
  }
  return fallback;
}

function unwrapInner(inner) {
  return unwrapVariant(inner, innerVariantKeys);
}

function unwrapVariant(value, allowedKeys) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }
  for (const [key, innerValue] of Object.entries(value)) {
    if (allowedKeys.has ? allowedKeys.has(key) : allowedKeys.includes(key)) {
      return { kind: key, value: innerValue };
    }
  }
  return undefined;
}

function isTypeObject(value) {
  return Boolean(unwrapVariant(value, typeVariantKeys));
}

function looksLikePath(value) {
  return (
    value &&
    typeof value === 'object' &&
    typeof value.id === 'string' &&
    typeof value.name === 'string' &&
    ('args' in value || !('inner' in value))
  );
}

function isPublicVisibility(visibility) {
  return visibility === 'public';
}

function normalizeCrateName(crateName) {
  return crateName.replaceAll('-', '_');
}

function uniqueCrates(crates) {
  const seen = new Set();
  const unique = [];
  for (const crateName of crates) {
    const normalized = normalizeCrateName(crateName);
    if (seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    unique.push(crateName);
  }
  return unique;
}

function runSelfTest() {
  const config = {
    crateName: 'skiff-compiler-projection-input',
    allowedCrates: defaultConfigs.get('skiff-compiler-projection-input').allowedCrates,
  };

  const allowedResult = checkPublicApi(fakeAllowedRustdoc(), config);
  assertEqual(allowedResult.violations.length, 0, 'allowed fake rustdoc should pass');

  const deniedResult = checkPublicApi(fakeDeniedRustdoc(), config);
  const deniedCrates = new Set(deniedResult.violations.map((violation) => violation.crateName));
  for (const crateName of [
    'skiff_compiler_compiled',
    'skiff_compiler_source',
    'skiff_compiler_lowering',
    'skiff_syntax',
  ]) {
    assert(
      deniedCrates.has(crateName),
      `denied fake rustdoc should report forbidden crate ${crateName}`,
    );
  }

  assert(
    deniedResult.violations.some((violation) => violation.site.includes('re-export')),
    'denied fake rustdoc should cover re-export checks',
  );
  assert(
    deniedResult.violations.some(
      (violation) =>
        violation.site.includes('ProjectionEnum variant::SourceBacked')
        && violation.referencedPath.endsWith('SourceCompileModel'),
    ),
    'denied fake rustdoc should cover enum variant field checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('signature input dep')),
    'denied fake rustdoc should cover public function signature checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('impl')),
    'denied fake rustdoc should cover exposed impl method checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('trait_impl_method')),
    'denied fake rustdoc should cover exposed trait impl method checks',
  );
  assert(
    !deniedResult.violations.some((violation) => violation.site.includes('private_helper')),
    'private impl methods should not be checked as public API',
  );

  console.log(
    `Self-test passed: allowed fixture 0 violation(s), denied fixture ${deniedResult.violations.length} violation(s).`,
  );
}

function fakeAllowedRustdoc() {
  return fakeRustdoc({
    rootItems: ['0:1', '0:10', '0:20'],
    index: {
      '0:1': publicItem('ProjectionDto', {
        struct: {
          generics: emptyGenerics(),
          kind: { plain: { fields: ['0:2', '0:3'] } },
          impls: ['0:30'],
        },
      }),
      '0:2': publicItem('artifact', {
        struct_field: resolvedType('3:1', 'ArtifactPublicationId'),
      }),
      '0:3': publicItem('spec', {
        struct_field: resolvedType('2:1', 'ApiSpec'),
      }),
      '0:10': publicItem('JsonDoc', {
        type_alias: {
          generics: emptyGenerics(),
          type: resolvedType('4:1', 'Value'),
        },
      }),
      '0:20': publicItem('make_projection', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['input', resolvedType('0:1', 'ProjectionDto')]],
            output: resolvedType('1:1', 'String'),
          },
        },
      }),
      '0:30': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ProjectionDto'),
          generics: emptyGenerics(),
          items: ['0:31'],
          trait: null,
        },
      }),
      '0:31': publicItem('artifact_id', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [],
            output: resolvedType('3:1', 'ArtifactPublicationId'),
          },
        },
      }),
    },
    paths: {
      '0:1': localPath('ProjectionDto', 'struct'),
      '0:10': localPath('JsonDoc', 'type_alias'),
      '0:20': localPath('make_projection', 'function'),
      '1:1': externalPath(1, ['alloc', 'string', 'String'], 'struct'),
      '2:1': externalPath(2, ['skiff_compiler_core', 'ApiSpec'], 'struct'),
      '3:1': externalPath(3, ['skiff_artifact_model', 'ArtifactPublicationId'], 'struct'),
      '4:1': externalPath(4, ['serde_json', 'Value'], 'enum'),
    },
  });
}

function fakeDeniedRustdoc() {
  return fakeRustdoc({
    rootItems: ['0:1', '0:10', '0:20', '0:40', '0:50'],
    index: {
      '0:1': publicItem('ProjectionDto', {
        struct: {
          generics: emptyGenerics(),
          kind: { plain: { fields: ['0:2'] } },
          impls: ['0:30', '0:60'],
        },
      }),
      '0:2': publicItem('compiled', {
        struct_field: resolvedType('5:1', 'CompiledPublication'),
      }),
      '0:10': publicItem('source_model', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['dep', resolvedType('6:1', 'SourceCompileModel')]],
            output: null,
          },
        },
      }),
      '0:20': publicItem(null, {
        use: {
          id: '8:1',
          name: 'AstNode',
          source: 'skiff_syntax::ast::AstNode',
        },
      }),
      '0:30': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ProjectionDto'),
          generics: emptyGenerics(),
          items: ['0:31', '0:32'],
          trait: null,
        },
      }),
      '0:31': publicItem('from_lowering', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['lowered', resolvedType('7:1', 'LoweringPrivateModel')]],
            output: resolvedType('0:1', 'ProjectionDto'),
          },
        },
      }),
      '0:32': privateItem('private_helper', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['compiled', resolvedType('5:1', 'CompiledPublication')]],
            output: null,
          },
        },
      }),
      '0:40': publicItem('BadAlias', {
        type_alias: {
          generics: emptyGenerics(),
          type: resolvedType('8:2', 'ParserState'),
        },
      }),
      '0:50': publicItem('ProjectionEnum', {
        enum: {
          generics: emptyGenerics(),
          variants: ['0:51'],
          impls: [],
        },
      }),
      '0:51': publicItem('SourceBacked', {
        variant: {
          kind: { tuple: ['0:52'] },
        },
      }),
      '0:52': privateItem('0', {
        struct_field: resolvedType('6:1', 'SourceCompileModel'),
      }),
      '0:60': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ProjectionDto'),
          generics: emptyGenerics(),
          items: ['0:61'],
          trait: {
            args: null,
            id: '2:1',
            name: 'ProjectionTrait',
          },
        },
      }),
      '0:61': privateItem('trait_impl_method', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['compiled', resolvedType('5:1', 'CompiledPublication')]],
            output: null,
          },
        },
      }),
    },
    paths: {
      '0:1': localPath('ProjectionDto', 'struct'),
      '0:10': localPath('source_model', 'function'),
      '0:20': localPath('AstNode', 'use'),
      '0:40': localPath('BadAlias', 'type_alias'),
      '0:50': localPath('ProjectionEnum', 'enum'),
      '5:1': externalPath(5, ['skiff_compiler_compiled', 'CompiledPublication'], 'struct'),
      '6:1': externalPath(6, ['skiff_compiler_source', 'SourceCompileModel'], 'struct'),
      '7:1': externalPath(7, ['skiff_compiler_lowering', 'LoweringPrivateModel'], 'struct'),
      '8:1': externalPath(8, ['skiff_syntax', 'ast', 'AstNode'], 'struct'),
      '8:2': externalPath(8, ['skiff_syntax', 'parser', 'ParserState'], 'struct'),
    },
  });
}

function fakeRustdoc({ rootItems, index, paths }) {
  return {
    crate_version: '0.0.0',
    external_crates: {
      1: { name: 'alloc' },
      2: { name: 'skiff_compiler_core' },
      3: { name: 'skiff_artifact_model' },
      4: { name: 'serde_json' },
      5: { name: 'skiff_compiler_compiled' },
      6: { name: 'skiff_compiler_source' },
      7: { name: 'skiff_compiler_lowering' },
      8: { name: 'skiff_syntax' },
    },
    format_version: 0,
    index: {
      '0:0': publicItem('skiff_compiler_projection_input', {
        module: {
          is_crate: true,
          items: rootItems,
        },
      }),
      ...index,
    },
    paths: {
      '0:0': {
        crate_id: 0,
        kind: 'module',
        path: ['skiff_compiler_projection_input'],
      },
      ...paths,
    },
    root: '0:0',
  };
}

function publicItem(name, inner) {
  return {
    attrs: [],
    docs: null,
    id: undefined,
    inner,
    links: {},
    name,
    visibility: 'public',
  };
}

function privateItem(name, inner) {
  return {
    ...publicItem(name, inner),
    visibility: 'default',
  };
}

function emptyGenerics() {
  return {
    params: [],
    where_predicates: [],
  };
}

function resolvedType(id, name) {
  return {
    resolved_path: {
      args: null,
      id,
      name,
    },
  };
}

function localPath(name, kind) {
  return {
    crate_id: 0,
    kind,
    path: ['skiff_compiler_projection_input', name],
  };
}

function externalPath(crateId, path, kind) {
  return {
    crate_id: crateId,
    kind,
    path,
  };
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`self-test failed: ${message}`);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`self-test failed: ${message}; expected ${expected}, got ${actual}`);
  }
}
