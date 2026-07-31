import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import {
  COMMAND_EXECUTION_LEDGER,
  COMMAND_OWNER_CLASSES,
} from './command-execution-ledger.mjs';
import { scanCommandExecutionSource } from './command-execution-scanner.mjs';
import { discoverJavaScriptFiles, repoRelative } from './verify-discovery.mjs';

const ALLOWED_IMPORTED_SYMBOLS = new Set(['spawn', 'execFile', 'execFileSync']);

export async function assertCommandExecutionPolicy(root, {
  ledger = COMMAND_EXECUTION_LEDGER,
} = {}) {
  const violations = await commandExecutionPolicyViolations(root, { ledger });
  if (violations.length > 0) {
    throw new Error([
      'command execution policy failed:',
      ...violations.map((violation) => `- ${violation}`),
    ].join('\n'));
  }
}

export async function commandExecutionPolicyViolations(root, {
  ledger = COMMAND_EXECUTION_LEDGER,
} = {}) {
  const violations = validateCommandExecutionLedger(ledger);
  if (violations.length > 0) {
    return violations;
  }

  const scriptsRoot = join(root, 'scripts');
  const discovered = await discoverJavaScriptFiles(scriptsRoot);
  const productionFiles = discovered
    .map((absolutePath) => ({
      absolutePath,
      path: `scripts/${repoRelative(scriptsRoot, absolutePath)}`,
    }))
    .filter(({ path }) => !path.startsWith('scripts/tests/'));
  const discoveredPaths = new Set(productionFiles.map(({ path }) => path));
  for (const entry of ledger) {
    if (!discoveredPaths.has(entry.path)) {
      violations.push(`stale ledger path is not a discovered production script: ${entry.path}`);
    }
  }

  for (const file of productionFiles) {
    const source = await readFile(file.absolutePath, 'utf8');
    const scan = scanCommandExecutionSource(source, file.path);
    violations.push(...scan.bypasses);
    const entries = ledger.filter((entry) => entry.path === file.path);

    for (const imported of scan.imports) {
      const exact = entries.filter((entry) =>
        entry.importedSymbol === imported.importedSymbol
        && entry.localAlias === imported.localAlias);
      if (exact.length === 0) {
        const aliasOwner = entries.find((entry) => entry.localAlias === imported.localAlias);
        violations.push(aliasOwner === undefined
          ? `${file.path}:${imported.line} unregistered child_process import ${imported.importedSymbol} as ${imported.localAlias}`
          : `${file.path}:${imported.line} imported symbol mismatch for ${imported.localAlias}: expected ${aliasOwner.importedSymbol}, found ${imported.importedSymbol}`);
      }
    }

    for (const entry of entries) {
      const imports = scan.imports.filter((imported) =>
        imported.importedSymbol === entry.importedSymbol
        && imported.localAlias === entry.localAlias);
      if (imports.length !== 1) {
        violations.push(
          `${entry.path} ledger owner ${entry.ownerId} expected exactly one import ${entry.importedSymbol} as ${entry.localAlias}, found ${imports.length}`,
        );
      }

      const calls = scan.calls.filter((call) => call.localAlias === entry.localAlias);
      if (calls.length !== entry.callCount) {
        violations.push(
          `${entry.path} ledger owner ${entry.ownerId} expected ${entry.callCount} direct call(s) through ${entry.localAlias}, found ${calls.length}`,
        );
      }
      for (const call of calls) {
        if (call.ownerId !== entry.ownerId) {
          violations.push(
            `${entry.path}:${call.line} owner marker mismatch for ${entry.localAlias}: expected ${entry.ownerId}, found ${call.ownerId ?? 'none'}`,
          );
        }
        if (call.ownerFunction !== entry.ownerFunction) {
          violations.push(
            `${entry.path}:${call.line} owner function mismatch for ${entry.localAlias}: expected ${entry.ownerFunction}, found ${call.ownerFunction ?? 'none'}`,
          );
        }
      }

      const references = scan.references.filter((reference) =>
        reference.localAlias === entry.localAlias);
      for (const reference of references) {
        violations.push(
          `${entry.path}:${reference.line} child_process alias ${entry.localAlias} has an unregistered non-call reference`,
        );
      }
    }

    for (const marker of scan.unusedMarkers) {
      violations.push(
        `${file.path}:${marker.line} unused child-process owner marker ${marker.ownerId}`,
      );
    }
  }
  return [...new Set(violations)].sort();
}

export function validateCommandExecutionLedger(ledger) {
  const violations = [];
  if (!Array.isArray(ledger)) {
    return ['command execution ledger must be an array'];
  }
  const entryKeys = new Set();
  const ownerIds = new Set();
  const aliasesByPath = new Map();
  const ownerClasses = new Set(Object.values(COMMAND_OWNER_CLASSES));
  for (const entry of ledger) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) {
      violations.push(`invalid command execution ledger entry: ${JSON.stringify(entry)}`);
      continue;
    }
    const entryKey = `${entry.path}:${entry.localAlias}`;
    if (entryKeys.has(entryKey)) {
      violations.push(`duplicate command execution ledger entry: ${entryKey}`);
    }
    entryKeys.add(entryKey);
    if (!isProductionScriptPath(entry.path)) {
      violations.push(`invalid command execution ledger path: ${entry.path}`);
    }
    if (!ALLOWED_IMPORTED_SYMBOLS.has(entry.importedSymbol)) {
      violations.push(`invalid child_process imported symbol for ${entryKey}: ${entry.importedSymbol}`);
    }
    if (!isIdentifier(entry.localAlias)) {
      violations.push(`invalid child_process local alias for ${entryKey}`);
    }
    const pathAliases = aliasesByPath.get(entry.path) ?? new Set();
    if (pathAliases.has(entry.localAlias)) {
      violations.push(`duplicate child_process local alias in ${entry.path}: ${entry.localAlias}`);
    }
    pathAliases.add(entry.localAlias);
    aliasesByPath.set(entry.path, pathAliases);
    if (!isNonEmptyString(entry.ownerId) || ownerIds.has(entry.ownerId)) {
      violations.push(`duplicate or invalid command execution owner id: ${entry.ownerId}`);
    }
    ownerIds.add(entry.ownerId);
    if (!isIdentifier(entry.ownerFunction)) {
      violations.push(`invalid owner function for ${entryKey}: ${entry.ownerFunction}`);
    }
    if (!Number.isInteger(entry.callCount) || entry.callCount <= 0) {
      violations.push(`invalid call count for ${entryKey}: ${entry.callCount}`);
    }
    if (!ownerClasses.has(entry.ownerClass)) {
      violations.push(`invalid owner class for ${entryKey}: ${entry.ownerClass}`);
    }
    if (!isNonEmptyString(entry.reason)) {
      violations.push(`missing owner reason for ${entryKey}`);
    }
  }
  return [...new Set(violations)].sort();
}

function isProductionScriptPath(path) {
  return typeof path === 'string'
    && /^scripts\/(?!tests\/).+\.(?:mjs|js|cjs)$/.test(path);
}

function isIdentifier(value) {
  return typeof value === 'string' && /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(value);
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}
