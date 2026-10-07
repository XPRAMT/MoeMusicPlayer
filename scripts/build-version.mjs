// @ts-nocheck
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export function buildVersion(date = new Date()) {
  return `${date.getFullYear() % 100}.${date.getMonth() + 1}.${date.getDate()}`;
}

export function moeBuildDefine(date = new Date()) {
  const version = buildVersion(date);
  const versionFile = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'build-version.txt');
  writeFileSync(versionFile, `${version}\n`, 'utf8');
  return {
    __MOE_BUILD_VERSION__: JSON.stringify(version),
  };
}
