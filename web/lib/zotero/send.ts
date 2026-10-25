// One "send to Zotero" run: the item, then its note and link children in
// one write, then (optionally) an imported file through the three-step
// upload. A failed child never loses the item: it is reported as a warning
// with the item key.

import { ZoteroApi, ZoteroApiError, itemWebUrl, keyAt, type KeyInfo, type ZoteroLibraryRef } from './api';
import { importedFileAttachment, linkedUrlAttachment } from './mapping';
import { md5OfBlob } from './md5';

export type SendPlan = {
  library: ZoteroLibraryRef;
  item: Record<string, unknown>;
  noteHtml?: string | null;
  link?: { url: string; title: string } | null;
  file?: { blob: Blob; filename: string; contentType: string; title: string } | null;
};
export type SendOutcome = { itemKey: string; noteKey?: string; linkKey?: string; fileKey?: string; webUrl: string | null; warnings: string[] };

export function describeError(reason: unknown): string {
  if (reason instanceof ZoteroApiError) return reason.message;
  if (reason instanceof Error) return reason.message || 'Unexpected error.';
  return String(reason);
}

/** Run the plan; `onProgress` receives short status sentences for a live region. */
export async function sendToZotero(api: ZoteroApi, info: KeyInfo | null, plan: SendPlan, onProgress: (message: string) => void = () => {}): Promise<SendOutcome> {
  onProgress('Creating the item in Zotero…');
  const itemKey = keyAt(await api.createItems(plan.library, [plan.item]), 0);
  const outcome: SendOutcome = { itemKey, webUrl: info ? itemWebUrl(info, plan.library, itemKey) : null, warnings: [] };
  const children: { kind: 'note' | 'link'; object: Record<string, unknown> }[] = [];
  if (plan.noteHtml) children.push({ kind: 'note', object: { itemType: 'note', parentItem: itemKey, note: plan.noteHtml, tags: [], relations: {} } });
  if (plan.link) children.push({ kind: 'link', object: linkedUrlAttachment(itemKey, plan.link.url, plan.link.title) });
  if (children.length) {
    onProgress('Adding the references note and link…');
    try {
      const result = await api.createItems(plan.library, children.map(child => child.object));
      children.forEach((child, index) => {
        try { const key = keyAt(result, index); if (child.kind === 'note') outcome.noteKey = key; else outcome.linkKey = key; }
        catch (reason) { outcome.warnings.push((child.kind === 'note' ? 'The references note' : 'The source link') + ' was not added: ' + describeError(reason)); }
      });
    } catch (reason) { outcome.warnings.push('The item was created, but its note and link were not: ' + describeError(reason)); }
  }
  if (plan.file) {
    try {
      onProgress('Preparing the file upload…');
      const md5 = await md5OfBlob(plan.file.blob);
      const mtimeMs = Date.now();
      const attachment = importedFileAttachment(itemKey, { title: plan.file.title, filename: plan.file.filename, contentType: plan.file.contentType, md5, mtimeMs });
      const fileKey = keyAt(await api.createItems(plan.library, [attachment]), 0);
      outcome.fileKey = fileKey;
      const authorization = await api.authorizeUpload(plan.library, fileKey, { md5, filename: plan.file.filename, filesize: plan.file.blob.size, mtimeMs });
      if (!authorization.exists) {
        onProgress('Uploading the file to Zotero storage…');
        await api.uploadFile(authorization.target, plan.file.blob, plan.file.filename);
        await api.registerUpload(plan.library, fileKey, authorization.target.uploadKey);
      }
    } catch (reason) { outcome.warnings.push('The item was created, but the file was not uploaded: ' + describeError(reason)); }
  }
  onProgress('Saved to Zotero.');
  return outcome;
}
