import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash, generateKeyPairSync, sign, type KeyObject } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { isSignedKey, Licenses, makeLicenseKey } from '../src/licenses.js';
import { channels, readSigned, Updates } from '../src/updates.js';
import { createUpdateService } from '../src/server.js';
import { KeyRegistry } from '../src/key-registry.js';

const pair = () => generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
const pem = (k: KeyObject) => k.export({ type: 'spki', format: 'pem' }).toString();
const Upload = 'upload-key-0123456789';

function build(app: string, n: number, exe: Buffer, key: KeyObject) {
  const manifest = Buffer.from(JSON.stringify({ app, build: n, commit: 'abc1234', message: 'Faster knives', sha256: createHash('sha256').update(exe).digest('hex'), size: exe.length, builtAt: '2026-11-04T03:00:00Z' }));
  return { manifest: manifest.toString('base64'), signature: sign('sha256', manifest, key).toString('base64') };
}

test('tester keys: made with the secret, typed loosely, revocable, useless with another secret or app', async () => {
  const key = makeLicenseKey('secret-one');
  assert.match(key, /^STICK(-[0-9A-HJKMNP-TV-Z]{5}){4}$/);
  assert.equal(isSignedKey(key, 'secret-one'), true);
  assert.equal(isSignedKey(key, 'secret-two'), false);
  // Lowercase, no dashes, and O/I typed for 0/1 still match.
  assert.equal(isSignedKey(key.toLowerCase().replace(/-/g, ' ').replace(/0/g, 'o').replace(/1/g, 'I'), 'secret-one'), true);
  const tampered = key.slice(0, -1) + (key.endsWith('Z') ? 'Y' : 'Z');
  assert.equal(isSignedKey(tampered, 'secret-one'), false);
  // A khovdesk key made with the very same secret still doesn't work here (different HMAC domain).
  assert.equal(isSignedKey(key.replace('STICK', 'KHOV'), 'secret-one'), false);

  const other = makeLicenseKey('secret-one');
  const licenses = new Licenses({ secret: 'secret-one', revokedKeys: [other.toLowerCase()] });
  assert.equal(await licenses.check(key), 'valid');
  assert.equal(await licenses.check(tampered), 'invalid');
  assert.equal(await licenses.check(other), 'invalid'); // revoked
  assert.equal(await licenses.check('../etc/passwd'), 'invalid');
  assert.equal(await new Licenses({}).check(key), 'invalid'); // no secret: no keys work
  // Twenty wrong keys an hour from one address, then it waits.
  for (let i = 0; i < 20; i++) assert.equal(await licenses.check(`WrongKey${i}`, '1.2.3.4'), 'invalid');
  assert.equal(await licenses.check(key, '1.2.3.4'), 'limited');
  assert.equal(await licenses.check(key, '5.6.7.8'), 'valid');
});

