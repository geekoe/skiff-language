import { stat } from 'node:fs/promises';
import { join } from 'node:path';

import {
  collectTestOnlyModuleFiles,
  externalModuleNames,
  lineNumberAt,
  loadRuntimeRustSources,
  productionRustViews,
  resolveModuleFile,
} from './runtime-artifact-boundary-rust-source.mjs';

import {
  REQUIRED_RUNTIME_ARTIFACT_BOUNDARY_SUBJECT_IDS,
  RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS,
} from './runtime-artifact-boundary-subjects.mjs';

const canonicalAnchor = /\b(?:RuntimeAssemblyLoader|HydratedRuntimeAssembly|SharedPackageLinkedImage|AssemblyLinkedCandidate|AssemblyAdmissionController|link_runtime_assembly|admit_runtime_assembly)\b/;
const canonicalOwnerDeclaration = /\b(?:struct|enum|trait)\s+(RuntimeAssemblyLoader|HydratedRuntimeAssembly|SharedPackageLinkedImage|AssemblyLinkedCandidate|AssemblyAdmissionController)\b/g;

const denyRules = Object.freeze([
  rule(
    'legacy-runtime-dto',
    'old ServiceUnit/PackageUnit/service assembly/artifact-index/linked-program DTO owner',
    /\b[A-Za-z0-9_]*(?:ServiceUnit|PackageUnit|ServiceAssembly|ArtifactIndexPointer|LinkedProgramImageBuild)[A-Za-z0-9_]*\b|\b[A-Za-z0-9_]*(?:service_unit|package_unit|service_assembly|artifact_index_pointer|linked_program_image_build)[A-Za-z0-9_]*\b/g,
  ),
  rule(
    'raw-service-assembly-wire',
    'raw serviceAssembly/service_assembly wire traversal',
    /\bserviceAssembly\b|\bservice_assembly\b/g,
    'commentless',
  ),
  rule(
    'raw-json-semantic-linking',
    'serde_json/json macro in the typed load/link/admission owner',
    /\bserde_json\b|\bjson\s*!/g,
  ),
  rule(
    'display-or-source-linking',
    'display/source path or symbol linking',
    /\b[A-Za-z0-9_]*(?:display_name|displayName|source_path|sourcePath|PackageSymbolRef|ServiceSymbolRef|PackageOperationSymbolRef)[A-Za-z0-9_]*\b|\b(?:resolve|link|select|target|infer)[A-Za-z0-9_]*(?:display(?:_name|Name|_path)?|source_(?:path|symbol)|sourcePath|symbol_path|by_name)[A-Za-z0-9_]*\b/g,
  ),
  rule(
    'request-time-lazy-load',
    'request-time, lazy, or on-demand artifact loading',
    /\b[A-Za-z0-9_]*(?:lazy_load|load_lazy|on_demand_load|request_time_load|lazy_artifact|load_artifact_for_request|load_service_for_request)[A-Za-z0-9_]*\b/g,
  ),
  rule(
    'compatibility-or-fallback-path',
    'legacy/compatibility/fallback/dual-read production path',
    /\b(?:legacy|compat(?:ibility)?|fallback|dual_(?:read|write|path|load))[A-Za-z0-9_]*\b|\b[A-Za-z0-9_]+_(?:legacy|compat(?:ibility)?|fallback|dual_(?:read|write|path|load))[A-Za-z0-9_]*\b|\b[A-Za-z0-9_]*(?:Legacy|Compat(?:ibility)?|Fallback|Dual(?:Read|Write|Path|Load))[A-Za-z0-9_]*\b/g,
  ),
  rule(
    'provider-executable-patch',
    'provider executable patching or provider inference',
    /\b[A-Za-z0-9_]*(?:(?:patch|rewrite|replace)[A-Za-z0-9_]*(?:provider|executable)|(?:provider|executable)[A-Za-z0-9_]*(?:patch|rewrite|replace)|(?:infer|guess|select)[A-Za-z0-9_]*provider)[A-Za-z0-9_]*\b/g,
  ),
]);

