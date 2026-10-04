import {scholarlyDisplayName} from './scholarly-naming';
import { validateCitations } from './citations';
import type { BibliographicReference, CitationBundle, CitationMention } from './citations';
import type { DocumentRow } from './types';

type Coordinate = { page: number; x: number; y: number; width: number; height: number };
type Range = [number, number];
type GrobidElement = { kind: string; section: string | null; text: string; attributes: Record<string, string>; source_range: Range; coordinates: Coordinate[] };
type GrobidCitation = { xml_id: string | null; raw: string | null; titles: string[]; authors: string[]; identifiers: Record<string, string[]>; publication: Record<string, string[]>; uris: string[]; source_range: Range; coordinates: Coordinate[] };
type GrobidDocument = {
  source_sha256: string; endpoint: string; server_version: string;
  options: { timeout_ms: number; max_input_bytes: number | null; max_response_bytes: number | null; consolidation: number };
  tei_sha256: string; raw_tei: string; coverage: 'semantic_projection';
  pages: { page: number; width: number; height: number }[];
  header: { titles: string[]; authors: string[]; identifiers: Record<string, string[]> };
  elements: GrobidElement[]; citations: GrobidCitation[]; warnings: string[];
};
export type ScholarlySource = Pick<DocumentRow, 'id' | 'sha256' | 'original_name' | 'source_url'>;
export type ScholarlyEvidenceEnvelope = {
  schema: 'tpe.scholarly-evidence'; version: 1;
  source: { document_id: string; sha256: string };
  generated_at: string;
  grobid: { raw_json: string; raw_json_sha256: string; raw_tei: string; tei_sha256: string };
  native_resolution?: { raw_json: string; raw_json_sha256: string };
};
export type ScholarlyAdapterResult = { bibliography: CitationBundle; naming: ReturnType<typeof scholarlyDisplayName>; warnings: string[]; evidence: ScholarlyEvidenceEnvelope };
const encoder = new TextEncoder(), decoder = new TextDecoder('utf-8', { fatal: true });
const HASH = /^[a-f\d]{64}$/i, XML_ID = '{http://www.w3.org/XML/1998/namespace}id';
const TEI_NS = 'http://www.tei-c.org/ns/1.0', XML_NS = 'http://www.w3.org/XML/1998/namespace';
const fail = (message: string): never => { throw new Error('Scholarly adapter: ' + message); };
const has = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);

