// Makes a channel's update-signing key pair, once (the launcher and the game each have their own):
//   node scripts/signing-key.mjs <launcher|game>
// The public half goes in the repo (keys/<channel>-signing-key.pub): the launcher is built with it, and the update
// service checks uploads against it. The private half is saved to %USERPROFILE%\.stickfell\<channel>-signing-key.pem
// (never into the repo): paste that file's contents into the channel's GitHub Environment as STICKFELL_SIGNING_KEY,
// then keep the file somewhere safe offline.
import { generateKeyPairSync } from 'node:crypto';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const channel = process.argv[2];
if (channel !== 'launcher' && channel !== 'game') { console.error('Say which channel: launcher or game.'); process.exit(1); }
const pub = fileURLToPath(new URL(`../../keys/${channel}-signing-key.pub`, import.meta.url));
const priv = join(homedir(), '.stickfell', `${channel}-signing-key.pem`);
if (existsSync(pub) || existsSync(priv)) {
  console.error(`A ${channel} key already exists (${existsSync(pub) ? pub : priv}). Builds signed with it would stop installing if it changed.`);
  process.exit(1);
}
const { publicKey, privateKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
mkdirSync(join(homedir(), '.stickfell'), { recursive: true });
writeFileSync(priv, privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
writeFileSync(pub, publicKey.export({ type: 'spki', format: 'pem' }));
console.error(`Public key: ${pub} (commit it)\nPrivate key: ${priv}\nPaste the private key file's contents into the "${channel}-release" GitHub Environment as STICKFELL_SIGNING_KEY.`);
