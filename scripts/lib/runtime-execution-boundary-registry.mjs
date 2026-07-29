import { lineNumberAt } from './runtime-artifact-boundary-rust-source.mjs';
import {
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS,
} from './runtime-execution-boundary-subjects.mjs';
import { scanRuntimeExecutionBoundarySource } from './runtime-execution-boundary-source.mjs';

const FORBIDDEN_REGISTRY_FIELDS = Object.freeze([
  'allowlist',
  'ignoredFiles',
  'ignoredSymbols',
  'knownViolations',
  'ledger',
]);

export function validateRuntimeExecutionBoundaryRegistry(registry) {
  if (!registry || typeof registry !== 'object') {
    return [runtimeExecutionBoundaryViolation({
      id: 'invalid-execution-boundary-registry',
      detail: 'execution boundary registry must be an object',
    })];
  }
  if (
    !Array.isArray(registry.sourceRoots)
    || !Array.isArray(registry.subjects)
    || !Array.isArray(registry.owners)
  ) {
    return [runtimeExecutionBoundaryViolation({
      id: 'invalid-execution-boundary-registry',
      detail: 'sourceRoots, subjects, and owners must all be arrays',
    })];
  }

  const violations = [];
  validateSourceRoots(registry.sourceRoots, violations);
  const subjectsById = validateSubjects(registry.subjects, violations);
  validateOwners(registry.owners, subjectsById, violations);
  return violations;
}

function validateSourceRoots(sourceRoots, violations) {
  const sourceIds = new Set();
  for (const sourceRoot of sourceRoots) {
    if (
      !sourceRoot
      || typeof sourceRoot.id !== 'string'
      || !['rust', 'typescript'].includes(sourceRoot.language)
      || !isExactPath(sourceRoot.root)
    ) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'invalid-execution-boundary-registry',
        detail: 'every source root requires an id, supported language, and exact path',
      }));
      continue;
    }
    if (sourceIds.has(sourceRoot.id)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'duplicate-source-root',
        subject: sourceRoot.id,
        detail: `source root ${sourceRoot.id} is registered more than once`,
      }));
    }
    sourceIds.add(sourceRoot.id);
    rejectExceptionFields(sourceRoot, sourceRoot.id, violations);
  }
}

function validateSubjects(subjects, violations) {
  const subjectsById = new Map();
  for (const subject of subjects) {
    if (
      !subject
      || typeof subject.id !== 'string'
      || !['rust', 'typescript'].includes(subject.language)
      || !Array.isArray(subject.discoveryRoots)
      || !Array.isArray(subject.requiredFiles)
    ) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'invalid-execution-boundary-registry',
        detail: 'every subject requires an id, supported language, and root/file arrays',
      }));
      continue;
    }
    if (subjectsById.has(subject.id)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'duplicate-subject',
        subject: subject.id,
        detail: `subject ${subject.id} is registered more than once`,
      }));
    }
    subjectsById.set(subject.id, subject);
    validateExactPaths(subject.id, 'discoveryRoots', subject.discoveryRoots, violations);
    validateExactPaths(subject.id, 'requiredFiles', subject.requiredFiles, violations);
    if (!subject.zones || typeof subject.zones !== 'object' || Array.isArray(subject.zones)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'invalid-execution-boundary-registry',
        subject: subject.id,
        detail: 'subject zones must be an object of exact path arrays',
      }));
    } else {
      for (const [zone, roots] of Object.entries(subject.zones)) {
        if (!Array.isArray(roots)) {
          violations.push(runtimeExecutionBoundaryViolation({
            id: 'invalid-execution-boundary-registry',
            subject: subject.id,
            detail: `zone ${zone} must be an exact path array`,
          }));
        } else {
          validateExactPaths(subject.id, `zones.${zone}`, roots, violations);
        }
      }
    }
    rejectExceptionFields(subject, subject.id, violations);
  }
  for (const requiredId of REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS) {
    if (!subjectsById.has(requiredId)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'subject-registry-omission',
        subject: requiredId,
        detail: `required execution boundary subject ${requiredId} is absent`,
      }));
    }
  }
  return subjectsById;
}

