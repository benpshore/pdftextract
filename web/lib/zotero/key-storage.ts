// Where the Zotero API key lives: in this browser only. "session" keeps it
// in sessionStorage (gone when the tab closes); "persistent" keeps it in
// IndexedDB on this device. It is never sent to this app's server; the only
// recipient is api.zotero.org, in a request header.

export type KeyPersistence = 'session' | 'persistent';
export type StoredKey = { key: string; persistence: KeyPersistence; savedAt: string };

const SESSION_ITEM = 'tpe-zotero-api-key';
const DATABASE = 'tpe-zotero-credentials';
const STORE = 'keys';
const RECORD = 'api-key';

function sessionArea(): Storage | null {
  try { return typeof sessionStorage === 'undefined' ? null : sessionStorage; } catch { return null; }
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (typeof indexedDB === 'undefined') { reject(new Error('IndexedDB is not available in this browser, so the key can only be kept for this tab.')); return; }
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('IndexedDB could not be opened.'));
    request.onblocked = () => reject(new Error('Close other tabs of this app and try again.'));
  });
}

async function withStore<T>(mode: IDBTransactionMode, run: (store: IDBObjectStore) => IDBRequest<T> | void): Promise<T | undefined> {
  const db = await openDatabase();
  try {
    return await new Promise<T | undefined>((resolve, reject) => {
      const transaction = db.transaction(STORE, mode);
      const request = run(transaction.objectStore(STORE));
      transaction.oncomplete = () => resolve(request ? (request.result as T) : undefined);
      transaction.onerror = () => reject(transaction.error ?? new Error('IndexedDB transaction failed.'));
      transaction.onabort = () => reject(transaction.error ?? new Error('IndexedDB transaction interrupted.'));
    });
  } finally { db.close(); }
}

/** The stored key, session storage first, then IndexedDB; null when none. */
export async function loadStoredKey(): Promise<StoredKey | null> {
  const session = sessionArea()?.getItem(SESSION_ITEM);
  if (session) { try { const parsed = JSON.parse(session) as Partial<StoredKey>; if (typeof parsed.key === 'string' && parsed.key) return { key: parsed.key, persistence: 'session', savedAt: parsed.savedAt ?? '' }; } catch {} }
  try {
    const record = await withStore<Partial<StoredKey> | undefined>('readonly', store => store.get(RECORD) as IDBRequest<Partial<StoredKey> | undefined>);
    if (record && typeof record.key === 'string' && record.key) return { key: record.key, persistence: 'persistent', savedAt: record.savedAt ?? '' };
  } catch {}
  return null;
}

/** Keep the key as the user chose; the other place is cleared. Rejects with a readable message when the chosen storage is unavailable. */
export async function storeKey(key: string, persistence: KeyPersistence): Promise<void> {
  const record: StoredKey = { key: key.trim(), persistence, savedAt: new Date().toISOString() };
  if (persistence === 'session') {
    const area = sessionArea();
    if (!area) throw new Error('Session storage is not available in this browser.');
    area.setItem(SESSION_ITEM, JSON.stringify(record));
    try { await withStore('readwrite', store => { store.delete(RECORD); }); } catch {}
    return;
  }
  await withStore('readwrite', store => { store.put(record, RECORD); });
  try { sessionArea()?.removeItem(SESSION_ITEM); } catch {}
}

/** Forget the key everywhere in this browser. */
export async function clearStoredKey(): Promise<void> {
  try { sessionArea()?.removeItem(SESSION_ITEM); } catch {}
  try { await withStore('readwrite', store => { store.delete(RECORD); }); } catch {}
}
