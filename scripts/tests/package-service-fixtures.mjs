import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function writePackageRoot(root, {
  packageId = 'example.com/provider',
  services = [],
  api = 'health: main.health\n',
  source = 'function health() -> string { return "ok" }\n',
} = {}) {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'package.yml'), `${JSON.stringify({
    id: packageId,
    version: '1.0.0',
    ...(services.length === 0 ? {} : { services }),
  }, null, 2)}\n`);
  await writeFile(join(root, 'api.yml'), api);
  await writeFile(join(root, 'main.skiff'), source);
}


export function contractCoordinate(alias = 'health') {
  return {
    alias,
    id: 'example.com/health',
    version: '1.0.0',
  };
}
