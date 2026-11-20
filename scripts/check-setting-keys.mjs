import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const admin = readFileSync('web/src/lib/components/AdminPage.svelte', 'utf8');
const keys = [...new Set([...admin.matchAll(/key:\s*'([^']+)'/g)].map((match) => match[1]))].sort();
if (keys.length === 0) {
  console.error('check-setting-keys: no setting keys found in AdminPage.svelte');
  process.exit(1);
}

const rustSources = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      walk(path);
    } else if (name.endsWith('.rs') && name !== 'config.rs') {
      rustSources.push(readFileSync(path, 'utf8'));
    }
  }
};
walk('src');

const missing = keys.filter((key) => {
  const leaf = key.split('.').pop();
  if (!leaf) return true;
  const pattern = new RegExp(`\\b${leaf.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`);
  return !rustSources.some((source) => pattern.test(source));
});

if (missing.length > 0) {
  console.error('check-setting-keys: settings exposed in the admin UI but never read by the server:');
  for (const key of missing) console.error(` - ${key}`);
  process.exit(1);
}

console.log(`check-setting-keys: ${keys.length} keys referenced`);
