#!/usr/bin/env node

import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { captureAttachedCommand } from './lib/command-execution.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffCli = join(scriptDir, 'skiff.mjs');
const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-package-store-discovery-'));

try {
  const artifactRoot = join(tempRoot, 'artifacts');
  const dependencyRoot = join(tempRoot, 'dependency');
  const consumerRoot = join(tempRoot, 'consumer');
  await writeDependency(dependencyRoot);
  await writeConsumer(consumerRoot);

  const dependency = await runSkiffJson([
    'package', 'publish', dependencyRoot, '--artifact-root', artifactRoot, '--json',
  ]);
  if (!dependency.packageArtifactReceipt || !dependency.packagePointerReceipt) {
    throw new Error(`package publish did not return separated typed receipts: ${JSON.stringify(dependency)}`);
  }

  const consumer = await runSkiffJson([
    'package', 'build', consumerRoot, '--artifact-root', artifactRoot, '--json',
  ]);
  const record = JSON.parse(await readFile(
    join(artifactRoot, consumer.packageArtifactReceipt.recordPath),
    'utf8',
  ));
  if (record.packageRequirements.length !== 1
      || record.packageRequirements[0].packageId !== 'google.com/cloud'
      || record.packageRequirements[0].alias !== 'gcloud') {
    throw new Error(`consumer did not resolve the published typed package pointer: ${JSON.stringify(record.packageRequirements)}`);
  }

  await runSkiffFailure([
    'package', 'build', consumerRoot,
    '--artifact-root', join(tempRoot, 'missing-artifacts'),
    '--json',
  ], 'no published PackageArtifact pointer');
  await runSkiffFailure([
    'package', 'build', consumerRoot,
    '--artifact-root', artifactRoot,
    '--packages-dir', join(tempRoot, 'legacy-source-store'),
  ], 'unknown option --packages-dir');

  await checkCanonicalDevRegistry();
  console.log('Package store discovery check passed.');
} finally {
  await rm(tempRoot, { force: true, recursive: true });
}

async function checkCanonicalDevRegistry() {
  const registryPath = join(tempRoot, 'watch.json');
  const contractRoot = join(tempRoot, 'contract');
  const deploymentRoot = join(tempRoot, 'deployment');
  await mkdir(contractRoot, { recursive: true });
  await mkdir(deploymentRoot, { recursive: true });
  await writeFile(join(contractRoot, 'contract.yml'), '{}\n');
  await writeFile(join(deploymentRoot, 'deployment.yml'), '{}\n');
  for (const root of [join(tempRoot, 'consumer'), contractRoot, deploymentRoot]) {
    await runSkiff([
      'dev', 'registry', 'add', root,
      '--config', registryPath,
      '--environment', 'checker',
    ]);
  }
  const registry = JSON.parse(await readFile(registryPath, 'utf8'));
  const kinds = registry.roots.map(({ kind }) => kind).sort();
  if (registry.schemaVersion !== 'skiff-package-service-dev-registry-v1'
      || registry.environment !== 'checker'
      || JSON.stringify(kinds) !== JSON.stringify(['contract', 'deployment', 'package'])
      || Object.hasOwn(registry, 'services')) {
    throw new Error(`dev registry is not the canonical root registry: ${JSON.stringify(registry)}`);
  }
}

async function writeDependency(root) {
  await mkdir(join(root, 'cloud'), { recursive: true });
  await writeFile(join(root, 'package.yml'), 'id: google.com/cloud\nversion: 1.0.0\n');
  await writeFile(join(root, 'api.yml'), 'storage:\n  upload: cloud.storage.upload\n');
  await writeFile(
    join(root, 'cloud', 'storage.skiff'),
    'function upload() -> string { return "ok" }\n',
  );
}

async function writeConsumer(root) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'package.yml'), [
    'id: example.com/facade',
    'version: 1.0.0',
    'packages:',
    '  - id: google.com/cloud',
    '    version: 1.0.0',
    '    alias: gcloud',
    '',
  ].join('\n'));
  await writeFile(join(root, 'api.yml'), 'facade: facade.facade\n');
  await writeFile(join(root, 'facade.skiff'), [
    'import gcloud',
    'function facade() -> string { return gcloud/storage.upload() }',
    '',
  ].join('\n'));
}

async function runSkiffJson(args) {
  const result = await runSkiff(args);
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`skiff returned invalid JSON: ${error.message}\n${result.stdout}`);
  }
}

async function runSkiff(args) {
  const outcome = await captureAttachedCommand(process.execPath, [skiffCli, ...args], {
    cwd: tempRoot,
  });
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw new Error([
      `skiff ${args.join(' ')} failed with ${outcome.signal ?? outcome.code}`,
      outcome.stderr.trim(),
      outcome.stdout.trim(),
    ].filter(Boolean).join('\n'));
  }
  return outcome;
}

async function runSkiffFailure(args, expected) {
  const outcome = await captureAttachedCommand(process.execPath, [skiffCli, ...args], {
    cwd: tempRoot,
  });
  if (outcome.code === 0) {
    throw new Error(`skiff ${args.join(' ')} unexpectedly succeeded`);
  }
  const output = `${outcome.stderr}\n${outcome.stdout}`;
  if (!output.includes(expected)) {
    throw new Error(`expected ${JSON.stringify(expected)} in failure output:\n${output}`);
  }
}
