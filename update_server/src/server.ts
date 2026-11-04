import { createHash, timingSafeEqual } from 'node:crypto';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { isSignedKey, keyIdentity, Licenses, makeLicenseKey, signedKeyBody } from './licenses.js';
import { KeyRegistry } from './key-registry.js';
import { channels, type ChannelName, MaxExeBytes, readLimited, Updates } from './updates.js';

function send(res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}) {
  res.writeHead(status, { 'content-type': 'application/json', ...headers }).end(JSON.stringify(body));
}

const bearer = (header: string | undefined) => /^Bearer\s+(\S+)\s*$/i.exec(header ?? '')?.[1];
const digest = (s: string) => createHash('sha256').update(s).digest();

/** The keys app's side of the service: the key list, with STICKFELL_ADMIN_KEY as the bearer. */
export interface KeyAdmin {
  key?: string;
  registry: KeyRegistry;
  /** STICKFELL_LICENSE_SECRET, to make and import tester keys. */
  secret?: string;
}

/** Wrong admin keys one address may send an hour. */
const MaxAdminFailures = 10;
const MaxBatch = 1000;

async function readJson(req: IncomingMessage): Promise<Record<string, unknown>> {
  try {
    const value = JSON.parse((await readLimited(req, 1_000_000)).toString('utf8'));
    return value && typeof value === 'object' ? value : {};
  } catch { return {}; }
}

const refusals: Record<string, [number, string]> = {
  invalid: [403, 'That tester key isn\'t valid for Stickfell. Check it, or ask for a new one.'],
  limited: [429, 'Too many wrong tester keys from this address. Try again in an hour.'],
};

/**
 * The update service the launcher checks: GitHub Actions uploads each signed build with the upload key; anyone may
 * see which build is the latest (so the launcher can say there's an update); downloading it needs a tester key.
 *   GET  /v1/<launcher|game>/latest     the signed manifest
 *   GET  /v1/<launcher|game>/history    every kept build's signed manifest, newest first (patch notes)
 *   GET  /v1/<launcher|game>/download   the exe (tester key)
 *   PUT  /v1/<launcher|game>/upload     a signed build (upload key)
 *   GET  /v1/license                    whether a tester key works
 *   /admin/keys…                        the keys app (admin key)
 */
export function createUpdateService(updates: Record<ChannelName, Updates>, licenses: Licenses, admin?: KeyAdmin): Server {
  const adminFailures = new Map<string, number[]>();
  const uploader = updates.game;
  return createServer(async (req, res) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    const path = url.pathname;
    // The host's proxy appends the real address last (earlier entries come from the client and can be made up).
    const client = (req.headers['x-forwarded-for'] as string | undefined)?.split(',').pop()?.trim() || req.socket.remoteAddress || '';
    if (path === '/health') return send(res, 200, { ok: true });
    if (path.startsWith('/admin/')) {
      if (!admin?.key) return send(res, 404, { error: 'The key list is off: set STICKFELL_ADMIN_KEY on the update service' });
      const now = Date.now();
      const recent = (adminFailures.get(client) ?? []).filter(t => now - t < 3600_000);
      adminFailures.set(client, recent);
      if (recent.length >= MaxAdminFailures) return send(res, 429, { error: 'Too many wrong admin keys from this address. Try again in an hour.' });
      const token = bearer(req.headers.authorization);
      if (!token || !timingSafeEqual(digest(token), digest(admin.key))) { recent.push(now); return send(res, 401, { error: 'Wrong admin key' }); }
      return adminRoute(admin, path, req, res);
    }
    // Whether a tester key works (the launcher asks before saving one). The upload key works too.
    const checkKey = async (): Promise<boolean> => {
      const key = bearer(req.headers.authorization);
      const result = !key ? 'missing' : uploader.uploadAllowed(req.headers.authorization) ? 'valid' : await licenses.check(key, client);
      if (result === 'missing') { send(res, 401, { error: 'Send your tester key' }, { 'www-authenticate': 'Bearer' }); return false; }
      if (result !== 'valid') { const [status, error] = refusals[result]; send(res, status, { error }); return false; }
      return true;
    };
    if (path === '/v1/license') {
      if (req.method !== 'GET') return send(res, 405, { error: 'Use GET' }, { allow: 'GET' });
      return await checkKey() ? send(res, 200, { valid: true }) : undefined;
    }
    const route = /^\/v1\/([a-z]+)\/(latest|history|download|upload)$/.exec(path);
    const channel = route && Object.hasOwn(updates, route[1]) ? updates[route[1] as ChannelName] : undefined;
    if (!route || !channel) return send(res, 404, { error: 'Not found' });
    if (route[2] === 'upload') {
      if (req.method !== 'PUT') return send(res, 405, { error: 'Use PUT' }, { allow: 'PUT' });
      if (!channel.uploadAllowed(req.headers.authorization)) return send(res, 401, { error: 'Unauthorized' });
      const manifest = req.headers['x-stickfell-manifest'], signature = req.headers['x-stickfell-signature'];
      if (typeof manifest !== 'string' || typeof signature !== 'string') return send(res, 400, { error: 'Send x-stickfell-manifest and x-stickfell-signature' });
      let body: Buffer;
      try { body = await readLimited(req, MaxExeBytes); } catch { return send(res, 413, { error: 'Too large' }, { connection: 'close' }); }
      try { return send(res, 200, { stored: await channel.upload(body, manifest, signature) }); }
      catch (e) { return send(res, 400, { error: (e as Error).message }); }
    }
    if (req.method !== 'GET') return send(res, 405, { error: 'Use GET' }, { allow: 'GET' });
    if (route[2] === 'latest') return send(res, 200, await channel.latest());
    if (route[2] === 'history') return send(res, 200, { builds: await channel.history() });
    if (await checkKey()) await channel.download(res);
  });
}