test('channels: only builds signed with that channel\'s own key, matching and newer, are stored', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'stickfell-updates-'));
  const game = pair(), launcher = pair();
  process.env.STICKFELL_GAME_PUBLIC_KEY = pem(game.publicKey);
  process.env.STICKFELL_LAUNCHER_PUBLIC_KEY = pem(launcher.publicKey);
  try {
    const updates = new Updates(dir, Upload, channels.game);
    assert.equal(updates.uploadAllowed(`Bearer ${Upload}`), true);
    assert.equal(updates.uploadAllowed('Bearer nope'), false);
    assert.equal(new Updates(dir, undefined, channels.game).uploadAllowed('Bearer anything'), false);
    assert.equal((await updates.latest()).available, false);

    const exe = Buffer.from('MZ fake stickfell exe');
    const v2 = build('stickfell-game', 2, exe, game.privateKey);
    assert.equal((await updates.upload(exe, v2.manifest, v2.signature)).build, 2);
    const latest = await updates.latest();
    assert.equal(readSigned(latest.manifest!, latest.signature!, pem(game.publicKey), 'stickfell-game').commit, 'abc1234');

    // The launcher's key can't publish a game build, nor a game manifest go on the launcher channel.
    const crossed = build('stickfell-game', 3, exe, launcher.privateKey);
    await assert.rejects(updates.upload(exe, crossed.manifest, crossed.signature), /signature/);
    const wrongApp = build('stickfell-launcher', 3, exe, game.privateKey);
    await assert.rejects(updates.upload(exe, wrongApp.manifest, wrongApp.signature), /malformed/);
    const v3 = build('stickfell-game', 3, exe, game.privateKey);
    await assert.rejects(updates.upload(Buffer.from('MZ something else!'), v3.manifest, v3.signature), /doesn't match its manifest/);
    const old = build('stickfell-game', 1, exe, game.privateKey);
    await assert.rejects(updates.upload(exe, old.manifest, old.signature), /isn't newer/);
    assert.equal((await updates.upload(exe, v3.manifest, v3.signature)).build, 3);
  } finally {
    delete process.env.STICKFELL_GAME_PUBLIC_KEY;
    delete process.env.STICKFELL_LAUNCHER_PUBLIC_KEY;
    await rm(dir, { recursive: true, force: true });
  }
});

test('update service: latest is public, downloading needs a tester key, uploading the upload key', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'stickfell-service-'));
  const game = pair(), launcher = pair();
  process.env.STICKFELL_GAME_PUBLIC_KEY = pem(game.publicKey);
  process.env.STICKFELL_LAUNCHER_PUBLIC_KEY = pem(launcher.publicKey);
  const tester = makeLicenseKey('secret-one');
  const licenses = new Licenses({ secret: 'secret-one' });
  const updates = { launcher: new Updates(dir, Upload, channels.launcher), game: new Updates(dir, Upload, channels.game) };
  const server = createUpdateService(updates, licenses);
  await new Promise<void>(r => server.listen(0, r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const get = (path: string, key?: string) => fetch(base + path, { headers: key ? { authorization: `Bearer ${key}` } : {} });
  const put = (channel: string, body: Buffer, b: { manifest: string; signature: string }, key = Upload) => fetch(`${base}/v1/${channel}/upload`, {
    method: 'PUT', body, headers: { authorization: `Bearer ${key}`, 'x-stickfell-manifest': b.manifest, 'x-stickfell-signature': b.signature },
  });
  try {
    assert.equal((await (await get('/v1/game/latest')).json()).available, false);
    const exe = Buffer.from('MZ stickfell build');
    const g = build('stickfell-game', 5, exe, game.privateKey);
    assert.equal((await put('game', exe, g, 'wrong')).status, 401);
    assert.equal((await put('game', exe, g)).status, 200);
    // The game's build can't go on the launcher channel.
    assert.equal((await put('launcher', exe, g)).status, 400);
    const l = build('stickfell-launcher', 1, exe, launcher.privateKey);
    assert.equal((await put('launcher', exe, l)).status, 200);

    assert.equal((await (await get('/v1/game/latest')).json()).manifest, g.manifest);
    assert.equal((await (await get('/v1/launcher/latest')).json()).manifest, l.manifest);
    assert.equal((await get('/v1/game/download')).status, 401);
    assert.equal((await get('/v1/game/download', 'STICK-NOPE0-NOPE0-NOPE0-NOPE0')).status, 403);
    assert.equal((await get('/v1/license', tester)).status, 200);
    assert.equal((await get('/v1/license', makeLicenseKey('another-secret'))).status, 403);
    const download = await get('/v1/game/download', tester.toLowerCase());
    assert.equal(download.status, 200);
    assert.deepEqual(Buffer.from(await download.arrayBuffer()), exe);
    assert.equal((await get('/v1/launcher/download', tester)).headers.get('content-disposition'), 'attachment; filename="Stickfell.exe"');
    // The publisher's upload key downloads too.
    assert.equal((await get('/v1/game/download', Upload)).status, 200);
    assert.equal((await get('/v1/nope/latest')).status, 404);
    assert.equal((await get('/v1/__proto__/latest')).status, 404);
    assert.equal((await get('/v1/game/../../etc/passwd')).status, 404);
  } finally {
    server.close();
    delete process.env.STICKFELL_GAME_PUBLIC_KEY;
    delete process.env.STICKFELL_LAUNCHER_PUBLIC_KEY;
    await rm(dir, { recursive: true, force: true });
  }
});