function validateOwners(owners, subjectsById, violations) {
  const ownersByRole = new Map();
  for (const owner of owners) {
    if (
      !owner
      || typeof owner.role !== 'string'
      || typeof owner.subjectId !== 'string'
      || !['rust', 'typescript'].includes(owner.language)
      || !['struct', 'function', 'method'].includes(owner.declarationKind)
      || typeof owner.symbol !== 'string'
      || !Array.isArray(owner.ownedRoots)
      || !Array.isArray(owner.requiredAnchors)
    ) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'invalid-execution-boundary-registry',
        detail: 'every owner requires a role, subject, declaration, symbol, and root/anchor arrays',
      }));
      continue;
    }
    if (ownersByRole.has(owner.role)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'duplicate-owner-role',
        ownerRole: owner.role,
        detail: `owner role ${owner.role} is registered more than once`,
      }));
    }
    ownersByRole.set(owner.role, owner);
    if (!subjectsById.has(owner.subjectId)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'owner-subject-missing',
        subject: owner.subjectId,
        ownerRole: owner.role,
        detail: `owner ${owner.role} references an absent subject`,
      }));
    } else {
      const subject = subjectsById.get(owner.subjectId);
      for (const ownedRoot of owner.ownedRoots) {
        if (!subject.discoveryRoots.some((root) => pathIsWithin(ownedRoot, root))) {
          violations.push(runtimeExecutionBoundaryViolation({
            id: 'owner-root-registry-omission',
            subject: owner.subjectId,
            ownerRole: owner.role,
            relPath: ownedRoot,
            detail: `owner root ${ownedRoot} is outside its subject discovery registry`,
          }));
        }
      }
      if (owner.requiredFile && !subject.requiredFiles.includes(owner.requiredFile)) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'required-owner-file-registry-omission',
          subject: owner.subjectId,
          ownerRole: owner.role,
          relPath: owner.requiredFile,
          detail: `required owner file ${owner.requiredFile} is absent from its subject registry`,
        }));
      }
    }
    validateExactPaths(owner.subjectId, 'ownedRoots', owner.ownedRoots, violations, owner.role);
    if (owner.requiredFile !== undefined && !isExactPath(owner.requiredFile)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'non-exact-subject-root',
        subject: owner.subjectId,
        ownerRole: owner.role,
        detail: `requiredFile contains a non-exact path: ${String(owner.requiredFile)}`,
      }));
    }
    rejectExceptionFields(owner, owner.role, violations);
  }
  for (const requiredRole of REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES) {
    if (!ownersByRole.has(requiredRole)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'owner-registry-omission',
        ownerRole: requiredRole,
        detail: `required production owner role ${requiredRole} is absent`,
      }));
    }
  }
}

export function inspectRuntimeExecutionBoundaryOwners(registry, sources, violations) {
  const ownerMatches = new Map();
  for (const owner of registry.owners) {
    if (!owner || typeof owner.role !== 'string' || typeof owner.symbol !== 'string') {
      continue;
    }
    if (owner.requiredFile && !sources.has(owner.requiredFile)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'required-owner-file-missing',
        subject: owner.subjectId,
        ownerRole: owner.role,
        relPath: owner.requiredFile,
        detail: `required exact owner file ${owner.requiredFile} is absent or test-only`,
      }));
    }
    const matches = [];
    const regexp = ownerDeclarationRegexp(owner);
    for (const [relPath, source] of sources) {
      if (source.language !== owner.language) {
        continue;
      }
      for (const match of source.identifiers.matchAll(regexp)) {
        const item = declarationItem(source, match.index);
        matches.push({
          index: match.index,
          item,
          line: lineNumberAt(source.identifiers, match.index),
          owner,
          relPath,
          source,
        });
      }
    }
    ownerMatches.set(owner.role, matches);
    if (matches.length === 0) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'required-owner-missing',
        subject: owner.subjectId,
        ownerRole: owner.role,
        relPath: owner.requiredFile ?? owner.ownedRoots[0],
        matched: owner.symbol,
        detail: `required ${owner.declarationKind} owner ${owner.symbol} is not declared`,
      }));
      continue;
    }
    if (matches.length > 1) {
      for (const match of matches) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'duplicate-required-owner',
          subject: owner.subjectId,
          ownerRole: owner.role,
          relPath: match.relPath,
          line: match.line,
          matched: owner.symbol,
          detail: `owner ${owner.symbol} has ${matches.length} production declarations`,
        }));
      }
    }
    for (const match of matches) {
      if (!owner.ownedRoots.some((root) => pathIsWithin(match.relPath, root))) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'owner-outside-registered-root',
          subject: owner.subjectId,
          ownerRole: owner.role,
          relPath: match.relPath,
          line: match.line,
          matched: owner.symbol,
          detail: `owner ${owner.symbol} moved or was copied outside its exact registered roots`,
        }));
      }
      for (const anchor of owner.requiredAnchors) {
        if (!ownerItemHasTokenAnchor(match.item, owner.language, anchor)) {
          violations.push(runtimeExecutionBoundaryViolation({
            id: ownerAnchorViolationId(owner.role),
            subject: owner.subjectId,
            ownerRole: owner.role,
            relPath: match.relPath,
            line: match.line,
            matched: anchor,
            detail: `owner ${owner.symbol} does not contain required structural anchor ${anchor}`,
          }));
        }
      }
    }
  }
  return ownerMatches;
}

