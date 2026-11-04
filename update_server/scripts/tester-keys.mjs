// Makes Stickfell tester keys (STICK-XXXXX-XXXXX-XXXXX-XXXXX) offline. The keys app makes them too (and lists them);
// keys made here can be added to its list with Import keys. The update service accepts them when its
// STICKFELL_LICENSE_SECRET is the same secret.
//   node scripts/tester-keys.mjs <count> [--new-secret]
//   node scripts/tester-keys.mjs --check <key> [<key> ...]   says whether each key was made with this secret
// The secret is read from STICKFELL_LICENSE_SECRET, or from %USERPROFILE%\.stickfell\license-secret.txt; --new-secret
// makes that file (once) and prints the value to set on the update service. Run `npm run build` first.
import { randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { isSignedKey, makeLicenseKey } from '../dist/licenses.js';

const file = join(homedir(), '.stickfell', 'license-secret.txt');
const args = process.argv.slice(2);
if (args.includes('--new-secret')) {
  if (existsSync(file)) { console.error(`${file} already exists; keys made with it would stop working if it changed.`); process.exit(1); }
  mkdirSync(join(homedir(), '.stickfell'), { recursive: true });
  const secret = randomBytes(32).toString('base64url');
  writeFileSync(file, secret, { mode: 0o600 });
  console.error(`Saved a new secret to ${file}. Set it on the update service as STICKFELL_LICENSE_SECRET.`);
  process.exit(0);
}
const secret = process.env.STICKFELL_LICENSE_SECRET?.trim() || (existsSync(file) ? readFileSync(file, 'utf8').trim() : '');
if (!secret) { console.error('No secret: set STICKFELL_LICENSE_SECRET or run with --new-secret first.'); process.exit(1); }
if (args[0] === '--check') {
  const given = args.slice(1);
  if (!given.length) { console.error('Give the keys to check.'); process.exit(1); }
  for (const key of given) console.log(`${key}  ${isSignedKey(key, secret) ? 'genuine' : 'NOT made with this secret'}`);
  process.exit(0);
}
const count = Number(args.find(a => /^\d+$/.test(a)) ?? 10);
if (!(count >= 1 && count <= 10_000)) { console.error('Give a count from 1 to 10000.'); process.exit(1); }
const keys = new Set();
while (keys.size < count) keys.add(makeLicenseKey(secret));
console.log([...keys].join('\n'));
