import { readFileSync, writeFileSync } from 'node:fs';

const args = process.argv.slice(2);
const mode = args.find((arg) => arg === '--patch' || arg === '--minor' || arg === '--major');
if (args.some((arg) => arg !== '--patch' && arg !== '--minor' && arg !== '--major')) {
  console.error('usage: node scripts/bump-version.mjs [--patch|--minor|--major] (default: minor)');
  process.exit(1);
}

const root = new URL('..', import.meta.url);
const cargoPath = new URL('Cargo.toml', root);
const lockPath = new URL('Cargo.lock', root);
const packagePath = new URL('web/package.json', root);

const cargo = readFileSync(cargoPath, 'utf8');
const packageJson = JSON.parse(readFileSync(packagePath, 'utf8'));
const current = String(packageJson.version ?? '0.1.0');
const [major, minor, patch] = current.split('.').map((part) => Number(part) || 0);

let next;
switch (mode) {
  case '--patch':
    next = `${major}.${minor}.${patch + 1}`;
    break;
  case '--major':
    next = `${major + 1}.0.0`;
    break;
  default:
    next = `${major}.${minor + 1}.0`;
    break;
}

if (next === current) throw new Error(`version did not change: ${current}`);

const cargoNext = cargo.replace(/^version = "[^"]+"/m, `version = "${next}"`);
if (cargoNext === cargo) throw new Error('Cargo.toml version not found');
writeFileSync(cargoPath, cargoNext);

const lock = readFileSync(lockPath, 'utf8');
const lockNext = lock.replace(/(\[\[package\]\]\nname = "filehub"\nversion = )"[^"]+"/, `$1"${next}"`);
if (lockNext === lock) throw new Error('Cargo.lock filehub version not found');
writeFileSync(lockPath, lockNext);

packageJson.version = next;
writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
console.log(`version ${current} -> ${next}`);
