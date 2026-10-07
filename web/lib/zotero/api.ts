// Zotero Web API v3 client for the browser. Calls api.zotero.org directly:
// the API answers every request with `Access-Control-Allow-Origin: *`,
// allows the `Zotero-API-Key`, `Zotero-API-Version`, `Zotero-Write-Token`,
// `If-Match`, `If-None-Match` and `If-Unmodified-Since-Version` request
// headers and exposes `Last-Modified-Version`, `Total-Results`, `Link`,
// `Backoff` and `Retry-After`, so no server of ours sees the key. The key
// travels only in the request header, never in a URL.
//
// Pure helpers (`parse*`, `nextLink`, `libraryPrefix`) are exported for tests;
// `ZoteroApi` adds transport, `Backoff` waits and `Retry-After` retries.

export const ZOTERO_API_BASE = 'https://api.zotero.org';
export const ZOTERO_API_VERSION = '3';
/** Most objects one write request may carry. */
export const MAX_WRITE_ITEMS = 50;
/** Largest page the API serves. */
export const MAX_LIMIT = 100;

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;
export type SleepLike = (ms: number) => Promise<void>;

export type ZoteroLibraryRef = { type: 'user' | 'group'; id: number };
export type Access = { library: boolean; files: boolean; notes: boolean; write: boolean };
export type KeyInfo = { userId: number; username?: string; displayName?: string; user: Access; groups: Record<string, Access> };
export type Group = { id: number; version: number; name: string; libraryEditing?: string; numItems?: number };
export type Collection = { key: string; version: number; name: string; parent: string | null };
export type WriteFailure = { key?: string; code: number; message: string };
export type WriteResult = { successful: Record<number, string>; unchanged: Record<number, string>; failed: Record<number, WriteFailure>; lastModifiedVersion?: number };
export type UploadTarget = { url: string; contentType?: string; prefix: string; suffix: string; uploadKey: string; params: Record<string, string> };
export type UploadAuthorization = { exists: true } | { exists: false; target: UploadTarget };
export type FileDescriptor = { md5: string; filename: string; filesize: number; mtimeMs: number };

export type ZoteroErrorKind = 'network' | 'bad-request' | 'forbidden' | 'not-found' | 'conflict' | 'precondition' | 'too-large' | 'rate-limited' | 'http' | 'parse' | 'write-failed';

/** A failed call, with a stable `kind` for the UI to explain. */
export class ZoteroApiError extends Error {
  kind: ZoteroErrorKind;
  status?: number;
  retryAfterSeconds?: number;
  constructor(kind: ZoteroErrorKind, message: string, status?: number, retryAfterSeconds?: number) {
    super(message);
    this.name = 'ZoteroApiError';
    this.kind = kind;
    this.status = status;
    this.retryAfterSeconds = retryAfterSeconds;
  }
}

/** `/users/<id>` or `/groups/<id>`. */
export function libraryPrefix(ref: ZoteroLibraryRef): string {
  return (ref.type === 'user' ? '/users/' : '/groups/') + ref.id;
}

/** Keys are 8 alphanumeric ASCII characters; anything else never reaches a URL. */
export function isValidKey(key: string): boolean { return /^[A-Za-z0-9]{8}$/.test(key); }

