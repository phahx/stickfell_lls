import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export type KeyKind = 'signed' | 'itch' | 'tester';
export type KeyEventType = 'issued' | 'imported' | 'activated' | 'new-address' | 'revoked' | 'restored';
export interface KeyEvent { at: number; type: KeyEventType; detail?: string }

/**
 * One tester key's life: issued (signed STICK- keys made here or imported), first used, used, revoked. Signed keys are
 * kept whole (they're yours to hand out); any other key only by hash, with its last characters as a label. Addresses
 * are hashed too: only how many there are matters (one key on many addresses is a shared key). (Copied from the
 * khovdesk update service, so the keys app works with both; 'itch' is never used here.)
 */
export interface KeyRecord {
  id: string;
  kind: KeyKind;
  label: string;
  key?: string;
  batch?: string;
  issuedAt?: number;
  firstUsed?: number;
  lastUsed?: number;
  uses: number;
  addresses: string[];
  revokedAt?: number;
  reason?: string;
  events: KeyEvent[];
}

/** What the keys app gets: addresses as a count. */
export type KeyView = Omit<KeyRecord, 'addresses'> & { addresses: number };

const MaxAddresses = 100;
const MaxEvents = 50;
const addressHash = (client: string) => createHash('sha256').update(`address:${client}`).digest('hex').slice(0, 16);

/** The update service's list of tester keys (keys.json in its data folder), for the keys app. */
export class KeyRegistry {
  private keys = new Map<string, KeyRecord>();
  private loading?: Promise<void>;
  private saving = Promise.resolve();

  constructor(private readonly directory: string, private readonly now: () => number = Date.now) {}

  private get file() { return join(this.directory, 'keys.json'); }

  private load() {
    return this.loading ??= readFile(this.file, 'utf8')
      .then(text => { for (const r of JSON.parse(text) as KeyRecord[]) this.keys.set(r.id, r); })
      .catch(() => { /* none yet */ });
  }

  private save() {
    const text = JSON.stringify([...this.keys.values()]);
    this.saving = this.saving.then(async () => {
      await mkdir(this.directory, { recursive: true });
      await writeFile(`${this.file}.tmp`, text);
      await rename(`${this.file}.tmp`, this.file);
    }).catch(() => {});
    return this.saving;
  }

  private event(r: KeyRecord, type: KeyEventType, detail?: string) {
    r.events.push({ at: this.now(), type, ...(detail ? { detail } : {}) });
    if (r.events.length > MaxEvents) r.events.splice(1, r.events.length - MaxEvents); // keep how it started
  }

  async list(): Promise<KeyView[]> {
    await this.load();
    return [...this.keys.values()].map(r => ({ ...r, addresses: r.addresses.length }));
  }

  async get(id: string): Promise<KeyView | undefined> {
    await this.load();
    const r = this.keys.get(id);
    return r && { ...r, addresses: r.addresses.length };
  }

  /** Adds signed keys (id = the key's 20 characters) made here or elsewhere; returns the ids that were new. */
  async issue(keys: { id: string; key: string }[], batch: string, imported = false): Promise<string[]> {
    await this.load();
    const added: string[] = [];
    for (const { id, key } of keys) {
      if (this.keys.has(id)) continue;
      const r: KeyRecord = { id, kind: 'signed', label: key, key, batch, issuedAt: this.now(), uses: 0, addresses: [], events: [] };
      this.event(r, imported ? 'imported' : 'issued', batch);
      this.keys.set(id, r);
      added.push(id);
    }
    if (added.length) await this.save();
    return added;
  }

  async isRevoked(id: string): Promise<boolean> {
    await this.load();
    return this.keys.get(id)?.revokedAt !== undefined;
  }

  /** A key that just worked: counts the use, and notes its first use and each new address. */
  async used(id: string, kind: KeyKind, label: string, client: string, key?: string) {
    await this.load();
    let r = this.keys.get(id);
    if (!r) this.keys.set(id, r = { id, kind, label, ...(key ? { key } : {}), uses: 0, addresses: [], events: [] });
    const now = this.now();
    r.uses++;
    r.lastUsed = now;
    if (r.firstUsed === undefined) { r.firstUsed = now; this.event(r, 'activated'); }
    const address = addressHash(client);
    if (!r.addresses.includes(address)) {
      if (r.addresses.length < MaxAddresses) r.addresses.push(address);
      if (r.addresses.length > 1) this.event(r, 'new-address', `address ${r.addresses.length}`);
    }
    await this.save();
  }

  /** Stops a key working (a refund, a leak); `label`/`kind` describe a key that was never seen before. */
  async revoke(id: string, reason: string, unseen: { kind: KeyKind; label: string; key?: string }): Promise<KeyView> {
    await this.load();
    let r = this.keys.get(id);
    if (!r) this.keys.set(id, r = { id, ...unseen, uses: 0, addresses: [], events: [] });
    if (r.revokedAt === undefined) {
      r.revokedAt = this.now();
      r.reason = reason || undefined;
      this.event(r, 'revoked', reason || undefined);
      await this.save();
    }
    return (await this.get(id))!;
  }

  async restore(id: string): Promise<KeyView | undefined> {
    await this.load();
    const r = this.keys.get(id);
    if (!r) return undefined;
    if (r.revokedAt !== undefined) {
      delete r.revokedAt;
      delete r.reason;
      this.event(r, 'restored');
      await this.save();
    }
    return this.get(id);
  }
}
