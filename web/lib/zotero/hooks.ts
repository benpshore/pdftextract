'use client';

// React hooks around the client: key connection (restored from the
// browser's own storage), collections of a library, and one send run.

import { useCallback, useEffect, useRef, useState } from 'react';
import { ZoteroApi, accessFor, type Collection, type Group, type KeyInfo, type ZoteroApiOptions, type ZoteroLibraryRef } from './api';
import { clearStoredKey, loadStoredKey, storeKey, type KeyPersistence } from './key-storage';
import { describeError, sendToZotero, type SendOutcome, type SendPlan } from './send';

export type LibraryChoice = { ref: ZoteroLibraryRef; label: string; writable: boolean };
export type ConnectionStatus = 'restoring' | 'disconnected' | 'checking' | 'connected';
type ConnectionState = { status: ConnectionStatus; api: ZoteroApi | null; info: KeyInfo | null; groups: Group[]; persistence: KeyPersistence | null; error: string | null };
export type Connection = ConnectionState & {
  libraries: LibraryChoice[];
  connect: (key: string, persistence: KeyPersistence) => Promise<boolean>;
  disconnect: () => Promise<void>;
};

const disconnected: ConnectionState = { status: 'disconnected', api: null, info: null, groups: [], persistence: null, error: null };

/** The libraries a key may write to, user library first. */
export function writableLibraries(info: KeyInfo | null, groups: Group[]): LibraryChoice[] {
  if (!info) return [];
  const out: LibraryChoice[] = [{ ref: { type: 'user', id: info.userId }, label: 'My library' + (info.username ? ' (' + info.username + ')' : ''), writable: info.user.write }];
  for (const group of groups) {
    const ref: ZoteroLibraryRef = { type: 'group', id: group.id };
    out.push({ ref, label: group.name || 'Group ' + group.id, writable: accessFor(info, ref).write });
  }
  return out;
}

/** Connect an API key kept only in this browser; the stored key (if any) is restored on mount. */
export function useZoteroConnection(options: ZoteroApiOptions = {}): Connection {
  const [settings] = useState(options);
  const [state, setState] = useState<ConnectionState>({ ...disconnected, status: 'restoring' });

  const verify = useCallback(async (key: string, persistence: KeyPersistence, store: boolean): Promise<boolean> => {
    const trimmed = key.trim();
    if (!trimmed) { setState({ ...disconnected, error: 'Enter your Zotero API key first.' }); return false; }
    setState(previous => ({ ...previous, status: 'checking', error: null }));
    const api = new ZoteroApi(trimmed, settings);
    try {
      const info = await api.keyInfo();
      let groups: Group[] = [];
      let error: string | null = null;
      try { groups = await api.groups(info.userId); } catch (reason) { error = 'Group libraries could not be listed: ' + describeError(reason); }
      if (store) { try { await storeKey(trimmed, persistence); } catch (reason) { error = 'The key is in use for now but could not be stored: ' + describeError(reason); } }
      setState({ status: 'connected', api, info, groups, persistence, error });
      return true;
    } catch (reason) {
      setState({ ...disconnected, error: describeError(reason) });
      return false;
    }
  }, [settings]);

  useEffect(() => {
    let active = true;
    loadStoredKey().then(stored => {
      if (!active) return;
      if (stored) void verify(stored.key, stored.persistence, false);
      else setState(disconnected);
    }).catch(() => { if (active) setState(disconnected); });
    return () => { active = false; };
  }, [verify]);

  const connect = useCallback((key: string, persistence: KeyPersistence) => verify(key, persistence, true), [verify]);
  const disconnect = useCallback(async () => { await clearStoredKey(); setState(disconnected); }, []);

  return { ...state, libraries: writableLibraries(state.info, state.groups), connect, disconnect };
}

type CollectionsState = { scope: string; collections: Collection[]; error: string | null };

/** Collections of one library, reloaded when the library or `reload()` changes. */
export function useZoteroCollections(api: ZoteroApi | null, ref: ZoteroLibraryRef | null): { collections: Collection[]; loading: boolean; error: string | null; reload: () => void } {
  const [generation, setGeneration] = useState(0);
  const [state, setState] = useState<CollectionsState | null>(null);
  // Primitive dependencies: callers may build a fresh `ref` object every render.
  const type = ref?.type ?? null;
  const id = ref?.id ?? null;
  const scope = api && type && id !== null ? type + ':' + id + ':' + generation : '';

  useEffect(() => {
    if (!api || !type || id === null || !scope) return;
    let active = true;
    api.collections({ type, id })
      .then(collections => { if (active) setState({ scope, collections, error: null }); })
      .catch((reason: unknown) => { if (active) setState({ scope, collections: [], error: 'Collections could not be loaded: ' + describeError(reason) }); });
    return () => { active = false; };
  }, [api, type, id, scope]);

  const current = state && state.scope === scope ? state : null;
  return { collections: current?.collections ?? [], loading: !!scope && !current, error: current?.error ?? null, reload: () => setGeneration(value => value + 1) };
}

/** One send run with live progress text. */
export function useSendToZotero(api: ZoteroApi | null, info: KeyInfo | null): { send: (plan: SendPlan | (() => Promise<SendPlan>)) => Promise<SendOutcome | null>; busy: boolean; progress: string; outcome: SendOutcome | null; error: string | null; reset: () => void; submitted: boolean } {
  const inFlight = useRef(false);
  const parentAttempted = useRef(false);
  const saved = useRef<SendOutcome | null>(null);
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState('');
  const [outcome, setOutcome] = useState<SendOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);

  const send = useCallback(async (input: SendPlan | (() => Promise<SendPlan>)) => {
    // Refs lock synchronously, before React renders or any original-file await.
    if (inFlight.current || parentAttempted.current) return saved.current;
    if (!api) { setError('Connect a Zotero API key first.'); return null; }
    inFlight.current = true;
    setBusy(true); setError(null); setProgress('Preparing…');
    try {
      const plan = typeof input === 'function' ? await input() : input;
      // Once a parent write starts, even a lost response may mean it exists.
      // Do not issue a new write token after an uncertain result.
      parentAttempted.current = true;
      setSubmitted(true);
      const result = await sendToZotero(api, info, plan, setProgress);
      saved.current = result;
      setOutcome(result);
      return result;
    } catch (reason) {
      setError(describeError(reason) + (parentAttempted.current ? ' The parent write may have completed. Check your Zotero library before starting another send; this panel will not create another parent.' : ''));
      setProgress('');
      return null;
    } finally { inFlight.current = false; setBusy(false); }
  }, [api, info]);

  const reset = useCallback(() => { if (!parentAttempted.current) { setOutcome(null); setError(null); setProgress(''); } }, []);
  return { send, busy, progress, outcome, error, reset, submitted };
}
