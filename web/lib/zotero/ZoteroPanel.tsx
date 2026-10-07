'use client';

// Self-contained "Send to Zotero" panel: connect a key (kept in this
// browser only), pick a library and collection, review the prefilled
// metadata, and send the item with its references note, source link and
// (optionally) the original file. Every control is labelled, reachable by
// keyboard, at least 44 px tall, and nothing disappears on a timer.

import { useId, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import type { Extracted } from '../types';
import type { ZoteroApiOptions, ZoteroLibraryRef } from './api';
import { useSendToZotero, useZoteroCollections, useZoteroConnection } from './hooks';
import type { KeyPersistence } from './key-storage';
import { ITEM_TYPES, articleMetadata, buildItem, references, referencesNoteHtml, suggestItemType, type ArticleMetadata, type ZoteroItemType } from './mapping';
import { describeError, type SendPlan } from './send';

export type ZoteroOriginal = { url?: string; blob?: Blob; name: string; contentType?: string };
export type ZoteroPanelProps = {
  /** The extraction result shown in the reader. */
  article: Extracted;
  /** Where the article was captured from, when known. */
  sourceUrl?: string | null;
  /** The saved original (same-origin URL or bytes) that can be uploaded as the attachment. */
  original?: ZoteroOriginal | null;
  /** Test hook: fetch and sleep implementations for the API client. */
  apiOptions?: ZoteroApiOptions;
};

const CSS = `
.zotero-panel{border:1px solid var(--border,#d7ded7);border-radius:var(--radius,.8rem);padding:1rem;margin:1rem 0;display:grid;gap:.9rem;background:var(--card,#fff);color:var(--card-foreground,inherit);font-size:1rem}
.zotero-panel h3{margin:0;font-size:1.15rem}
.zotero-panel p{margin:0;line-height:1.5}
.zotero-panel .zotero-help{color:var(--muted-foreground,#5d655f)}
.zotero-panel label,.zotero-panel legend{font-weight:600;display:block;margin-bottom:.3rem}
.zotero-panel .zotero-field{display:grid;gap:.25rem}
.zotero-panel input[type=text],.zotero-panel input[type=password],.zotero-panel input[type=url],.zotero-panel select,.zotero-panel textarea{width:100%;box-sizing:border-box;min-height:44px;padding:.55rem .75rem;font:inherit;border:1px solid var(--input,#c3cdc4);border-radius:.5rem;background:var(--background,#fff);color:inherit}
.zotero-panel textarea{min-height:5.5rem;resize:vertical}
.zotero-panel fieldset{border:1px solid var(--border,#d7ded7);border-radius:.5rem;padding:.6rem .9rem .8rem;margin:0;display:grid;gap:.5rem}
.zotero-panel .zotero-choice{display:flex;align-items:center;gap:.75rem;min-height:44px;font-weight:400;margin:0;cursor:pointer}
.zotero-panel .zotero-choice input{width:1.4rem;height:1.4rem;margin:0;flex:none}
.zotero-panel .zotero-actions{display:flex;flex-wrap:wrap;gap:.75rem}
.zotero-panel button{min-height:48px;min-width:44px;padding:.6rem 1.2rem;font:inherit;font-weight:600;border-radius:.6rem;border:1px solid var(--border,#d7ded7);background:var(--secondary,#f0f2ef);color:var(--secondary-foreground,inherit);cursor:pointer}
.zotero-panel button.zotero-primary{background:var(--primary,#263d35);color:var(--primary-foreground,#fff);border-color:transparent}
.zotero-panel button[disabled]{opacity:.6;cursor:not-allowed}
.zotero-panel :is(button,input,select,textarea,a):focus-visible{outline:3px solid var(--ring,#426e51);outline-offset:2px}
.zotero-panel .zotero-status{padding:.7rem .9rem;border-radius:.5rem;background:var(--notice,#edf3ef)}
.zotero-panel .zotero-error{padding:.7rem .9rem;border-radius:.5rem;background:var(--error-background,#fff1ef);border:1px solid var(--error-border,#c46c60)}
.zotero-panel .zotero-grid{display:grid;gap:.75rem;grid-template-columns:repeat(auto-fit,minmax(14rem,1fr))}
.zotero-panel ul{margin:0;padding-left:1.2rem}
`;

function libraryValue(ref: ZoteroLibraryRef): string { return ref.type + ':' + ref.id; }
function parseLibraryValue(value: string): ZoteroLibraryRef | null {
  const [type, id] = value.split(':');
  return (type === 'user' || type === 'group') && /^\d+$/.test(id ?? '') ? { type, id: Number(id) } : null;
}

export function ZoteroPanel({ article, sourceUrl, original, apiOptions }: ZoteroPanelProps) {
  const id = useId();
  const connection = useZoteroConnection(apiOptions);
  const initial = useMemo(() => articleMetadata(article, sourceUrl), [article, sourceUrl]);
  const [meta, setMeta] = useState<ArticleMetadata>(initial);
  const [itemType, setItemType] = useState<ZoteroItemType>(() => suggestItemType(initial));
  const refs = useMemo(() => references(article, meta.doi), [article, meta.doi]);
  const [keyInput, setKeyInput] = useState('');
  const [persistence, setPersistence] = useState<KeyPersistence>('session');
  const [libraryChoice, setLibraryChoice] = useState('');
  const [collection, setCollection] = useState('');
  const [tags, setTags] = useState('tpe');
  const [includeNote, setIncludeNote] = useState(true);
  const [includeLink, setIncludeLink] = useState(true);
  const [includeFile, setIncludeFile] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);

  const libraries = connection.libraries;
  const selectedLibrary = parseLibraryValue(libraryChoice) ?? libraries[0]?.ref ?? null;
  const libraryWritable = libraries.find(choice => selectedLibrary && libraryValue(choice.ref) === libraryValue(selectedLibrary))?.writable ?? false;
  const collections = useZoteroCollections(connection.api, selectedLibrary);
  const sending = useSendToZotero(connection.api, connection.info);
  const collectionTree = useMemo(() => orderCollections(collections.collections), [collections.collections]);

  const field = (name: keyof ArticleMetadata) => (event: { target: { value: string } }) => setMeta(previous => ({ ...previous, [name]: event.target.value }));

  async function connect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (await connection.connect(keyInput, persistence)) setKeyInput('');
  }

  async function send(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedLibrary) return;
    await sending.send(async () => {
      setFileError(null);
      const item = buildItem({ ...meta, authors: meta.authors.map(author => author.trim()).filter(Boolean) }, { itemType, collections: collection ? [collection] : [], tags: tags.split(',').map(tag => tag.trim()).filter(Boolean) });
      const plan: SendPlan = { library: selectedLibrary, item };
      if (includeNote && refs.length) plan.noteHtml = referencesNoteHtml(refs, article.engine);
      if (includeLink && meta.url) plan.link = { url: meta.url, title: 'Source' };
      if (includeFile && original) {
        try {
          const blob = original.blob ?? await (async () => {
            const response = await fetch(original.url as string);
            if (!response.ok) throw new Error('the original could not be read (' + response.status + ')');
            return response.blob();
          })();
          plan.file = { blob, filename: original.name || 'original', contentType: original.contentType || blob.type || 'application/octet-stream', title: original.name || 'Original' };
        } catch (reason) {
          setFileError('The file will not be uploaded: ' + (reason instanceof Error ? reason.message : String(reason)) + '. You can send the item without it.');
          throw new Error('Original acquisition failed: ' + describeError(reason));
        }
      }
      return plan;
    });
  }

  const busy = connection.status === 'checking' || sending.busy;

  return <section className="zotero-panel" aria-labelledby={id + '-title'}>
    <style>{CSS}</style>
    <h3 id={id + '-title'}>Send to Zotero</h3>
    {connection.status === 'restoring' && <p role="status">Checking for a saved Zotero key…</p>}
    {connection.status !== 'connected' && connection.status !== 'restoring' && <form onSubmit={connect} aria-describedby={id + '-key-help'}>
      <div className="zotero-field">
        <label htmlFor={id + '-key'}>Zotero API key</label>
        <input id={id + '-key'} type="password" autoComplete="off" spellCheck={false} value={keyInput} onChange={event => setKeyInput(event.target.value)} disabled={busy} aria-describedby={id + '-key-help'} />
        <p id={id + '-key-help'} className="zotero-help">Create a key at zotero.org → Settings → Security → Applications with library, notes and write access. The key stays in this browser and is sent only to api.zotero.org, never to this site’s server.</p>
      </div>
      <fieldset>
        <legend>Remember the key</legend>
        <label className="zotero-choice"><input type="radio" name={id + '-persist'} value="session" checked={persistence === 'session'} onChange={() => setPersistence('session')} />Until this tab is closed</label>
        <label className="zotero-choice"><input type="radio" name={id + '-persist'} value="persistent" checked={persistence === 'persistent'} onChange={() => setPersistence('persistent')} />On this device (browser storage)</label>
      </fieldset>
      {connection.error && <div className="zotero-error" role="alert">{connection.error}</div>}
      <div className="zotero-actions"><button type="submit" className="zotero-primary" disabled={busy || !keyInput.trim()}>{connection.status === 'checking' ? 'Checking the key…' : 'Connect'}</button></div>
    </form>}
    {connection.status === 'connected' && connection.info && <form onSubmit={send}>
      <div className="zotero-status" role="status">Connected as {connection.info.displayName || connection.info.username || 'user ' + connection.info.userId}; key kept {connection.persistence === 'persistent' ? 'on this device' : 'for this tab'}.</div>
      {connection.error && <div className="zotero-error" role="alert">{connection.error}</div>}
      <div className="zotero-grid">
        <div className="zotero-field">
          <label htmlFor={id + '-library'}>Library</label>
          <select id={id + '-library'} value={selectedLibrary ? libraryValue(selectedLibrary) : ''} onChange={event => { setLibraryChoice(event.target.value); setCollection(''); }} disabled={busy}>
            {libraries.map(choice => <option key={libraryValue(choice.ref)} value={libraryValue(choice.ref)}>{choice.label}{choice.writable ? '' : ' (read only)'}</option>)}
          </select>
        </div>
        <div className="zotero-field">
          <label htmlFor={id + '-collection'}>Collection</label>
          <select id={id + '-collection'} value={collection} onChange={event => setCollection(event.target.value)} disabled={busy || collections.loading} aria-describedby={id + '-collection-help'}>
            <option value="">No collection (library root)</option>
            {collectionTree.map(entry => <option key={entry.key} value={entry.key}>{'  '.repeat(entry.depth) + entry.name}</option>)}
          </select>
          <p id={id + '-collection-help'} className="zotero-help">{collections.loading ? 'Loading collections…' : collections.error ?? collectionTree.length + ' collections'}</p>
        </div>
        <div className="zotero-field">
          <label htmlFor={id + '-type'}>Item type</label>
          <select id={id + '-type'} value={itemType} onChange={event => setItemType(event.target.value as ZoteroItemType)} disabled={busy}>
            {ITEM_TYPES.map(type => <option key={type.value} value={type.value}>{type.label}</option>)}
          </select>
        </div>
      </div>
      <div className="zotero-field"><label htmlFor={id + '-f-title'}>Title</label><input id={id + '-f-title'} type="text" value={meta.title} onChange={field('title')} required disabled={busy} /></div>
      <div className="zotero-field">
        <label htmlFor={id + '-f-authors'}>Authors, one per line (write “Family, Given” to split names)</label>
        <textarea id={id + '-f-authors'} value={meta.authors.join('\n')} onChange={event => setMeta(previous => ({ ...previous, authors: event.target.value.split('\n') }))} disabled={busy} />
      </div>
      <div className="zotero-grid">
        <div className="zotero-field"><label htmlFor={id + '-f-date'}>Date</label><input id={id + '-f-date'} type="text" value={meta.date} onChange={field('date')} disabled={busy} /></div>
        <div className="zotero-field"><label htmlFor={id + '-f-venue'}>Journal or venue</label><input id={id + '-f-venue'} type="text" value={meta.venue} onChange={field('venue')} disabled={busy} /></div>
        <div className="zotero-field"><label htmlFor={id + '-f-doi'}>DOI</label><input id={id + '-f-doi'} type="text" value={meta.doi} onChange={field('doi')} disabled={busy} /></div>
        <div className="zotero-field"><label htmlFor={id + '-f-url'}>URL</label><input id={id + '-f-url'} type="url" value={meta.url} onChange={field('url')} disabled={busy} /></div>
        <div className="zotero-field"><label htmlFor={id + '-f-tags'}>Tags, separated by commas</label><input id={id + '-f-tags'} type="text" value={tags} onChange={event => setTags(event.target.value)} disabled={busy} /></div>
      </div>
      <fieldset>
        <legend>Also add</legend>
        <label className="zotero-choice"><input type="checkbox" checked={includeNote && refs.length > 0} disabled={busy || !refs.length} onChange={event => setIncludeNote(event.target.checked)} />A note with the {refs.length} reference{refs.length === 1 ? '' : 's'} found in the text</label>
        <label className="zotero-choice"><input type="checkbox" checked={includeLink && !!meta.url} disabled={busy || !meta.url} onChange={event => setIncludeLink(event.target.checked)} />A link to the source URL</label>
        <label className="zotero-choice"><input type="checkbox" checked={includeFile && !!original} disabled={busy || !original} onChange={event => setIncludeFile(event.target.checked)} />The original file{original ? ' (' + original.name + ')' : ' (none saved)'}, uploaded to Zotero storage</label>
      </fieldset>
      {!libraryWritable && selectedLibrary && <div className="zotero-error" role="alert">This key cannot write to the selected library. Choose another library or create a key with write access.</div>}
      {fileError && <div className="zotero-error" role="alert">{fileError}</div>}
      {sending.error && <div className="zotero-error" role="alert">{sending.error}</div>}
      <p className="zotero-status" role="status" aria-live="polite">{sending.progress || 'Ready to send.'}</p>
      {sending.outcome && <div className="zotero-status">
        <p>Item {sending.outcome.itemKey} saved{sending.outcome.webUrl ? <> — <a href={sending.outcome.webUrl} target="_blank" rel="noreferrer noopener">open it on zotero.org</a></> : ''}.</p>
        {sending.outcome.warnings.length > 0 && <ul>{sending.outcome.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul>}
      </div>}
      <div className="zotero-actions">
        <button type="submit" className="zotero-primary" disabled={busy || sending.submitted || !selectedLibrary || !libraryWritable || !meta.title.trim()}>{sending.busy ? 'Sending…' : 'Send to Zotero'}</button>
        <button type="button" onClick={() => { void connection.disconnect(); sending.reset(); }} disabled={busy}>Forget key</button>
      </div>
    </form>}
  </section>;
}

/** Depth-first order with indentation depth, children after their parent. */
export function orderCollections(collections: { key: string; name: string; parent: string | null }[]): { key: string; name: string; depth: number }[] {
  const byParent = new Map<string | null, { key: string; name: string; parent: string | null }[]>();
  for (const entry of collections) {
    const list = byParent.get(entry.parent) ?? [];
    list.push(entry);
    byParent.set(entry.parent, list);
  }
  const known = new Set(collections.map(entry => entry.key));
  const out: { key: string; name: string; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number) => {
    for (const entry of (byParent.get(parent) ?? []).sort((a, b) => a.name.localeCompare(b.name))) {
      if (seen.has(entry.key)) continue;
      seen.add(entry.key);
      out.push({ key: entry.key, name: entry.name, depth });
      walk(entry.key, depth + 1);
    }
  };
  walk(null, 0);
  // Orphans whose parent is not in the list are shown at the top level.
  for (const entry of collections) if (!seen.has(entry.key) && entry.parent && !known.has(entry.parent)) { seen.add(entry.key); out.push({ key: entry.key, name: entry.name, depth: 0 }); walk(entry.key, 1); }
  return out;
}
