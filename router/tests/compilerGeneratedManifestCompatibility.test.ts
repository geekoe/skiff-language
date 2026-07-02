import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { loadManifestFile } from '../src/manifest/loadManifest.js';
import type { SkiffRuntimeManifest } from '../src/manifest/types.js';

const execFileAsync = promisify(execFile);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const websocketFixturePath = join(
  repoRoot,
  'compiler/tests/fixtures/router-websocket-fixture'
);
const gatewayIdentityPrefix = 'skiff-gateway-v1:sha256:';
const operationAbiIdPattern = /^skiff-operation-abi-v1:sha256:[0-9a-f]{64}$/;

describe('Rust compiler generated manifest compatibility', () => {
  it(
    'loads a dynamically generated router manifest from a real websocket fixture',
    async () => {
      const tempDir = await mkdtemp(join(tmpdir(), 'skiff-router-manifest-compat-'));
      try {
        const artifactPath = join(tempDir, 'artifact.json');
        const manifestPath = join(tempDir, 'router-manifest.json');

        await execFileAsync(
          'cargo',
          [
            'run',
            '--quiet',
            '--manifest-path',
            join(repoRoot, 'compiler/Cargo.toml'),
            '--',
            websocketFixturePath,
            '--out',
            artifactPath,
            '--manifest-out',
            manifestPath
          ],
          { cwd: repoRoot }
        );

        const rawManifest = JSON.parse(
          await readFile(manifestPath, 'utf8')
        ) as SkiffRuntimeManifest;
        expect(
          rawManifest.gateway?.websocket?.connect?.gatewayEntryIdentity?.startsWith(
            gatewayIdentityPrefix
          )
        ).toBe(true);
        expect(
          rawManifest.gateway?.websocket?.receive.gatewayEntryIdentity?.startsWith(
            gatewayIdentityPrefix
          )
        ).toBe(true);
        expect(rawManifest.operations).toHaveLength(2);
        expect(
          rawManifest.operations.every(
            (operation) => operationAbiIdPattern.test(operation.operationAbiId)
          )
        ).toBe(true);

        const manifest = await loadManifestFile(manifestPath);
        const websocketEntry = manifest.websocketEntry;
        expect(websocketEntry).toBeDefined();
        if (!websocketEntry) {
          throw new Error('generated manifest did not load a websocket entry');
        }

        const connect = websocketEntry.connect;
        expect(connect).toBeDefined();
        if (!connect) {
          throw new Error('generated manifest did not load a websocket connect operation');
        }

        expect(manifest.operations).toHaveLength(2);
        expect(manifest.gateway?.http).toBeUndefined();
        expect(websocketEntry.id).toBe('client');
        expect(websocketEntry.path).toBeUndefined();
        expect(connect.operation).toBe('WebSocketFixtureService.connect');
        expect(websocketEntry.receive.operation).toBe('WebSocketFixtureService.receive');

        const rawConnectOperation = rawManifest.operations.find(
          (operation) => operation.operation === connect.operationManifest.operation
        );
        const rawReceiveOperation = rawManifest.operations.find(
          (operation) => operation.operation === websocketEntry.receive.operationManifest.operation
        );
        expect(rawConnectOperation).toBeDefined();
        expect(rawReceiveOperation).toBeDefined();
        if (!rawConnectOperation || !rawReceiveOperation) {
          throw new Error('generated websocket operations were not present in the raw manifest');
        }

        const connectOperationAbiId = connect.operationManifest.operationAbiId;
        const receiveOperationAbiId = websocketEntry.receive.operationManifest.operationAbiId;
        expect(connectOperationAbiId).toMatch(operationAbiIdPattern);
        expect(receiveOperationAbiId).toMatch(operationAbiIdPattern);
        expect(connectOperationAbiId).not.toBe(receiveOperationAbiId);
        expect(connectOperationAbiId).toBe(rawConnectOperation.operationAbiId);
        expect(receiveOperationAbiId).toBe(rawReceiveOperation.operationAbiId);
        expect(rawManifest.gateway?.websocket?.connect?.operationAbiId).toBe(connectOperationAbiId);
        expect(rawManifest.gateway?.websocket?.receive.operationAbiId).toBe(receiveOperationAbiId);
        expect(connect.operationAbiId).toBe(connectOperationAbiId);
        expect(websocketEntry.receive.operationAbiId).toBe(receiveOperationAbiId);
        expect(connect.gatewayEntryIdentity.startsWith(gatewayIdentityPrefix)).toBe(true);
        expect(websocketEntry.receive.gatewayEntryIdentity.startsWith(gatewayIdentityPrefix)).toBe(
          true
        );
        expect(connect.operationManifest.target).toMatch(
          /^entry\.[a-z0-9_~]+\.websocket\.connect$/
        );
        expect(websocketEntry.receive.operationManifest.target).toMatch(
          /^entry\.[a-z0-9_~]+\.websocket\.receive$/
        );
        expect(manifest.timeout?.defaultMs).toBe(120000);
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    },
    120_000
  );
});
