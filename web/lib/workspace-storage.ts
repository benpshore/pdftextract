// Browser recovery is owner-scoped. Deletion intents live outside the replaceable
// snapshot, so a stale checkpoint (even from another tab) cannot undo deletion.
const database = 'tpe-private-workspace';
const stores = ['workspaces', 'documentDeletions'];
export type WorkspaceDeletion = {id:string;pending:boolean;title?:string};
type Snapshot = {items:{id:string;record?:{id:string}}[];selection?:{queueId?:string;documentId?:string}|null};
function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(database, 3);
    request.onupgradeneeded = () => {
      for (const name of stores) if (!request.result.objectStoreNames.contains(name)) request.result.createObjectStore(name);
    };
    let blocked = false;
    request.onsuccess = () => {
      const db = request.result;
      if (blocked) { db.close(); return; }
      db.onversionchange = () => db.close();
      resolve(db);
    };
    request.onerror = () => reject(request.error);
    request.onblocked = () => { blocked = true; reject(new Error('Close older TPE tabs, then retry.')); };
  });
}
function withoutDeleted<T>(value:T, deletions:WorkspaceDeletion[]):T {
  if (!value || typeof value !== 'object' || !('items' in value) || !Array.isArray(value.items)) return value;
  const snapshot = value as T & Snapshot;
  const ids = new Set(deletions.map(entry => entry.id));
  const removed = new Set(snapshot.items.filter(item => item.record && ids.has(item.record.id)).map(item => item.id));
  const selection = snapshot.selection;
  return {...snapshot, items:snapshot.items.filter(item => !removed.has(item.id)),
    selection:selection && (ids.has(selection.documentId || '') || removed.has(selection.queueId || '')) ? null : selection};
}
async function transaction<T>(mode:IDBTransactionMode, action:(tx:IDBTransaction, result:(value:T)=>void, guard:(callback:()=>void)=>()=>void)=>void):Promise<T> {
  const db = await openDatabase();
  try {
    return await new Promise<T>((resolve, reject) => {
      const tx = db.transaction(stores, mode);
      let value:T;
      tx.oncomplete = () => resolve(value);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error ?? new Error('Local recovery transaction interrupted.'));
      const guard = (callback:()=>void) => () => {
        try { callback(); } catch (error) { tx.abort(); reject(error); }
      };
      guard(() => action(tx, result => { value = result; }, guard))();
    });
  } finally { db.close(); }
}
export async function readWorkspace<T>(userId:string):Promise<T|null> {
  return transaction('readonly', (tx, done, guard) => {
    const snapshot = tx.objectStore('workspaces').get(userId);
    const journal = tx.objectStore('documentDeletions').get(userId);
    journal.onsuccess = guard(() => done(withoutDeleted(snapshot.result ?? null, journal.result ?? [])));
  });
}
const writes = new Map<string, Promise<void>>();
export async function writeWorkspace<T>(userId:string, value:T):Promise<void> {
  const snapshot = structuredClone(value);
  const pending = (writes.get(userId) ?? Promise.resolve()).catch(() => {}).then(() => transaction<void>('readwrite', (tx, done, guard) => {
    const journal = tx.objectStore('documentDeletions').get(userId);
    journal.onsuccess = guard(() => { tx.objectStore('workspaces').put(withoutDeleted(snapshot, journal.result ?? []), userId); done(); });
  }));
  writes.set(userId, pending);
  try { await pending; } finally { if (writes.get(userId) === pending) writes.delete(userId); }
}
export async function readWorkspaceDeletions(userId:string):Promise<WorkspaceDeletion[]> {
  return transaction('readonly', (tx, done, guard) => {
    const request = tx.objectStore('documentDeletions').get(userId);
    request.onsuccess = guard(() => done(request.result ?? []));
  });
}
async function updateDeletion(userId:string, entry:WorkspaceDeletion):Promise<WorkspaceDeletion[]> {
  const entries = await transaction<WorkspaceDeletion[]>('readwrite', (tx, done, guard) => {
    const values = tx.objectStore('workspaces'), journal = tx.objectStore('documentDeletions');
    const snapshot = values.get(userId), request = journal.get(userId);
    request.onsuccess = guard(() => {
      const entries:WorkspaceDeletion[] = request.result ?? [];
      const existing = entries.find(value => value.id === entry.id);
      // Completion is monotonic, including concurrent repeats from stale tabs.
      const next = [...entries.filter(value => value.id !== entry.id), existing && !existing.pending ? existing : entry];
      journal.put(next, userId);
      if (snapshot.result) values.put(withoutDeleted(snapshot.result, next), userId);
      done(next);
    });
  });
  if (typeof BroadcastChannel !== 'undefined') {
    const channel = new BroadcastChannel('tpe-document-deletions');channel.postMessage(userId);channel.close();
  }
  return entries;
}
/** Only after confirmation: reserve this ID and remove its local copies atomically. */
export function beginWorkspaceDeletion(userId:string, record:{id:string;title:string}) {
  return updateDeletion(userId, {...record, pending:true});
}
/** Retain only the ID after both server cleanup and this transaction complete. */
export function completeWorkspaceDeletion(userId:string, id:string) {
  return updateDeletion(userId, {id, pending:false});
}