function object(value: unknown, required?: string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail('expected an object.');
  const result = value as Record<string, unknown>;
  if (required && (required.some(key => !has(result, key)) || Object.keys(result).some(key => !required.includes(key)))) fail('missing or unsupported native fields.');
  return result;
}
function text(value: unknown, nonempty = false): asserts value is string {
  if (typeof value !== 'string' || value.includes('\0') || (nonempty && !value.trim())) fail('invalid native text.');
}
function integer(value: unknown, minimum = 0): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum) fail('invalid native integer.');
}
function number(value: unknown, minimum = -1e8): asserts value is number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum || value > 1e8) fail('invalid native coordinate.');
}
function array(value: unknown): unknown[] { if (!Array.isArray(value)) fail('expected a native array.'); return value as unknown[]; }
function strings(value: unknown) { for (const item of array(value)) text(item); }
function stringMap(value: unknown, lists: boolean) { for (const item of Object.values(object(value))) { if (lists) strings(item); else text(item); } }
function utf8(value: string): Uint8Array<ArrayBuffer> {
  const bytes = encoder.encode(value);
  if (decoder.decode(bytes) !== value) fail('input is not lossless UTF-8 text.');
  return bytes;
}
async function sha256(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(byte => byte.toString(16).padStart(2, '0')).join('');
}
function range(value: unknown, tei: Uint8Array, kind: string): asserts value is Range {
  const span = array(value);
  if (span.length !== 2) fail('invalid TEI byte range.');
  integer(span[0]); integer(span[1]);
  const [start, end] = span as number[];
  if (start >= end || end > tei.length || (tei[start] & 0xc0) === 0x80 || (end < tei.length && (tei[end] & 0xc0) === 0x80)) fail('TEI byte range is outside UTF-8 boundaries.');
  const source = decoder.decode(tei.subarray(start, end));
  // Native ranges are complete element ranges, not text offsets or PDF positions.
  if (!/^[A-Za-z][A-Za-z\d]*$/.test(kind) || !new RegExp('^<(?:[A-Za-z_][\\w.-]*:)?' + kind + '(?:\\s|/?>)').test(source) || !source.endsWith('>')) fail('TEI range does not identify the reported element.');
}
function coordinates(value: unknown) {
  for (const coordinate of array(value)) {
    const item = object(coordinate, ['page', 'x', 'y', 'width', 'height']);
    integer(item.page, 1); for (const key of ['x', 'y']) number(item[key]); for (const key of ['width', 'height']) number(item[key], 0);
  }
}
function coordinateAttributes(item: GrobidElement) {
  if (!has(item.attributes, 'coords')) {
    if (item.coordinates.length) fail('coordinates have no reported attribute.');
    return;
  }
  const attribute = item.attributes.coords;
  if (!attribute.trim()) { if (item.coordinates.length) fail('empty coordinate attribute contains invented boxes.'); return; }
  const groups = attribute.split(';');
  if (groups.length !== item.coordinates.length) fail('coordinate attribute/projection disagreement.');
  for (let i = 0; i < groups.length; i++) {
    const cells = groups[i].split(',');
    if (cells.length !== 5 || cells.some(cell => !cell.trim()) || !/^\d+$/.test(cells[0])) fail('malformed nonempty coordinate attribute.');
    const values = cells.map(Number), coordinate = item.coordinates[i];
    if (values.some((value, index) => !Number.isFinite(value) || value !== [coordinate.page, coordinate.x, coordinate.y, coordinate.width, coordinate.height][index])) fail('coordinate attribute/projection disagreement.');
  }
}
function sameCoordinates(left: Coordinate[], right: Coordinate[]) {
  return left.length === right.length && left.every((coordinate, index) => (['page', 'x', 'y', 'width', 'height'] as const).every(key => coordinate[key] === right[index][key]));
}
type XmlNode = { name: string; qualified: string; namespace: string; attributes: Record<string, string>; section: string | null; source_range: Range; text: string; children: XmlNode[]; namespaces: Map<string, string> };
function xmlText(value: string): string {
  return value.replace(/&([^;]*);|&/g, (match, entity: string | undefined) => {
    if (!entity) return fail('malformed XML entity.');
    const predefined: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" };
    if (has(predefined, entity)) return predefined[entity];
    if (!/^#(?:x[\da-fA-F]+|\d+)$/.test(entity)) return fail('unsupported XML entity.');
    const code = entity.startsWith('#x') ? Number.parseInt(entity.slice(2), 16) : Number.parseInt(entity.slice(1), 10);
    if (!(code === 9 || code === 10 || code === 13 || (code >= 0x20 && code <= 0xd7ff) || (code >= 0xe000 && code <= 0xfffd) || (code >= 0x10000 && code <= 0x10ffff))) return fail('invalid XML character reference.');
    return String.fromCodePoint(code);
  });
}
/** Non-resolving XML index: no DTD, custom entities, DOM, HTML or network access. */
function indexTei(source: string): Map<string, XmlNode> {
  const byteOffsets = new Map<number, number>(); let byteOffset = 0, offset = 0;
  // Ranges use tag boundaries only. A per-character Map multiplies a large
  // text/comment payload into hundreds of MB in a memory-limited Worker.
  for (const character of source) {
    if(character==='<')byteOffsets.set(offset,byteOffset);
    const point=character.codePointAt(0)!;
    byteOffset+=point<=0x7f?1:point<=0x7ff?2:point<=0xffff?3:4;
    offset+=character.length;
    if(character==='>')byteOffsets.set(offset,byteOffset);
  }
  byteOffsets.set(source.length, byteOffset);
  const nodes = new Map<string, XmlNode>(), stack: XmlNode[] = []; let cursor = 0, roots = 0;
  function append(value: string) { if (stack.length) stack[stack.length - 1].text += value; else if (value.trim()) fail('text outside the TEI root.'); }
  function complete(node: XmlNode, end: number) {
    node.source_range[1] = byteOffsets.get(end)!;
    nodes.set(node.source_range.join(':'), node);
    if (stack.length) { stack[stack.length - 1].children.push(node); stack[stack.length - 1].text += node.text; }
  }
  while (cursor < source.length) {
    if (source[cursor] !== '<') { const end = source.indexOf('<', cursor), next = end < 0 ? source.length : end; append(xmlText(source.slice(cursor, next).replace(/\r\n?/g,'\n'))); cursor = next; continue; }
    if (source.startsWith('<!--', cursor)) { const end = source.indexOf('-->', cursor + 4); if (end < 0 || source.slice(cursor + 4, end).includes('--')) fail('invalid XML comment.'); cursor = end + 3; continue; }
    if (source.startsWith('<![CDATA[', cursor)) { const end = source.indexOf(']]>', cursor + 9); if (end < 0 || !stack.length) fail('invalid CDATA.'); append(source.slice(cursor + 9, end).replace(/\r\n?/g,'\n')); cursor = end + 3; continue; }
    if (source.startsWith('<?', cursor)) { const end = source.indexOf('?>', cursor + 2); if (end < 0 || stack.length) fail('unsupported XML processing instruction.'); cursor = end + 2; continue; }
    if (source.startsWith('<!', cursor)) fail('XML declarations other than comments/CDATA are unsupported.');
    const start = cursor, closing = source.startsWith('</', cursor);
    const nameMatch = source.slice(cursor + (closing ? 2 : 1)).match(/^([A-Za-z_][A-Za-z\d_.-]*(?::[A-Za-z_][A-Za-z\d_.-]*)?)/);
    if (!nameMatch) fail('invalid XML element name.');
    const qualified = nameMatch![1]; cursor += (closing ? 2 : 1) + qualified.length;
    if (closing) {
      const endMatch = source.slice(cursor).match(/^\s*>/); if (!endMatch) fail('invalid closing tag.'); cursor += endMatch![0].length;
      const node = stack.pop(); if (!node || node.qualified !== qualified) fail('mismatched XML element.'); complete(node!, cursor); continue;
    }
    const rawAttributes = new Map<string, string>(); let selfClosing = false;
    while (true) {
      const separator = source.slice(cursor).match(/^\s*/)?.[0] || ''; cursor += separator.length;
      if (source.startsWith('/>', cursor)) { cursor += 2; selfClosing = true; break; }
      if (source[cursor] === '>') { cursor++; break; }
      if (!separator.length) fail('XML attributes need whitespace.');
      const attribute = source.slice(cursor).match(/^([A-Za-z_][A-Za-z\d_.-]*(?::[A-Za-z_][A-Za-z\d_.-]*)?)\s*=\s*(["'])([\s\S]*?)\2/);
      if (!attribute || attribute[3].includes('<') || rawAttributes.has(attribute[1])) fail('invalid or duplicate XML attribute.');
      rawAttributes.set(attribute![1], xmlText(attribute![3].replace(/\r\n?|\n|\t/g, ' '))); cursor += attribute![0].length;
    }
    const namespaces = new Map(stack.at(-1)?.namespaces || [['xml', XML_NS]]);
    for (const [name, value] of rawAttributes) if (name === 'xmlns') namespaces.set('', value); else if (name.startsWith('xmlns:')) namespaces.set(name.slice(6), value);
    if (namespaces.get('xml') !== XML_NS) fail('invalid XML namespace binding.');
    function expanded(name: string, attribute: boolean): { local: string; namespace: string } {
      const parts = name.split(':');
      if (parts.length === 1) return { local: name, namespace: attribute ? '' : namespaces.get('') || '' };
      const namespace = namespaces.get(parts[0]); if (!namespace) fail('unbound XML prefix.'); return { local: parts[1], namespace: namespace! };
    }
    const { local: name, namespace } = expanded(qualified, false), attributes: Record<string, string> = Object.create(null);
    for (const [attribute, value] of rawAttributes) {
      if (attribute === 'xmlns' || attribute.startsWith('xmlns:')) continue;
      const expandedAttribute = expanded(attribute, true), key = expandedAttribute.namespace ? '{' + expandedAttribute.namespace + '}' + expandedAttribute.local : expandedAttribute.local;
      if (has(attributes, key)) fail('duplicate expanded XML attribute.'); attributes[key] = value;
    }
    if (!stack.length) { roots++; if (roots !== 1 || name !== 'TEI' || namespace !== TEI_NS) fail('invalid TEI root.'); }
    const node: XmlNode = { name, qualified, namespace, attributes, section: ['teiHeader', 'body', 'back', 'front'].includes(name) ? name : stack.at(-1)?.section || null, source_range: [byteOffsets.get(start)!, 0], text: '', children: [], namespaces };
    if (selfClosing) complete(node, cursor); else stack.push(node);
  }
  if (stack.length || roots !== 1) fail('unclosed TEI document.');
  return nodes;
}
function sameStringMap(left: Record<string, string>, right: Record<string, string>) { return Object.keys(left).length === Object.keys(right).length && Object.entries(left).every(([key, value]) => right[key] === value); }
function sameStringLists(left: Record<string, string[]>, right: Record<string, string[]>) { return Object.keys(left).length === Object.keys(right).length && Object.entries(left).every(([key, value]) => JSON.stringify(right[key]) === JSON.stringify(value)); }
function personName(node: XmlNode) {
  const parts = node.children.filter(child => ['forename', 'surname', 'nameLink', 'genName'].includes(child.name)).map(child => child.text.trim());
  return parts.length ? parts.join(' ') : node.text.trim();
}
function verifyCitation(citation: GrobidCitation, nodes: XmlNode[]) {
  const descendants = nodes.filter(node => node.namespace === TEI_NS && within(node.source_range, citation.source_range));
  const titles: string[] = [], authors: string[] = [], identifiers: Record<string, string[]> = Object.create(null), publication: Record<string, string[]> = Object.create(null), uris: string[] = []; let raw: string | null = null;
  for (const node of descendants) {
    if (node.name === 'title') titles.push(node.text.trim());
    else if (node.name === 'persName') authors.push(personName(node));
    else if (node.name === 'idno') (identifiers[node.attributes.type || 'unknown'] ||= []).push(node.text.trim());
    else if (['date', 'biblScope', 'publisher', 'pubPlace'].includes(node.name)) (publication[node.name === 'biblScope' ? node.attributes.unit || 'biblScope' : node.name] ||= []).push(node.attributes.when === undefined ? node.text.trim() : node.attributes.when);
    else if (['ptr', 'ref'].includes(node.name) && node.attributes.target !== undefined) uris.push(node.attributes.target);
    else if (node.name === 'note' && node.attributes.type === 'raw_reference') raw = node.text.trim();
  }
  if (JSON.stringify(titles) !== JSON.stringify(citation.titles) || JSON.stringify(authors) !== JSON.stringify(citation.authors) || JSON.stringify(uris) !== JSON.stringify(citation.uris) || raw !== citation.raw || !sameStringLists(identifiers, citation.identifiers) || !sameStringLists(publication, citation.publication)) fail('citation fields disagree with canonical TEI.');
}
function parseGrobid(rawJson: unknown): { rawJson: string; document: GrobidDocument; jsonBytes: Uint8Array<ArrayBuffer>; teiBytes: Uint8Array<ArrayBuffer> } {
  text(rawJson, true);
  const jsonBytes = utf8(rawJson);
  let input: unknown;
  try { input = JSON.parse(rawJson); } catch { fail('GROBID response is not valid JSON.'); }
  const root = object(input, ['source_sha256', 'endpoint', 'server_version', 'options', 'tei_sha256', 'raw_tei', 'coverage', 'pages', 'header', 'elements', 'citations', 'warnings']);
  for (const key of ['source_sha256', 'tei_sha256']) { text(root[key]); if (!HASH.test(root[key])) fail('invalid native SHA-256.'); }
  text(root.endpoint, true); text(root.server_version, true); text(root.raw_tei, true);
  try { const endpoint = new URL(root.endpoint); if (!['http:', 'https:'].includes(endpoint.protocol) || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) fail('invalid reported service endpoint.'); } catch { fail('invalid reported service endpoint.'); }
  if (root.coverage !== 'semantic_projection') fail('unsupported GROBID coverage.');
  const options = object(root.options, ['timeout_ms', 'max_input_bytes', 'max_response_bytes', 'consolidation']);
  integer(options.timeout_ms, 1); if (options.timeout_ms > 300_000) fail('invalid native timeout.');
  integer(options.consolidation); if (options.consolidation > 2) fail('invalid consolidation mode.');
  for (const key of ['max_input_bytes', 'max_response_bytes']) if (options[key] !== null) integer(options[key], 1);
  const teiBytes = utf8(root.raw_tei);
  // No XML parser, entity expansion or network lookup is performed by this adapter.
  if (/<!DOCTYPE|<!ENTITY/i.test(root.raw_tei)) fail('DTD/entity declarations are unsupported.');
  if (!/^\s*(?:<\?xml[^?]*\?>\s*)?<TEI\b[^>]*\bxmlns=["']http:\/\/www\.tei-c\.org\/ns\/1\.0["'][^>]*>/.test(root.raw_tei) || !/<\/TEI>\s*$/.test(root.raw_tei)) fail('unsupported TEI root.');
  const xmlNodes = indexTei(root.raw_tei), orderedNodes = [...xmlNodes.values()].sort((left, right) => left.source_range[0] - right.source_range[0]);
  const pageIds = new Set<number>();
  for (const page of array(root.pages)) {
    const item = object(page, ['page', 'width', 'height']); integer(item.page, 1); number(item.width, 0); number(item.height, 0);
    if (!item.width || !item.height || pageIds.has(item.page)) fail('invalid or duplicate page surface.'); pageIds.add(item.page);
  }
  const header = object(root.header, ['titles', 'authors', 'identifiers']); strings(header.titles); strings(header.authors); stringMap(header.identifiers, true);
  const elementRanges = new Set<string>(), xmlIds = new Set<string>();
  for (const element of array(root.elements)) {
    const item = object(element, ['kind', 'section', 'text', 'attributes', 'source_range', 'coordinates']);
    text(item.kind, true); text(item.text); if (item.section !== null && !['teiHeader', 'body', 'back', 'front'].includes(item.section as string)) fail('invalid native section.');
    stringMap(item.attributes, false); range(item.source_range, teiBytes, item.kind); coordinates(item.coordinates);
    const key = (item.source_range as number[]).join(':'); if (elementRanges.has(key)) fail('duplicate native element range.'); elementRanges.add(key);
    const xmlNode = xmlNodes.get(key);
    if (!xmlNode || xmlNode.namespace !== TEI_NS || xmlNode.name !== item.kind || xmlNode.section !== item.section || xmlNode.text.trim() !== item.text || !sameStringMap(xmlNode.attributes, item.attributes as Record<string, string>)) fail('native element disagrees with canonical TEI range, section, text or attributes.');
    coordinateAttributes(item as GrobidElement);
    const xmlId = (item.attributes as Record<string, string>)[XML_ID];
    if (xmlId !== undefined) { text(xmlId, true); if (xmlIds.has(xmlId)) fail('duplicate native XML ID.'); xmlIds.add(xmlId); }
  }
  const citationIds = new Set<string>(), citationRanges = new Set<string>();
  for (const citation of array(root.citations)) {
    const item = object(citation, ['xml_id', 'raw', 'titles', 'authors', 'identifiers', 'publication', 'uris', 'source_range', 'coordinates']);
    if (item.xml_id !== null) { text(item.xml_id, true); if (citationIds.has(item.xml_id)) fail('duplicate citation XML ID.'); citationIds.add(item.xml_id); }
    if (item.raw !== null) text(item.raw);
    for (const key of ['titles', 'authors', 'uris']) strings(item[key]); for (const key of ['identifiers', 'publication']) stringMap(item[key], true);
    range(item.source_range, teiBytes, 'biblStruct'); coordinates(item.coordinates);
    const key = (item.source_range as number[]).join(':'); if (citationRanges.has(key)) fail('duplicate citation range.'); citationRanges.add(key);
    const element = (root.elements as GrobidElement[]).find(candidate => candidate.kind === 'biblStruct' && candidate.source_range.join(':') === key);
    if (!element || (element.attributes[XML_ID] || null) !== item.xml_id || !sameCoordinates(element.coordinates, item.coordinates as Coordinate[])) fail('citation disagrees with its native element evidence.');
    verifyCitation(item as GrobidCitation, orderedNodes);
  }
  const headerNodes = orderedNodes.filter(node => node.namespace === TEI_NS && node.section === 'teiHeader'), headerIdentifiers: Record<string, string[]> = Object.create(null);
  for (const node of headerNodes) if (node.name === 'idno') (headerIdentifiers[node.attributes.type || 'unknown'] ||= []).push(node.text.trim());
  if (JSON.stringify(headerNodes.filter(node => node.name === 'title').map(node => node.text.trim())) !== JSON.stringify(header.titles) || JSON.stringify(headerNodes.filter(node => node.name === 'persName').map(personName)) !== JSON.stringify(header.authors) || !sameStringLists(headerIdentifiers, header.identifiers as Record<string, string[]>)) fail('header metadata disagrees with canonical TEI.');
  strings(root.warnings);
  return { rawJson, document: input as GrobidDocument, jsonBytes, teiBytes };
}
const within = (inner: Range, outer: Range) => inner[0] >= outer[0] && inner[1] <= outer[1];
const uniqueValue = (values: string[]) => { const unique = [...new Set(values.filter(value => value.trim()))]; return unique.length === 1 ? unique[0] : undefined; };
const onePage = (coordinates: Coordinate[]) => { const pages = [...new Set(coordinates.map(coordinate => coordinate.page))]; return pages.length === 1 ? pages[0] : undefined; };
const identifier = (citation: GrobidCitation, type: string) => uniqueValue(Object.entries(citation.identifiers).filter(([key]) => key.toLowerCase() === type).flatMap(([, values]) => values));

type NativeResolved = { doi: string | null; pmid: string | null; pmcid: string | null; title: string | null; authors: string[]; year: number | null; venue: string | null; source: string; method: string; score: number };
type NativeReference = { index: number; raw: string; doi: string | null; doi_link: string | null; attempts: { method: string; doi: string | null; outcome: string; detail: string | null }[]; resolved: NativeResolved | null };
type NativeResolution = { references: NativeReference[]; warnings: string[]; status: string; extraction_status: string };
function nullableText(value: unknown) { if (value !== null) text(value); }
function resolved(value: unknown) {
  if (value === null) return;
  const item = object(value, ['doi', 'pmid', 'pmcid', 'title', 'authors', 'year', 'venue', 'source', 'method', 'score']);
  for (const key of ['doi', 'pmid', 'pmcid', 'title', 'venue']) nullableText(item[key]); strings(item.authors);
  if (item.year !== null) integer(item.year); text(item.source, true); text(item.method, true);
  if (typeof item.score !== 'number' || !Number.isFinite(item.score) || item.score < 0 || item.score > 1) fail('invalid native resolution score.');
}
function parseNativeResolution(rawJson: unknown, sourceHash: string): { rawJson: string; bytes: Uint8Array<ArrayBuffer>; document: NativeResolution } {
  text(rawJson, true); const bytes = utf8(rawJson);
  let value: unknown; try { value = JSON.parse(rawJson); } catch { fail('native resolution response is not JSON.'); }
  const root = object(value, ['path', 'sha256', 'backend', 'status', 'extraction_status', 'total_pages', 'pages_scanned', 'section_page', 'heading', 'references', 'warnings', 'assessment', 'plausible', 'paper', 'resolution', 'elapsed_ms', 'error']);
  text(root.path); text(root.sha256);
  if (!HASH.test(root.sha256) || root.sha256.toLowerCase() !== sourceHash.toLowerCase()) fail('native resolver source hash mismatch.');
  if (!['found', 'not_found', 'failed'].includes(root.status as string) || !['complete', 'partial', 'failed', 'deferred'].includes(root.extraction_status as string)) fail('invalid native bibliography status.');
  const backend = object(root.backend, ['name', 'version', 'config_digest']); for (const key of ['name', 'version', 'config_digest']) text(backend[key]);
  for (const key of ['total_pages', 'pages_scanned', 'section_page']) if (root[key] !== null) integer(root[key]);
  nullableText(root.heading); nullableText(root.error); strings(root.warnings); resolved(root.paper);
  object(root.assessment); if (root.resolution !== null) object(root.resolution);
  if (typeof root.plausible !== 'boolean' || typeof root.elapsed_ms !== 'number' || !Number.isFinite(root.elapsed_ms) || root.elapsed_ms < 0) fail('invalid native bibliography diagnostics.');
  const indices = new Set<number>();
  for (const reference of array(root.references)) {
    const item = object(reference, ['index', 'label', 'raw', 'authors', 'title', 'year', 'venue', 'volume', 'issue', 'pages', 'doi', 'arxiv_id', 'url', 'page', 'anchor', 'doi_link', 'attempts', 'resolved']);
    integer(item.index, 1); integer(item.page, 1); if (indices.has(item.index)) fail('duplicate native reference index.'); indices.add(item.index);
    text(item.raw); strings(item.authors);
    for (const key of ['label', 'title', 'venue', 'volume', 'issue', 'pages', 'doi', 'arxiv_id', 'url', 'doi_link']) nullableText(item[key]);
    if (item.year !== null) integer(item.year);
    if (item.anchor !== null) { const anchor = object(item.anchor, ['x0', 'y0', 'x1', 'y1']); for (const key of ['x0', 'y0', 'x1', 'y1']) number(anchor[key]); }
    for (const attempt of array(item.attempts)) {
      const entry = object(attempt, ['method', 'doi', 'outcome', 'detail']); text(entry.method); text(entry.outcome); nullableText(entry.doi); nullableText(entry.detail);
    }
    resolved(item.resolved);
  }
  return { rawJson, bytes, document: value as NativeResolution };
}
function applyNativeResolution(references: BibliographicReference[], native: NativeResolution, warnings: string[]) {
  warnings.push(...native.warnings);
  for (const reference of references) {
    const matches = reference.raw?.trim() ? native.references.filter(candidate => candidate.raw === reference.raw) : [];
    if (matches.length !== 1) {
      reference.resolution = { status: 'unavailable', providers: [], note: 'Native resolver output could not be joined by a unique exact raw reference; canonical evidence is retained.' };
      continue;
    }
    const match = matches[0], result = match.resolved;
    if (native.status === 'failed' || ['failed', 'deferred'].includes(native.extraction_status)) {
      reference.resolution = { status: 'unavailable', providers: [], note: 'The supplied native bibliography stage did not complete.' }; continue;
    }
    if (!result) {
      if(match.attempts.some(attempt=>attempt.outcome==='error')){
        reference.resolution={status:'unavailable',providers:[],note:'Native resolution could not complete because a reported request failed. Extracted identifiers remain unverified; attempts are retained in canonical evidence.'};
        continue;
      }
      reference.resolution = { status: match.attempts.length ? 'unresolved' : 'not-requested', providers: [], note: match.attempts.length ? 'Native attempts supplied no accepted resolution; details remain in the canonical artifact.' : 'No native resolution attempts were supplied for this reference.' }; continue;
    }
    const doiValues = [reference.doi, match.doi, match.doi_link, result.doi].filter((value): value is string => !!value?.trim()).map(value => value.trim().toLowerCase());
    const conflictingDoi = new Set(doiValues).size > 1;
    const conflictingPmid = !!reference.pmid && !!result.pmid && reference.pmid.trim() !== result.pmid.trim();
    const provider = result.source === 'crossref' ? 'crossref' : result.source === 'europepmc' ? 'europe-pmc' : undefined;
    if (conflictingDoi || conflictingPmid || !provider) {
      reference.resolution = { status: 'unavailable', providers: [], note: 'Conflicting identifiers or an unsupported native provider prevented accepting this resolution.' };
      warnings.push('A native resolution was withheld because its identifiers conflict or its provider is unsupported by the citation projection.'); continue;
    }
    reference.resolution = { status: 'resolved', providers: [provider], note: 'Reported native resolution joined by exact raw-reference equality; canonical metadata, attempts and source fields are retained separately.' };
    // Printed/parser fields remain distinct from the resolved record in canonical evidence.
  }
}

/** Adapts existing evidence only. Never submits a document, resolves metadata or infers matches. */
export async function adaptGrobidResult(input: { grobidJson: unknown; record: ScholarlySource; generatedAt: string; nativeResolutionJson?: unknown }): Promise<ScholarlyAdapterResult> {
  const { rawJson, document, jsonBytes, teiBytes } = parseGrobid(input.grobidJson);
  const record = input.record;
  if (!record || typeof record.id !== 'string' || !record.id.trim() || typeof record.original_name !== 'string' || !record.original_name.trim() || typeof record.sha256 !== 'string' || !HASH.test(record.sha256)) fail('a saved source ID, original name and SHA-256 are required.');
  if (record.source_url !== null && typeof record.source_url !== 'string') fail('invalid source URL metadata.');
  if (document.source_sha256.toLowerCase() !== record.sha256.toLowerCase()) fail('GROBID source hash does not match the original.');
  const [jsonHash, teiHash] = await Promise.all([sha256(jsonBytes), sha256(teiBytes)]);
  if (teiHash !== document.tei_sha256.toLowerCase()) fail('raw TEI hash does not match the native result.');
  const warnings = [...document.warnings], references: BibliographicReference[] = [], referenceIds = new Map<string, string>();
  for (const citation of document.citations) {
    const id = citation.xml_id || 'grobid-range-' + citation.source_range.join('-');
    if (citation.xml_id) referenceIds.set(citation.xml_id, id);
    const scoped = document.elements.filter(element => within(element.source_range, citation.source_range));
    const title = uniqueValue(scoped.filter(element => element.kind === 'title' && element.attributes.level === 'a' && citation.titles.includes(element.text)).map(element => element.text));
    const venue = uniqueValue(scoped.filter(element => element.kind === 'title' && element.attributes.level === 'j' && citation.titles.includes(element.text)).map(element => element.text));
    const year = uniqueValue(citation.publication.date || []);
    const reference: BibliographicReference = {
      id, authors: citation.authors.filter(author => author.trim()),
      ...(title ? { title } : {}), ...(venue ? { venue } : {}), ...(year && /^\d{4}$/.test(year) ? { year } : {}),
      ...(citation.raw?.trim() ? { raw: citation.raw } : {}),
      ...(identifier(citation, 'doi') ? { doi: identifier(citation, 'doi') } : {}),
      ...(identifier(citation, 'pmid') ? { pmid: identifier(citation, 'pmid') } : {}),
      ...(uniqueValue(citation.uris) ? { url: uniqueValue(citation.uris) } : {}),
      ...(onePage(citation.coordinates) === undefined ? {} : { page: onePage(citation.coordinates) }),
      resolution: document.options.consolidation === 0 ? { status: 'not-requested', providers: [] } : { status: 'unavailable', providers: [], note: 'Server consolidation was requested; provider-specific resolution outcomes were not supplied.' },
    };
    references.push(reference);
  }
  const native = input.nativeResolutionJson === undefined ? undefined : parseNativeResolution(input.nativeResolutionJson, record.sha256);
  if (native) applyNativeResolution(references, native.document, warnings);
  const mentions: CitationMention[] = [];
  for (const element of document.elements) {
    if (element.kind !== 'ref' || element.section !== 'body' || element.attributes.type !== 'bibr') continue;
    if (!element.text.trim()) { warnings.push('A body bibliography reference has no marker text; its native evidence is retained without an invented mention.'); continue; }
    const targets = (element.attributes.target || '').trim().split(/\s+/).filter(Boolean), matched: string[] = [];
    let unmatched = targets.length === 0;
    for (const target of targets) {
      const id = /^#[^\s#]+$/.test(target) ? referenceIds.get(target.slice(1)) : undefined;
      if (id) { if (!matched.includes(id)) matched.push(id); } else unmatched = true;
    }
    if (unmatched) warnings.push('A body bibliography reference contains absent, external or unmatched targets; only explicit matched relationships are displayed.');
    const containers = document.elements.filter(candidate => candidate.section === 'body' && ['p', 's'].includes(candidate.kind) && within(element.source_range, candidate.source_range));
    containers.sort((a, b) => (a.source_range[1] - a.source_range[0]) - (b.source_range[1] - b.source_range[0]));
    mentions.push({ id: 'grobid-mention-' + element.source_range.join('-'), text: element.text, reference_ids: matched,
      ...(containers[0]?.text.trim() ? { context: containers[0].text } : {}),
      ...(onePage(element.coordinates) === undefined ? {} : { page: onePage(element.coordinates) }),
    });
  }
  const title = uniqueValue(document.header.titles);
  const naming=scholarlyDisplayName(document.header.titles,document.header.authors,record.original_name);
  const bibliography: CitationBundle = {
    schema: 'tpe.web-citations', version: 1,
    source: { document_id: record.id, sha256: document.source_sha256, original_name: record.original_name, ...(record.source_url ? { source_url: record.source_url } : {}) },
    provenance: { producer: 'grobid', grobid_version: document.server_version, generated_at: input.generatedAt },
    document: { ...(title ? { title } : {}), display_name:naming.value, authors: document.header.authors.filter(author => author.trim()) },
    references: { state: 'partial', items: references, reason: 'GROBID supplied a semantic projection; complete bibliography coverage has not been established.' },
    mentions: mentions.length ? { state: 'partial', items: mentions, reason: 'Only explicitly supplied body bibliography references are represented; occurrence coverage has not been established.' } : { state: 'unavailable', reason: 'No usable body bibliography-reference elements were supplied. In-text extraction has not been established.' },
  };
  const checked = validateCitations(bibliography, record);
  if (checked.state !== 'available') fail(checked.message);
  return { bibliography, naming, warnings, evidence: { schema: 'tpe.scholarly-evidence', version: 1,
    source: { document_id: record.id, sha256: document.source_sha256 }, generated_at: input.generatedAt,
    grobid: { raw_json: rawJson, raw_json_sha256: jsonHash, raw_tei: document.raw_tei, tei_sha256: teiHash },
    ...(native ? { native_resolution: { raw_json: native.rawJson, raw_json_sha256: await sha256(native.bytes) } } : {}),
  } };
}
