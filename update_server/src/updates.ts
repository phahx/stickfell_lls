import { createHash, timingSafeEqual, verify } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Stickfell updates, on two separate channels: the launcher and the game. GitHub Actions builds each one, signs a
// small manifest (build number, commit, patch notes, SHA-256, size) with that channel's own key, which only lives in
// its GitHub Environment, and uploads both here with the upload key. The launcher checks the signature against the
// channel's public key built into it, downloads the exe and checks its hash. This server only stores and hands out
// what was signed: it checks every upload against the channel's public key itself, refuses an older build than the
// one it has, and can't make an update of its own. (Adapted from the khovdesk update service.)

export interface Manifest { app: string; build: number; commit: string; message?: string; sha256: string; size: number; builtAt: string }
interface Stored { manifest: string; signature: string; uploadedAt: string }

export const MaxExeBytes = 200 * 1024 * 1024;

/** A channel: the ID its manifests carry, the file it serves, and its own signing key. */
export interface Channel { name: string; app: string; file: string; keyEnv: string; keyFile: string }
export const channels = {
  launcher: { name: 'launcher', app: 'stickfell-launcher', file: 'Stickfell.exe', keyEnv: 'STICKFELL_LAUNCHER_PUBLIC_KEY', keyFile: 'launcher-signing-key.pub' },
  game: { name: 'game', app: 'stickfell-game', file: 'stickfell.exe', keyEnv: 'STICKFELL_GAME_PUBLIC_KEY', keyFile: 'game-signing-key.pub' },
} satisfies Record<string, Channel>;
export type ChannelName = keyof typeof channels;

/** A channel's public key (SPKI PEM): from its variable, or from the repo's keys/ folder. */
export async function publicKey(channel: Channel): Promise<string | null> {
  const fromEnv = process.env[channel.keyEnv]?.trim();
  if (fromEnv) return fromEnv;
  try { return await readFile(fileURLToPath(new URL(`../../keys/${channel.keyFile}`, import.meta.url)), 'utf8'); } catch { return null; }
}

/** Checks a manifest's signature (ECDSA P-256 with SHA-256, DER) and reads it. Throws when it isn't genuine. */
export function readSigned(manifestB64: string, signatureB64: string, key: string, app: string): Manifest {
  const bytes = Buffer.from(manifestB64, 'base64');
  if (!verify('sha256', bytes, key, Buffer.from(signatureB64, 'base64'))) throw new Error('The manifest\'s signature doesn\'t match the channel\'s signing key');
  const m = JSON.parse(bytes.toString('utf8')) as Manifest;
  if (m.app !== app || !Number.isInteger(m.build) || m.build < 1 || !/^[0-9a-f]{64}$/.test(m.sha256) || !Number.isInteger(m.size) || m.size < 1 || m.size > MaxExeBytes) throw new Error('The manifest is malformed');
  return m;
}

const digest = (s: string) => createHash('sha256').update(s).digest();

export class Updates {
  constructor(
    private readonly directory: string,
    private readonly uploadKey: string | undefined,
    readonly channel: Channel,
  ) {}

  private get folder() { return join(this.directory, this.channel.name); }
  private get exe() { return join(this.folder, this.channel.file); }
  private get meta() { return join(this.folder, 'latest.json'); }

  private async stored(): Promise<Stored | null> {
    try { return JSON.parse(await readFile(this.meta, 'utf8')) as Stored; } catch { return null; }
  }

  uploadAllowed(header: string | undefined): boolean {
    const token = /^Bearer\s+(\S+)\s*$/i.exec(header ?? '')?.[1];
    return Boolean(this.uploadKey && token && timingSafeEqual(digest(token), digest(this.uploadKey)));
  }

  /** Stores an uploaded build after checking its signature, hash and size, and that it's newer than the current one. */
  async upload(body: Buffer, manifestB64: string, signatureB64: string): Promise<Manifest> {
    const key = await publicKey(this.channel);
    if (!key) throw new Error(`This server has no public key for the ${this.channel.name} channel`);
    const m = readSigned(manifestB64, signatureB64, key, this.channel.app);
    if (body.length !== m.size || createHash('sha256').update(body).digest('hex') !== m.sha256) throw new Error('The upload doesn\'t match its manifest (size or SHA-256)');
    const current = await this.stored();
    if (current) {
      const had = readSigned(current.manifest, current.signature, key, this.channel.app);
      if (m.build <= had.build) throw new Error(`Build ${m.build} isn't newer than the stored build ${had.build}`);
    }
    await mkdir(this.folder, { recursive: true });
    await writeFile(`${this.exe}.tmp`, body);
    await rename(`${this.exe}.tmp`, this.exe);
    await writeFile(`${this.meta}.tmp`, JSON.stringify({ manifest: manifestB64, signature: signatureB64, uploadedAt: new Date().toISOString() } satisfies Stored));
    await rename(`${this.meta}.tmp`, this.meta);
    return m;
  }

  /** The latest build's signed manifest (the launcher checks the signature itself). */
  async latest() {
    const s = await this.stored();
    return { available: Boolean(s), manifest: s?.manifest ?? null, signature: s?.signature ?? null, uploadedAt: s?.uploadedAt ?? null };
  }

  async download(res: ServerResponse) {
    const s = await this.stored();
    const size = s ? (await stat(this.exe).catch(() => null))?.size : undefined;
    if (!s || !size) { res.writeHead(404, { 'content-type': 'application/json' }).end(JSON.stringify({ error: `No ${this.channel.name} build has been uploaded yet` })); return; }
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': String(size), 'content-disposition': `attachment; filename="${this.channel.file}"` });
    createReadStream(this.exe).pipe(res);
  }
}

/** Reads a request body up to a limit (an exe upload). */
export function readLimited(req: IncomingMessage, limit: number): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (c: Buffer) => { size += c.length; if (size > limit) { reject(new Error('too large')); req.destroy(); } else chunks.push(c); });
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}
