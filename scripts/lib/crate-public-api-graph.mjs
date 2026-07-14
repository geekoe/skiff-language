// Pure rustdoc JSON public-API graph traversal.

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

export function checkPublicApi(rustdocJson, config) {
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