export async function collectRuntimeArtifactBoundaryViolations(
  repoRoot,
  subjects = RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS,
) {
  const violations = validateSubjectRegistry(subjects);
  const runtimeRoot = join(repoRoot, 'runtime');
  if (!(await isDirectory(runtimeRoot))) {
    return [
      ...violations,
      violation({
        id: 'runtime-root-missing',
        detail: `runtime source root does not exist: ${runtimeRoot}`,
      }),
    ];
  }

  const sources = await loadRuntimeRustSources(repoRoot, runtimeRoot);
  const testOnlyFiles = collectTestOnlyModuleFiles(sources);
  const production = new Map();
  for (const [relPath, source] of sources) {
    if (testOnlyFiles.has(relPath)) {
      continue;
    }
    production.set(relPath, productionRustViews(source));
  }

  const selected = new Map();
  for (const subject of subjects) {
    for (const root of subject.ownedRoots ?? []) {
      const sourceMatches = [...sources.keys()].filter((relPath) => pathIsWithin(relPath, root));
      const matches = sourceMatches.filter((relPath) => production.has(relPath));
      if (sourceMatches.length === 0 && !subject.allowMissingOwnedRoots) {
        violations.push(
          violation({
            id: 'subject-root-missing',
            subject: subject.id,
            relPath: root,
            detail: `registered production owner root is absent: ${root}`,
          }),
        );
      }
      for (const relPath of matches) {
        selectFile(selected, relPath, subject.id);
      }
    }

    for (const root of subject.discoveryRoots ?? []) {
      for (const [relPath, views] of production) {
        if (pathIsWithin(relPath, root) && canonicalAnchor.test(views.identifiers)) {
          selectFile(selected, relPath, subject.id);
          selectDeclaredModules(relPath, subject.id, production, selected);
        }
      }
    }
  }

  const ownerDeclarations = new Map();
  for (const [relPath, views] of production) {
    for (const match of views.identifiers.matchAll(canonicalOwnerDeclaration)) {
      const owner = match[1];
      const paths = ownerDeclarations.get(owner) ?? [];
      paths.push(relPath);
      ownerDeclarations.set(owner, paths);
      if (!isInsideAnySubject(relPath, subjects)) {
        violations.push(
          violation({
            id: 'unregistered-canonical-owner',
            relPath,
            line: lineNumberAt(views.identifiers, match.index),
            matched: owner,
            detail: `canonical owner ${owner} moved or copied outside every registered subject boundary`,
          }),
        );
      }
    }
  }
  for (const [owner, paths] of ownerDeclarations) {
    if (paths.length > 1) {
      for (const relPath of paths) {
        violations.push(
          violation({
            id: 'duplicate-canonical-owner',
            relPath,
            matched: owner,
            detail: `canonical owner ${owner} is declared in multiple production files: ${paths.join(', ')}`,
          }),
        );
      }
    }
  }

  for (const [relPath, subjectIds] of selected) {
    const views = production.get(relPath);
    for (const denyRule of denyRules) {
      const text = views[denyRule.view];
      for (const match of text.matchAll(denyRule.regexp)) {
        violations.push(
          violation({
            id: denyRule.id,
            subject: [...subjectIds].sort().join(','),
            relPath,
            line: lineNumberAt(text, match.index),
            matched: match[0],
            detail: denyRule.detail,
          }),
        );
      }
    }
  }

  return violations.sort(compareViolations);
}

export function formatRuntimeArtifactBoundaryViolation(entry) {
  const location = entry.relPath
    ? `${entry.relPath}${entry.line ? `:${entry.line}` : ''}`
    : '<subject-registry>';
  const subject = entry.subject ? ` subject=${entry.subject}` : '';
  const matched = entry.matched ? ` matched=${JSON.stringify(entry.matched)}` : '';
  return `${location} ${entry.id}${subject}${matched}: ${entry.detail}`;
}

