import { createHash, createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import type { KeyRegistry } from './key-registry.js';

/** Wrong keys one address may try per hour before it has to wait. */
const MaxFailuresPerHour = 20;

export type LicenseResult = 'valid' | 'invalid' | 'limited';

const digest = (s: string) => createHash('sha256').update(s).digest();
const hash = (s: string) => digest(s).toString('hex');

// Tester keys (STICK-XXXXX-XXXXX-XXXXX-XXXXX): 10 random characters and 10 of an HMAC of them with
// STICKFELL_LICENSE_SECRET, in Crockford base32 (no I, L, O or U, so they read and type safely). The HMAC is
// domain-separated, so a key made for any other app with the same secret never works here.
const Base32 = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const base32 = (bytes: Buffer, length: number) => {
  let bits = 0, value = 0, out = '';
  for (const b of bytes) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5 && out.length < length) { out += Base32[(value >>> (bits - 5)) & 31]; bits -= 5; }
  }
  return out;
};
const mac = (secret: string, id: string) => base32(createHmac('sha256', secret).update(`stickfell-tester:${id}`).digest(), 10);

const formatSigned = (body: string) => `STICK-${body.match(/.{5}/g)!.join('-')}`;

/** A new tester key. */
export function makeLicenseKey(secret: string): string {
  const id = base32(randomBytes(8), 10);
  return formatSigned(id + mac(secret, id));
}

/** The 20 characters of a tester key as typed (any case, with or without dashes, I/L read as 1 and O as 0), or null. */
export function signedKeyBody(text: string): string | null {
  const t = text.trim().toUpperCase().replace(/^STICK/, '').replace(/[\s-]/g, '').replace(/[IL]/g, '1').replace(/O/g, '0');
  return /^[0-9A-HJKMNP-TV-Z]{20}$/.test(t) ? t : null;
}

/** A key's id in the key list and how it's shown there: a signed key by its 20 characters and in full, anything else
 * by its hash and its last characters. */
export function keyIdentity(text: string): { id: string; label: string; signed: boolean } {
  const key = text.trim();
  const body = signedKeyBody(key);
  return body ? { id: body, label: formatSigned(body), signed: true } : { id: hash(key), label: `…${key.slice(-6)}`, signed: false };
}

/** Whether a tester key was made with this secret. */
export function isSignedKey(text: string, secret: string): boolean {
  const body = signedKeyBody(text);
  return body !== null && timingSafeEqual(Buffer.from(body.slice(10)), Buffer.from(mac(secret, body.slice(0, 10))));
}

/**
 * Who may download Stickfell builds: testers, by a signed key (made in the keys app or with scripts/tester-keys.mjs,
 * checked here with STICKFELL_LICENSE_SECRET). Revoked keys (STICKFELL_REVOKED_KEYS) never work, nor do keys revoked
 * in the key list (KeyRegistry, from the keys app), which also records each key's use.
 */
export class Licenses {
  private failures = new Map<string, number[]>();

  constructor(private readonly options: {
    /** Checks tester keys (STICKFELL_LICENSE_SECRET). */
    secret?: string;
    revokedKeys?: string[];
    registry?: KeyRegistry;
    now?: () => number;
  }) {}

  private now() { return this.options.now?.() ?? Date.now(); }

  /** Checks a key for a client (its address limits how many wrong keys it can try). */
  async check(text: string, client = ''): Promise<LicenseResult> {
    const now = this.now();
    const recent = (this.failures.get(client) ?? []).filter(t => now - t < 3600_000);
    this.failures.set(client, recent);
    if (recent.length >= MaxFailuresPerHour) return 'limited';
    const who = keyIdentity(text);
    const registry = this.options.registry;
    const result = await registry?.isRevoked(who.id) ? 'invalid' : this.lookup(text);
    if (result === 'invalid') recent.push(now);
    if (result === 'valid' && registry) await registry.used(who.id, 'signed', who.label, client, who.label).catch(() => {});
    return result;
  }

  private lookup(text: string): 'valid' | 'invalid' {
    const signed = signedKeyBody(text);
    const revoked = (this.options.revokedKeys ?? []).map(k => signedKeyBody(k) ?? k.trim());
    if (signed === null || revoked.includes(signed)) return 'invalid';
    return this.options.secret && isSignedKey(signed, this.options.secret) ? 'valid' : 'invalid';
  }
}