export function pathIsWithin(relPath, root) {
  return relPath === root || relPath.startsWith(`${root}/`);
}

export function escapeRuntimeExecutionBoundaryRegexp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function runtimeExecutionBoundaryViolation({
  id,
  subject,
  ownerRole,
  relPath,
  line,
  matched,
  detail,
}) {
  return { id, subject, ownerRole, relPath, line, matched, detail };
}

function ownerDeclarationRegexp(owner) {
  const symbol = escapeRuntimeExecutionBoundaryRegexp(owner.symbol);
  if (owner.language === 'rust' && owner.declarationKind === 'struct') {
    return new RegExp(`\\bstruct\\s+${symbol}\\b`, 'g');
  }
  if (owner.language === 'rust' && owner.declarationKind === 'function') {
    return new RegExp(`\\bfn\\s+${symbol}\\b`, 'g');
  }
  if (owner.language === 'typescript' && owner.declarationKind === 'method') {
    return new RegExp(
      `(?:^|\\n)\\s*(?:(?:public|private|protected|static|async|override)\\s+)*${symbol}\\s*\\(`,
      'gm',
    );
  }
  return /$a/g;
}

function declarationItem(source, index) {
  const code = source.code;
  const semicolon = code.indexOf(';', index);
  const brace = code.indexOf('{', index);
  let end;
  if (semicolon !== -1 && (brace === -1 || semicolon < brace)) {
    end = semicolon + 1;
  } else if (brace !== -1) {
    const close = matchingDelimiterIndex(code, brace, '{', '}');
    end = close === -1 ? code.length : close + 1;
  } else {
    const newline = code.indexOf('\n', index);
    end = newline === -1 ? code.length : newline;
  }
  return {
    commentless: source.commentless.slice(index, end),
    code: source.code.slice(index, end),
    identifiers: source.identifiers.slice(index, end),
    tokens: source.tokens
      .filter((entry) => entry.start >= index && entry.end <= end)
      .map((entry) => ({ ...entry, end: entry.end - index, start: entry.start - index })),
  };
}

function ownerItemHasTokenAnchor(item, language, anchor) {
  const expected = scanRuntimeExecutionBoundarySource(anchor, language).tokens
    .filter(({ kind }) => !['comment', 'literal'].includes(kind));
  if (expected.length === 0) {
    return false;
  }
  const actual = item.tokens.filter(({ kind }) => !['comment', 'literal'].includes(kind));
  return actual.some((_token, start) => expected.every(
    (candidate, offset) => actual[start + offset]?.kind === candidate.kind
      && actual[start + offset]?.value === candidate.value,
  ));
}

function matchingDelimiterIndex(text, openIndex, open, close) {
  let depth = 0;
  for (let index = openIndex; index < text.length; index += 1) {
    if (text[index] === open) {
      depth += 1;
    } else if (text[index] === close) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function validateExactPaths(subject, field, roots, violations, ownerRole) {
  for (const root of roots) {
    if (!isExactPath(root)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'non-exact-subject-root',
        subject,
        ownerRole,
        detail: `${field} contains a non-exact path: ${String(root)}`,
      }));
    }
  }
}

function rejectExceptionFields(entry, label, violations) {
  for (const field of FORBIDDEN_REGISTRY_FIELDS) {
    if (Object.hasOwn(entry, field)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'forbidden-registry-exception-field',
        subject: label,
        detail: `execution boundary registries may not carry ${field}`,
      }));
    }
  }
}

function ownerAnchorViolationId(role) {
  if (role === 'service-dispatcher') {
    return 'dispatcher-owner-incomplete';
  }
  if (
    role === 'host-request-route-lookup'
    || role === 'assembly-request-wire'
    || role === 'assembly-request-spawn'
  ) {
    return 'host-active-assembly-entry-missing';
  }
  if (role === 'router-runtime-service-rejection') {
    return 'router-service-rejection-incomplete';
  }
  return 'required-owner-anchor-missing';
}

function isExactPath(path) {
  return (
    typeof path === 'string'
    && path.length > 0
    && !path.startsWith('/')
    && !path.includes('..')
    && !/[*?{}[\]]/.test(path)
  );
}