/** Parse a `Link` header into `{url, rels}` entries (commas inside `<...>` are kept). */
export function parseLinkHeader(value: string | null | undefined): { url: string; rels: string[] }[] {
  const out: { url: string; rels: string[] }[] = [];
  if (!value) return out;
  const pattern = /<([^>]*)>([^<]*)/g;
  for (let match = pattern.exec(value); match; match = pattern.exec(value)) {
    const rels: string[] = [];
    for (const param of match[2].split(';')) {
      const [name, raw] = param.split('=');
      if (name?.trim().toLowerCase() === 'rel' && raw) rels.push(...raw.trim().replace(/^"|"$|,$/g, '').replace(/"$/, '').split(/\s+/).map(rel => rel.toLowerCase()));
    }
    out.push({ url: match[1].trim(), rels });
  }
  return out;
}

/** URL of the `rel="next"` page, or null on the last page. */
export function nextLink(value: string | null | undefined): string | null {
  return parseLinkHeader(value).find(entry => entry.rels.includes('next'))?.url ?? null;
}

/** A fresh 32-character hexadecimal `Zotero-Write-Token`. */
export function newWriteToken(): string {
  const bytes = new Uint8Array(16);
  if (typeof crypto !== 'undefined' && crypto.getRandomValues) crypto.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

function asObject(value: unknown, what: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ZoteroApiError('parse', what + ' is not a JSON object.');
  return value as Record<string, unknown>;
}
function asArray(value: unknown, what: string): unknown[] {
  if (!Array.isArray(value)) throw new ZoteroApiError('parse', what + ' is not a JSON array.');
  return value;
}
const text = (object: Record<string, unknown>, name: string): string | undefined => typeof object[name] === 'string' ? (object[name] as string) : undefined;
const number = (object: Record<string, unknown>, name: string): number | undefined => typeof object[name] === 'number' ? (object[name] as number) : undefined;

function accessOf(value: unknown): Access {
  const object = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  return { library: object.library === true, files: object.files === true, notes: object.notes === true, write: object.write === true };
}

/** Parse `GET /keys/current`. */
export function parseKeyInfo(body: unknown): KeyInfo {
  const object = asObject(body, 'Key information');
  const userId = number(object, 'userID');
  if (userId === undefined) throw new ZoteroApiError('parse', 'Key information has no userID.');
  const access = object.access && typeof object.access === 'object' ? (object.access as Record<string, unknown>) : {};
  const groups: Record<string, Access> = {};
  if (access.groups && typeof access.groups === 'object') for (const [id, value] of Object.entries(access.groups as Record<string, unknown>)) groups[id] = accessOf(value);
  return { userId, username: text(object, 'username'), displayName: text(object, 'displayName'), user: accessOf(access.user), groups };
}

/** Permissions of a key on one library (`all` covers unlisted groups). */
export function accessFor(info: KeyInfo, ref: ZoteroLibraryRef): Access {
  if (ref.type === 'user') return info.user;
  return info.groups[String(ref.id)] ?? info.groups.all ?? { library: false, files: false, notes: false, write: false };
}

/** Parse `GET /users/<id>/groups`. */
export function parseGroups(body: unknown): Group[] {
  return asArray(body, 'Group list').map(entry => {
    const envelope = asObject(entry, 'Group');
    const data = envelope.data && typeof envelope.data === 'object' ? (envelope.data as Record<string, unknown>) : {};
    const id = number(envelope, 'id') ?? number(data, 'id');
    if (id === undefined) throw new ZoteroApiError('parse', 'Group without an id.');
    const meta = envelope.meta && typeof envelope.meta === 'object' ? (envelope.meta as Record<string, unknown>) : {};
    return { id, version: number(envelope, 'version') ?? number(data, 'version') ?? 0, name: text(data, 'name') ?? '', libraryEditing: text(data, 'libraryEditing'), numItems: number(meta, 'numItems') };
  });
}

/** Parse a page of `GET <prefix>/collections`. */
export function parseCollections(body: unknown): Collection[] {
  return asArray(body, 'Collection list').map(entry => {
    const envelope = asObject(entry, 'Collection');
    const data = asObject(envelope.data, 'Collection data');
    const key = text(envelope, 'key') ?? text(data, 'key');
    if (!key) throw new ZoteroApiError('parse', 'Collection without a key.');
    return { key, version: number(envelope, 'version') ?? 0, name: text(data, 'name') ?? '', parent: text(data, 'parentCollection') ?? null };
  });
}

/** Parse the 200 body of a multi-object write (`success`/`successful`, `unchanged`, `failed`). */
export function parseWriteResult(body: unknown, lastModifiedVersion?: number): WriteResult {
  const object = asObject(body, 'Write result');
  const result: WriteResult = { successful: {}, unchanged: {}, failed: {}, lastModifiedVersion };
  const map = (name: string) => (object[name] && typeof object[name] === 'object' ? (object[name] as Record<string, unknown>) : {});
  for (const [index, key] of Object.entries(map('success'))) if (typeof key === 'string') result.successful[Number(index)] = key;
  for (const [index, entry] of Object.entries(map('successful'))) {
    const key = typeof entry === 'string' ? entry : entry && typeof entry === 'object' ? text(entry as Record<string, unknown>, 'key') : undefined;
    if (key && result.successful[Number(index)] === undefined) result.successful[Number(index)] = key;
  }
  for (const [index, key] of Object.entries(map('unchanged'))) if (typeof key === 'string') result.unchanged[Number(index)] = key;
  for (const [index, entry] of Object.entries(map('failed'))) {
    const failure = entry && typeof entry === 'object' ? (entry as Record<string, unknown>) : {};
    result.failed[Number(index)] = { key: text(failure, 'key'), code: number(failure, 'code') ?? 0, message: text(failure, 'message') ?? '' };
  }
  return result;
}

/** The key written at `index`, or the server's refusal as an error. */
export function keyAt(result: WriteResult, index: number): string {
  const key = result.successful[index] ?? result.unchanged[index];
  if (key) return key;
  const failure = result.failed[index];
  if (failure) throw new ZoteroApiError('write-failed', 'Zotero refused the object (' + failure.code + '): ' + failure.message, failure.code);
  throw new ZoteroApiError('parse', 'The write response has no entry for object ' + index + '.');
}

/** Parse the 200 body of the upload authorisation request. */
export function parseUploadAuthorization(body: unknown): UploadAuthorization {
  const object = asObject(body, 'Upload authorisation');
  if (object.exists === 1 || object.exists === true) return { exists: true };
  const url = text(object, 'url'), uploadKey = text(object, 'uploadKey');
  if (!url || !uploadKey) throw new ZoteroApiError('parse', 'Upload authorisation without url or uploadKey.');
  const params: Record<string, string> = {};
  if (object.params && typeof object.params === 'object') for (const [name, value] of Object.entries(object.params as Record<string, unknown>)) if (typeof value === 'string') params[name] = value;
  return { exists: false, target: { url, contentType: text(object, 'contentType'), prefix: text(object, 'prefix') ?? '', suffix: text(object, 'suffix') ?? '', uploadKey, params } };
}

/** `application/x-www-form-urlencoded` body. */
export function formEncode(pairs: Record<string, string | number>): string {
  return Object.entries(pairs).map(([name, value]) => encodeURIComponent(name) + '=' + encodeURIComponent(String(value))).join('&');
}

/** Web page of an item on zotero.org, when it can be named (user libraries need the username). */
export function itemWebUrl(info: KeyInfo, ref: ZoteroLibraryRef, key: string): string | null {
  if (!isValidKey(key)) return null;
  if (ref.type === 'group') return 'https://www.zotero.org/groups/' + ref.id + '/items/' + key;
  return info.username ? 'https://www.zotero.org/' + encodeURIComponent(info.username) + '/items/' + key : null;
}

export type ZoteroApiOptions = { fetch?: FetchLike; sleep?: SleepLike; base?: string; maxAttempts?: number; maxDelayMs?: number };
type Reply = { status: number; headers: Headers; text: string };

const defaultSleep: SleepLike = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Browser client bound to one API key. */
export class ZoteroApi {
  private readonly key: string;
  private readonly fetchImpl: FetchLike;
  private readonly sleep: SleepLike;
  private readonly base: string;
  private readonly maxAttempts: number;
  private readonly maxDelayMs: number;
  private backoffUntil = 0;

  constructor(key: string, options: ZoteroApiOptions = {}) {
    this.key = key.trim();
    this.fetchImpl = options.fetch ?? ((input, init) => fetch(input, init));
    this.sleep = options.sleep ?? defaultSleep;
    this.base = (options.base ?? ZOTERO_API_BASE).replace(/\/$/, '');
    this.maxAttempts = options.maxAttempts ?? 3;
    this.maxDelayMs = options.maxDelayMs ?? 60_000;
  }

  /** The user id and permissions behind the key (`GET /keys/current`). */
  async keyInfo(): Promise<KeyInfo> { return parseKeyInfo(JSON.parse((await this.request('GET', this.base + '/keys/current')).text)); }

  /** Groups of a user (all pages). */
  async groups(userId: number): Promise<Group[]> { return this.pages(this.base + '/users/' + userId + '/groups?limit=' + MAX_LIMIT, parseGroups); }

  /** Collections of a library (all pages). */
  async collections(ref: ZoteroLibraryRef): Promise<Collection[]> { return this.pages(this.base + libraryPrefix(ref) + '/collections?limit=' + MAX_LIMIT, parseCollections); }

  /** Empty template of a new item (`GET /items/new?itemType=…`). */
  async itemTemplate(itemType: string, linkMode?: string): Promise<Record<string, unknown>> {
    const url = this.base + '/items/new?itemType=' + encodeURIComponent(itemType) + (linkMode ? '&linkMode=' + encodeURIComponent(linkMode) : '');
    const template = asObject(JSON.parse((await this.request('GET', url)).text), 'Item template');
    if (typeof template.itemType !== 'string') throw new ZoteroApiError('parse', 'Item template without an itemType.');
    return template;
  }

  /** Create up to 50 items in one write-token guarded request. */
  async createItems(ref: ZoteroLibraryRef, items: Record<string, unknown>[]): Promise<WriteResult> {
    if (items.length > MAX_WRITE_ITEMS) throw new ZoteroApiError('bad-request', 'At most ' + MAX_WRITE_ITEMS + ' objects per write request.');
    if (!items.length) return { successful: {}, unchanged: {}, failed: {} };
    const token = newWriteToken();
    const reply = await this.request('POST', this.base + libraryPrefix(ref) + '/items', { body: JSON.stringify(items), contentType: 'application/json', headers: { 'Zotero-Write-Token': token } });
    return parseWriteResult(JSON.parse(reply.text), versionOf(reply.headers));
  }

  /** Create one collection; returns its key. */
  async createCollection(ref: ZoteroLibraryRef, name: string, parent?: string | null): Promise<string> {
    if (parent && !isValidKey(parent)) throw new ZoteroApiError('bad-request', 'Invalid parent collection key.');
    const reply = await this.request('POST', this.base + libraryPrefix(ref) + '/collections', { body: JSON.stringify([{ name, parentCollection: parent || false, relations: {} }]), contentType: 'application/json', headers: { 'Zotero-Write-Token': newWriteToken() } });
    return keyAt(parseWriteResult(JSON.parse(reply.text), versionOf(reply.headers)), 0);
  }

  /** Partial update guarded by the item's version (412 → `precondition`). */
  async updateItem(ref: ZoteroLibraryRef, key: string, version: number, patch: Record<string, unknown>): Promise<number | undefined> {
    if (!isValidKey(key)) throw new ZoteroApiError('bad-request', 'Invalid item key.');
    const reply = await this.request('PATCH', this.base + libraryPrefix(ref) + '/items/' + key, { body: JSON.stringify(patch), contentType: 'application/json', headers: { 'If-Unmodified-Since-Version': String(version) } });
    return versionOf(reply.headers);
  }

  /** Upload step 1: ask where to send the file of attachment `key`. */
  async authorizeUpload(ref: ZoteroLibraryRef, key: string, file: FileDescriptor, existingMd5?: string): Promise<UploadAuthorization> {
    if (!isValidKey(key)) throw new ZoteroApiError('bad-request', 'Invalid attachment key.');
    const reply = await this.request('POST', this.base + libraryPrefix(ref) + '/items/' + key + '/file', { body: formEncode({ md5: file.md5, filename: file.filename, filesize: file.filesize, mtime: file.mtimeMs }), contentType: 'application/x-www-form-urlencoded', headers: existingMd5 ? { 'If-Match': existingMd5 } : { 'If-None-Match': '*' } });
    return parseUploadAuthorization(JSON.parse(reply.text));
  }

  /** Upload step 2: send the bytes to the storage host. No Zotero headers go there. */
  async uploadFile(target: UploadTarget, file: Blob, filename: string): Promise<void> {
    let body: BodyInit, headers: Record<string, string> = {};
    if (Object.keys(target.params).length) {
      const form = new FormData();
      for (const [name, value] of Object.entries(target.params)) form.append(name, value);
      form.append('file', file, filename);
      body = form;
    } else {
      body = new Blob([target.prefix, file, target.suffix]);
      headers = { 'Content-Type': target.contentType || 'application/octet-stream' };
    }
    let response: Response;
    try { response = await this.fetchImpl(target.url, { method: 'POST', headers, body }); }
    catch (reason) { throw new ZoteroApiError('network', 'The file could not be sent to Zotero storage (' + describe(reason) + '). The browser may have blocked the cross-origin upload; add a link to the source instead.'); }
    if (response.status < 200 || response.status > 299) throw new ZoteroApiError('http', 'Zotero storage answered ' + response.status + '.', response.status);
  }

  /** Upload step 3: register the finished upload (204). */
  async registerUpload(ref: ZoteroLibraryRef, key: string, uploadKey: string, existingMd5?: string): Promise<void> {
    if (!isValidKey(key)) throw new ZoteroApiError('bad-request', 'Invalid attachment key.');
    await this.request('POST', this.base + libraryPrefix(ref) + '/items/' + key + '/file', { body: formEncode({ upload: uploadKey }), contentType: 'application/x-www-form-urlencoded', headers: existingMd5 ? { 'If-Match': existingMd5 } : { 'If-None-Match': '*' } });
  }

  /** Follow `Link: rel="next"` until the last page; links leaving the API base are refused. */
  private async pages<T>(first: string, parse: (body: unknown) => T[]): Promise<T[]> {
    const out: T[] = [];
    let url: string | null = first;
    while (url) {
      if (!url.startsWith(this.base + '/')) throw new ZoteroApiError('parse', 'Refusing to follow a pagination link outside the API: ' + url);
      const reply: Reply = await this.request('GET', url);
      out.push(...parse(JSON.parse(reply.text)));
      url = nextLink(reply.headers.get('Link'));
    }
    return out;
  }

  /** One request with the Zotero headers, `Backoff` waits, `Retry-After`/5xx/network retries and status mapping. */
  private async request(method: string, url: string, options: { body?: string; contentType?: string; headers?: Record<string, string> } = {}): Promise<Reply> {
    if (!url.startsWith(this.base + '/')) throw new ZoteroApiError('bad-request', 'Request outside the API base.');
    for (let attempt = 1; ; attempt++) {
      const wait = this.backoffUntil - Date.now();
      if (wait > 0) await this.sleep(Math.min(wait, this.maxDelayMs));
      this.backoffUntil = 0;
      const headers: Record<string, string> = { 'Zotero-API-Version': ZOTERO_API_VERSION, 'Zotero-API-Key': this.key, ...options.headers };
      if (options.contentType) headers['Content-Type'] = options.contentType;
      let error: ZoteroApiError;
      try {
        const response = await this.fetchImpl(url, { method, headers, body: options.body, redirect: 'error' });
        const backoff = Number(response.headers.get('Backoff'));
        if (backoff > 0) this.backoffUntil = Date.now() + Math.min(backoff * 1000, this.maxDelayMs);
        const body = await response.text();
        if ((response.status >= 200 && response.status < 300) || response.status === 304) return { status: response.status, headers: response.headers, text: body };
        error = statusError(response.status, body, response.headers);
      } catch (reason) {
        error = reason instanceof ZoteroApiError ? reason : new ZoteroApiError('network', 'Zotero could not be reached (' + describe(reason) + '). Check the connection; if it persists, the browser blocked the request.');
      }
      const delay = retryDelay(error, attempt, this.maxAttempts, this.maxDelayMs);
      if (delay === null) throw error;
      await this.sleep(delay);
    }
  }
}

function versionOf(headers: Headers): number | undefined {
  const value = Number(headers.get('Last-Modified-Version'));
  return Number.isFinite(value) && headers.get('Last-Modified-Version') ? value : undefined;
}

function describe(reason: unknown): string { return reason instanceof Error ? reason.message : String(reason); }

/** Map an error status to a `ZoteroApiError`. */
export function statusError(status: number, body: string, headers?: Headers): ZoteroApiError {
  const snippet = body.slice(0, 300);
  switch (status) {
    case 400: return new ZoteroApiError('bad-request', 'Zotero rejected the request: ' + snippet, status);
    case 403: return new ZoteroApiError('forbidden', 'Zotero refused the API key: it is invalid or lacks the needed permission (library, notes and write access).', status);
    case 404: return new ZoteroApiError('not-found', 'Zotero could not find that library or object.', status);
    case 409: return new ZoteroApiError('conflict', 'The Zotero library is locked right now. Try again shortly.', status);
    case 412: return new ZoteroApiError('precondition', 'The Zotero object changed since it was read, or this write was already applied.', status);
    case 413: return new ZoteroApiError('too-large', 'Zotero refused the request as too large: ' + snippet, status);
    case 428: return new ZoteroApiError('precondition', 'Zotero requires a version header for this request.', status);
    case 429: { const after = Number(headers?.get('Retry-After')); return new ZoteroApiError('rate-limited', 'Zotero asked to slow down.', status, Number.isFinite(after) && after > 0 ? after : undefined); }
    default: return new ZoteroApiError('http', 'Zotero answered ' + status + (snippet ? ': ' + snippet : '.'), status);
  }
}

/** Milliseconds to wait before attempt `attempt + 1`, or null to give up. */
export function retryDelay(error: ZoteroApiError, attempt: number, maxAttempts: number, maxDelayMs: number): number | null {
  if (attempt >= maxAttempts) return null;
  const exponential = Math.min(1000 * 2 ** (attempt - 1), maxDelayMs);
  if (error.kind === 'rate-limited') return Math.min((error.retryAfterSeconds ?? exponential / 1000) * 1000, maxDelayMs);
  if (error.kind === 'network' || (error.kind === 'http' && error.status !== undefined && error.status >= 500)) return exponential;
  return null;
}
