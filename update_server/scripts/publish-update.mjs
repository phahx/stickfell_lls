// Signs a Stickfell build and uploads it to the update service (run by the release workflows in .github/workflows):
//   node scripts/publish-update.mjs <path to the exe> <launcher|game>
// Needs STICKFELL_SIGNING_KEY (that channel's private key, PEM), STICKFELL_UPLOAD_KEY, STICKFELL_UPDATE_URL (the
// service's address), BUILD_NUMBER and COMMIT; the patch notes come from NOTES, else COMMIT_MESSAGE's first line. The
// service may still be redeploying from the same push, so the upload is retried for up to 15 minutes.
import { createHash, createPrivateKey, sign } from 'node:crypto';
import { readFileSync } from 'node:fs';

const [exePath, channel] = process.argv.slice(2);
if (!exePath || (channel !== 'launcher' && channel !== 'game')) { console.error('Usage: publish-update.mjs <exe> <launcher|game>'); process.exit(1); }
const need = name => { const v = process.env[name]?.trim(); if (!v) { console.error(`Set ${name}`); process.exit(1); } return v; };
const key = createPrivateKey(need('STICKFELL_SIGNING_KEY'));
const exe = readFileSync(exePath);
const notes = (process.env.NOTES?.trim() || (process.env.COMMIT_MESSAGE ?? '').split('\n')[0]).slice(0, 500);
const manifest = {
  app: `stickfell-${channel}`,
  build: Number(need('BUILD_NUMBER')),
  commit: need('COMMIT'),
  message: notes,
  sha256: createHash('sha256').update(exe).digest('hex'),
  size: exe.length,
  builtAt: new Date().toISOString(),
};
const bytes = Buffer.from(JSON.stringify(manifest));
const signature = sign('sha256', bytes, key).toString('base64');

let base = need('STICKFELL_UPDATE_URL').replace(/\/+$/, '');
if (!/^https?:\/\//i.test(base)) base = `https://${base}`;
const url = new URL(`/v1/${channel}/upload`, base).href;

const until = Date.now() + 15 * 60_000;
for (let attempt = 1; ; attempt++) {
  try {
    const res = await fetch(url, {
      method: 'PUT', body: exe,
      headers: { authorization: `Bearer ${need('STICKFELL_UPLOAD_KEY')}`, 'content-type': 'application/octet-stream', 'x-stickfell-manifest': bytes.toString('base64'), 'x-stickfell-signature': signature },
    });
    const text = await res.text();
    if (res.ok) { console.log(`${channel}: uploaded build ${manifest.build} (${manifest.commit.slice(0, 7)}, ${(exe.length / 1048576).toFixed(1)} MB)`); process.exit(0); }
    // 404: the service doesn't have this endpoint yet (still deploying). Other refusals are final.
    if (res.status !== 404 && res.status < 500) { console.error(`${channel}: upload refused (${res.status}): ${text}`); process.exit(1); }
    console.log(`attempt ${attempt}: answered ${res.status}; retrying`);
  } catch (e) { console.log(`attempt ${attempt}: ${e.message}; retrying`); }
  if (Date.now() > until) { console.error('Gave up: the update service never accepted the upload'); process.exit(1); }
  await new Promise(r => setTimeout(r, 30_000));
}