/** The key list: GET /admin/keys, and POST to make, import, revoke and restore keys. */
async function adminRoute(admin: KeyAdmin, path: string, req: IncomingMessage, res: ServerResponse) {
  const { registry, secret } = admin;
  if (path === '/admin/keys' && req.method === 'GET')
    return send(res, 200, { keys: await registry.list(), signing: Boolean(secret), itch: false });
  if (req.method !== 'POST') return send(res, 405, { error: 'Use POST' }, { allow: 'POST' });
  const body = await readJson(req);
  const batch = typeof body.batch === 'string' ? body.batch.trim().slice(0, 100) : '';
  if (path === '/admin/keys') {
    if (!secret) return send(res, 409, { error: 'Set STICKFELL_LICENSE_SECRET on the update service to make keys' });
    const count = Number(body.count);
    if (!Number.isInteger(count) || count < 1 || count > MaxBatch) return send(res, 400, { error: `Make 1 to ${MaxBatch} keys at a time` });
    const keys = new Set<string>();
    while (keys.size < count) keys.add(makeLicenseKey(secret));
    const made = [...keys];
    await registry.issue(made.map(key => ({ id: signedKeyBody(key)!, key })), batch || new Date().toISOString().slice(0, 10));
    return send(res, 200, { keys: made });
  }
  if (path === '/admin/keys/import') {
    if (!secret) return send(res, 409, { error: 'Set STICKFELL_LICENSE_SECRET on the update service to import keys' });
    const given = Array.isArray(body.keys) ? body.keys.filter((k): k is string => typeof k === 'string' && k.trim() !== '') : [];
    if (given.length > 10 * MaxBatch) return send(res, 400, { error: `Import up to ${10 * MaxBatch} keys at a time` });
    const genuine = given.filter(k => isSignedKey(k, secret)).map(k => keyIdentity(k));
    const added = await registry.issue(genuine.map(k => ({ id: k.id, key: k.label })), batch || 'Imported', true);
    return send(res, 200, { added: added.length, already: genuine.length - added.length, invalid: given.length - genuine.length });
  }
  if (path === '/admin/keys/revoke') {
    const reason = typeof body.reason === 'string' ? body.reason.trim().slice(0, 200) : '';
    if (typeof body.id === 'string') {
      if (!await registry.get(body.id)) return send(res, 404, { error: 'No such key' });
      return send(res, 200, { key: await registry.revoke(body.id, reason, { kind: 'signed', label: '' }) });
    }
    if (typeof body.key !== 'string' || !body.key.trim()) return send(res, 400, { error: 'Send the key (or its id) to revoke' });
    // A key that was never used can be revoked ahead of time (one handed out by mistake).
    const who = keyIdentity(body.key);
    return send(res, 200, { key: await registry.revoke(who.id, reason, { kind: who.signed ? 'signed' : 'tester', label: who.label, ...(who.signed ? { key: who.label } : {}) }) });
  }
  if (path === '/admin/keys/restore') {
    const key = typeof body.id === 'string' ? await registry.restore(body.id) : undefined;
    return key ? send(res, 200, { key }) : send(res, 404, { error: 'No such key' });
  }
  return send(res, 404, { error: 'Not found' });
}

/** Where the service keeps builds and the key list: STICKFELL_DATA_DIR, else the Railway volume, else beside it. */
export function dataDirectory(env: Record<string, string | undefined> = process.env): string {
  const dir = env.STICKFELL_DATA_DIR?.trim() || env.RAILWAY_VOLUME_MOUNT_PATH?.trim();
  return dir ? resolve(dir) : fileURLToPath(new URL('../.data/', import.meta.url));
}

// Run directly (node dist/server.js): the update service.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const env = process.env;
  const directory = dataDirectory();
  const list = (value?: string) => (value ?? '').split(/[\n,]+/).map(s => s.trim()).filter(Boolean);
  const secret = env.STICKFELL_LICENSE_SECRET?.trim() || undefined;
  const upload = env.STICKFELL_UPLOAD_KEY?.trim() || undefined;
  const registry = new KeyRegistry(directory);
  const licenses = new Licenses({ secret, revokedKeys: list(env.STICKFELL_REVOKED_KEYS), registry });
  if (!upload) console.error('STICKFELL_UPLOAD_KEY isn\'t set: no builds can be uploaded');
  if (!secret) console.error('STICKFELL_LICENSE_SECRET isn\'t set: no tester key will work');
  if (!env.STICKFELL_ADMIN_KEY?.trim()) console.error('STICKFELL_ADMIN_KEY isn\'t set: the keys app can\'t connect');
  const admin = { key: env.STICKFELL_ADMIN_KEY?.trim() || undefined, registry, secret };
  const updates = { launcher: new Updates(directory, upload, channels.launcher), game: new Updates(directory, upload, channels.game) };
  const port = Number(env.PORT ?? 3000);
  createUpdateService(updates, licenses, admin).listen(port, () => console.error(`Stickfell update service on port ${port}`));
}