test('key list: made and imported keys, uses and addresses, revoking and restoring, the admin key', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'stickfell-keys-'));
  const registry = new KeyRegistry(dir);
  const licenses = new Licenses({ secret: 'secret-one', registry });
  const updates = { launcher: new Updates(dir, undefined, channels.launcher), game: new Updates(dir, undefined, channels.game) };
  const server = createUpdateService(updates, licenses, { key: 'admin-key-0123456789', registry, secret: 'secret-one' });
  await new Promise<void>(r => server.listen(0, r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const admin = (path: string, body?: unknown, key = 'admin-key-0123456789') => fetch(base + path, {
    method: body ? 'POST' : 'GET', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' }, ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const list = async () => (await (await admin('/admin/keys')).json()).keys as { id: string; label: string; kind: string; uses: number; addresses: number; revokedAt?: number; events: { type: string }[] }[];
  try {
    assert.equal((await admin('/admin/keys', undefined, 'wrong')).status, 401);
    const made: string[] = (await (await admin('/admin/keys', { count: 3, batch: 'Playtest 1' })).json()).keys;
    assert.equal(made.length, 3);
    assert.ok(made.every(k => isSignedKey(k, 'secret-one')));
    const offline = makeLicenseKey('secret-one');
    assert.deepEqual(await (await admin('/admin/keys/import', { keys: [offline, made[0], 'STICK-NOPE'], batch: 'Old batch' })).json(), { added: 1, already: 1, invalid: 1 });
    assert.equal((await list()).length, 4);

    // Using a key activates it; a second address shows up; addresses are stored hashed.
    assert.equal(await licenses.check(made[0], '1.1.1.1'), 'valid');
    assert.equal(await licenses.check(made[0], '1.1.1.1'), 'valid');
    assert.equal(await licenses.check(made[0].toLowerCase(), '2.2.2.2'), 'valid');
    const first = (await list()).find(k => k.label === made[0])!;
    assert.equal(first.uses, 3);
    assert.equal(first.addresses, 2);
    assert.deepEqual(first.events.map(e => e.type), ['issued', 'activated', 'new-address']);
    assert.ok(!(await readFile(join(dir, 'keys.json'), 'utf8')).includes('1.1.1.1'));

    // Revoked keys stop working (by id, or by the key itself, even one never seen), and work again when restored.
    assert.equal((await (await admin('/admin/keys/revoke', { id: first.id, reason: 'Leaked' })).json()).key.reason, 'Leaked');
    assert.equal(await licenses.check(made[0]), 'invalid');
    const unseen = makeLicenseKey('secret-one');
    assert.equal((await admin('/admin/keys/revoke', { key: unseen })).status, 200);
    assert.equal(await licenses.check(unseen), 'invalid');
    await admin('/admin/keys/restore', { id: first.id });
    assert.equal(await licenses.check(made[0]), 'valid');
    assert.equal((await new KeyRegistry(dir).list()).length, 5);

    // Without an admin key the key list is off.
    const off = createUpdateService(updates, licenses);
    await new Promise<void>(r => off.listen(0, r));
    assert.equal((await fetch(`http://127.0.0.1:${(off.address() as AddressInfo).port}/admin/keys`)).status, 404);
    off.close();
  } finally {
    server.close();
    await rm(dir, { recursive: true, force: true });
  }
});