function validateSubjectRegistry(subjects) {
  const violations = [];
  if (!Array.isArray(subjects)) {
    return [violation({ id: 'invalid-subject-registry', detail: 'subject registry must be an array' })];
  }
  const ids = new Set();
  for (const subject of subjects) {
    if (!subject || typeof subject !== 'object' || typeof subject.id !== 'string') {
      violations.push(
        violation({ id: 'invalid-subject-registry', detail: 'every subject requires a string id' }),
      );
      continue;
    }
    if (ids.has(subject.id)) {
      violations.push(
        violation({
          id: 'duplicate-subject',
          subject: subject.id,
          detail: `subject ${subject.id} is registered more than once`,
        }),
      );
    }
    ids.add(subject.id);
    if (!['canonical', 'consumer'].includes(subject.kind)) {
      violations.push(
        violation({
          id: 'invalid-subject-kind',
          subject: subject.id,
          detail: `subject kind must be canonical or consumer, got ${subject.kind}`,
        }),
      );
    }
    for (const [field, roots] of [
      ['ownedRoots', subject.ownedRoots],
      ['discoveryRoots', subject.discoveryRoots],
    ]) {
      if (!Array.isArray(roots)) {
        violations.push(
          violation({
            id: 'invalid-subject-roots',
            subject: subject.id,
            detail: `${field} must be an array`,
          }),
        );
        continue;
      }
      for (const root of roots) {
        if (
          typeof root !== 'string'
          || !root.startsWith('runtime/')
          || root.includes('..')
          || /[*?{}[\]]/.test(root)
        ) {
          violations.push(
            violation({
              id: 'non-exact-subject-root',
              subject: subject.id,
              detail: `${field} contains a non-exact runtime path: ${String(root)}`,
            }),
          );
        }
      }
    }
    for (const forbiddenField of ['allowlist', 'knownViolations', 'ledger', 'ignoredSymbols']) {
      if (Object.hasOwn(subject, forbiddenField)) {
        violations.push(
          violation({
            id: 'forbidden-subject-exception-field',
            subject: subject.id,
            detail: `subject registries may not carry ${forbiddenField}`,
          }),
        );
      }
    }
    if ((subject.ownedRoots?.length ?? 0) + (subject.discoveryRoots?.length ?? 0) === 0) {
      violations.push(
        violation({
          id: 'empty-subject-boundary',
          subject: subject.id,
          detail: 'subject must own or discover at least one exact production root',
        }),
      );
    }
  }
  for (const required of REQUIRED_RUNTIME_ARTIFACT_BOUNDARY_SUBJECT_IDS) {
    if (!ids.has(required)) {
      violations.push(
        violation({
          id: 'subject-registry-omission',
          subject: required,
          detail: `required production owner subject ${required} is absent`,
        }),
      );
    }
  }
  return violations;
}

function selectFile(selected, relPath, subjectId) {
  const subjectIds = selected.get(relPath) ?? new Set();
  subjectIds.add(subjectId);
  selected.set(relPath, subjectIds);
}

function selectDeclaredModules(relPath, subjectId, production, selected, seen = new Set()) {
  if (seen.has(relPath)) {
    return;
  }
  seen.add(relPath);
  const views = production.get(relPath);
  if (!views) {
    return;
  }
  for (const moduleName of externalModuleNames(views.identifiers)) {
    const child = resolveModuleFile(relPath, moduleName, production);
    if (!child) {
      continue;
    }
    selectFile(selected, child, subjectId);
    selectDeclaredModules(child, subjectId, production, selected, seen);
  }
}

async function isDirectory(path) {
  try {
    return (await stat(path)).isDirectory();
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function isInsideAnySubject(relPath, subjects) {
  return subjects.some((subject) =>
    [...(subject.ownedRoots ?? []), ...(subject.discoveryRoots ?? [])].some((root) =>
      pathIsWithin(relPath, root)));
}

function pathIsWithin(relPath, root) {
  return relPath === root || relPath.startsWith(`${root}/`);
}

function rule(id, detail, regexp, view = 'identifiers') {
  return Object.freeze({ id, detail, regexp, view });
}

function violation({ id, subject, relPath, line, matched, detail }) {
  return { id, subject, relPath, line, matched, detail };
}

function compareViolations(left, right) {
  return [left.relPath ?? '', left.line ?? 0, left.id, left.subject ?? '']
    .join('\0')
    .localeCompare([right.relPath ?? '', right.line ?? 0, right.id, right.subject ?? ''].join('\0'));
}
