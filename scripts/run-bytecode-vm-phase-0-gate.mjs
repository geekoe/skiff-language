#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDir, '..');
const manifestSchema = 'skiff-vcp-phase-0-v1';

async function main() {
  try {
    const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-bytecode-vm-phase-0-'));
    const manifestPath = join(tempRoot, 'vcp-phase-0-evidence.json');
    // child-process-owner: bytecode-vm-phase-0-gate
    const commit = execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    const env = {
      ...process.env,
      SKIFF_VCP_PHASE0_MANIFEST: manifestPath,
      SKIFF_VCP_PHASE0_COMMIT: commit,
    };
    try {
      // child-process-owner: bytecode-vm-phase-0-gate
      execFileSync(
        'cargo',
        [
          'test',
          '--quiet',
          '--manifest-path',
          join(root, 'runtime', 'request', 'Cargo.toml'),
          '--test',
          'bytecode_vm_phase_0_vcp',
          '--',
          '--nocapture',
        ],
        {
          cwd: root,
          env,
          encoding: 'utf8',
          stdio: ['ignore', 'pipe', 'pipe'],
        },
      );
    } catch (error) {
      throw new Error(`VCP cargo test failed\n${error?.stdout ?? ''}\n${error?.stderr ?? ''}`);
    }

    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    validateManifest(manifest, commit);
    console.log(`[bytecode-vm-phase-0] VCP manifest validated: ${manifestPath}`);
    console.log(
      `[bytecode-vm-phase-0] scenarios=${manifest.counts.total} passed=${manifest.counts.passed} skipped=${manifest.counts.skipped}`,
    );
    await rm(tempRoot, { recursive: true, force: true });
  } catch (error) {
    console.error(error?.stack ?? error);
    process.exitCode = 1;
  }
}

await main();

function validateManifest(manifest, commit) {
  if (manifest?.schemaVersion !== manifestSchema) {
    throw new Error(`manifest schemaVersion must be ${manifestSchema}`);
  }
  if (manifest?.candidate?.commit !== commit) {
    throw new Error('manifest candidate commit does not match current HEAD');
  }
  if (manifest?.result !== 'pass') {
    throw new Error(`manifest result must be pass, got ${manifest?.result}`);
  }
  const counts = manifest?.counts;
  if (
    !counts
    || !Number.isInteger(counts.total)
    || counts.total < 4
    || counts.passed !== counts.total
    || counts.failed !== 0
    || counts.skipped !== 0
  ) {
    throw new Error('manifest counts must show at least 4 non-skipped passing scenarios');
  }
  if (!Array.isArray(manifest?.scenarios) || manifest.scenarios.length !== counts.total) {
    throw new Error('manifest scenarios must match counts.total');
  }
  if (manifest.scenarios.some((scenario) => scenario?.status !== 'pass')) {
    throw new Error('all VCP scenarios must pass');
  }
  if (manifest?.composition?.bypassCount !== 0 || manifest?.composition?.fallbackCount !== 0) {
    throw new Error('manifest must prove zero bypass and fallback paths');
  }
  if (
    !manifest?.artifactStore?.packageBuildId
    || !manifest?.artifactStore?.bytecodeIdentity
    || !manifest?.artifactStore?.deploymentArtifactIdentity
  ) {
    throw new Error('manifest artifactStore identities are incomplete');
  }
  if (!/^[a-f0-9]{64}$/.test(manifest?.binaryIdentities?.harnessSha256 ?? '')) {
    throw new Error('manifest harness sha256 must be a 64-character hex digest');
  }
}
