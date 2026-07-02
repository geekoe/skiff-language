import { chmod, mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

export async function writeMockIdentityCli(input: {
  dir: string;
  dynamicBuildId?: string;
  stderrJson?: unknown;
  stdoutText?: string;
  exitCode?: number;
  capturePath?: string;
}): Promise<string> {
  const path = join(input.dir, process.platform === 'win32'
    ? 'skiff-artifact-identity.cmd'
    : 'skiff-artifact-identity');
  await mkdir(dirname(path), { recursive: true });
  const script = process.platform === 'win32'
    ? windowsScript(input)
    : posixScript(input);
  await writeFile(path, script);
  if (process.platform !== 'win32') {
    await chmod(path, 0o755);
  }
  return path;
}

function posixScript(input: {
  dynamicBuildId?: string;
  stderrJson?: unknown;
  stdoutText?: string;
  exitCode?: number;
  capturePath?: string;
}): string {
  return [
    '#!/usr/bin/env node',
    nodeScript(input),
    '',
  ].join('\n');
}

function windowsScript(input: {
  dynamicBuildId?: string;
  stderrJson?: unknown;
  stdoutText?: string;
  exitCode?: number;
  capturePath?: string;
}): string {
  return [
    '@echo off',
    'node -e ' + JSON.stringify(nodeScript(input)) + ' %*',
    '',
  ].join('\r\n');
}

function nodeScript(input: {
  dynamicBuildId?: string;
  stderrJson?: unknown;
  stdoutText?: string;
  exitCode?: number;
  capturePath?: string;
}): string {
  return `
const fs = require('node:fs');
const chunks = [];
process.stdin.on('data', (chunk) => chunks.push(chunk));
process.stdin.on('end', () => {
  const stdin = Buffer.concat(chunks).toString('utf8');
  ${input.capturePath !== undefined
    ? `fs.writeFileSync(${JSON.stringify(input.capturePath)}, stdin);`
    : ''}
  const exitCode = ${JSON.stringify(input.exitCode ?? 0)};
  if (exitCode !== 0) {
    process.stderr.write(${JSON.stringify(JSON.stringify(input.stderrJson ?? {
      error: { code: 'schema_invalid', message: 'mock identity error' },
    }))});
    process.exit(exitCode);
  }
  ${input.stdoutText !== undefined
    ? `process.stdout.write(${JSON.stringify(input.stdoutText)});`
    : `
  const payload = JSON.parse(stdin);
  process.stdout.write(JSON.stringify({
    results: payload.services.map((service) => ({
      key: service.key,
      dynamicBuildId: ${JSON.stringify(input.dynamicBuildId ??
        'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')},
    })),
  }));
  `}
});
`;
}
