import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const source = readFileSync('web/src/lib/i18n.ts', 'utf8');
const start = source.indexOf('const en: Record<string, string> = {');
const end = source.indexOf('\n};', start);
if (start < 0 || end < 0) {
  console.error('check-i18n: en dictionary not found');
  process.exit(1);
}

const keys = [];
for (const line of source.slice(start, end).split('\n').slice(1)) {
  const match = line.match(/^\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"|([^\s:]+))\s*:/);
  if (match) keys.push(match[1] ?? match[2] ?? match[3]);
}

const counts = new Map();
for (const key of keys) counts.set(key, (counts.get(key) ?? 0) + 1);
const duplicates = [...counts].filter(([, count]) => count > 1).map(([key]) => key);

const used = new Set();
const files = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const stat = statSync(path);
    if (stat.isDirectory()) walk(path);
    else if (/\.(svelte|ts)$/.test(name)) files.push(path);
  }
};
walk('web/src');

const call = /\bt\(\s*(?:[^,()]|\([^)]*\))*,\s*'((?:[^'\\]|\\.)*)'/g;
const callDouble = /\bt\(\s*(?:[^,()]|\([^)]*\))*,\s*"((?:[^"\\]|\\.)*)"/g;
for (const file of files) {
  const text = readFileSync(file, 'utf8');
  for (const match of text.matchAll(call)) used.add(match[1]);
  for (const match of text.matchAll(callDouble)) used.add(match[1]);
}

// Admin settings labels and hints are resolved through t() by the row snippets.
const admin = readFileSync('web/src/lib/components/AdminPage.svelte', 'utf8');
for (const match of admin.matchAll(/\b(?:label|hint):\s*'((?:[^'\\]|\\.)*)'/g)) {
  used.add(match[1]);
}
for (const match of admin.matchAll(/groupHeading\('((?:[^'\\]|\\.)*)'/g)) {
  used.add(match[1]);
}

const missing = [...used].filter((key) => !counts.has(key)).sort();
const unused = [...counts.keys()].filter((key) => !used.has(key)).sort();

const placeholders = (value) => new Set([...value.matchAll(/\{(\w+)\}/g)].map((m) => m[1]));
const mismatched = [];
const dictionary = new Map();
for (const line of source.slice(start, end).split('\n').slice(1)) {
  const match = line.match(
    /^\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"|([^\s:]+))\s*:\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)")/,
  );
  if (match) dictionary.set(match[1] ?? match[2] ?? match[3], match[4] ?? match[5] ?? '');
}
for (const [key, value] of dictionary) {
  const kp = placeholders(key);
  const vp = placeholders(value);
  if (kp.size !== vp.size || [...kp].some((name) => !vp.has(name))) {
    mismatched.push(key);
  }
}

let failed = false;
if (missing.length > 0) {
  failed = true;
  console.error(`check-i18n: ${missing.length} keys are used but missing from the English dictionary:`);
  for (const key of missing) console.error(` - ${key}`);
}
if (duplicates.length > 0) {
  failed = true;
  console.error(`check-i18n: duplicate keys: ${duplicates.join(', ')}`);
}
if (mismatched.length > 0) {
  failed = true;
  console.error(`check-i18n: placeholder mismatch: ${mismatched.join(', ')}`);
}
if (unused.length > 0) {
  console.warn(`check-i18n: ${unused.length} dictionary keys are currently unused`);
}
if (failed) process.exit(1);
console.log(`check-i18n: ${used.size} keys covered (${unused.length} unused)`);
