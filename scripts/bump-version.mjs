import { readFileSync, writeFileSync } from 'node:fs';

const root = new URL('..', import.meta.url);
const cargoPath = new URL('Cargo.toml', root);
const lockPath = new URL('Cargo.lock', root);
const packagePath = new URL('web/package.json', root);

const cargo = readFileSync(cargoPath, 'utf8');
const packageJson = JSON.parse(readFileSync(packagePath, 'utf8'));
const current = String(packageJson.version ?? '0.1.0');
const [major, minor] = current.split('.').map(Number);
const next = `${major}.${minor + 1}.0`;

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
