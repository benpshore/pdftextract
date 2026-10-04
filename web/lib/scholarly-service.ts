import {env} from 'cloudflare:workers';
import {boundedBody, ownedRecord, storage} from './server';
import {assertDocumentNotDeleted} from './document-lifecycle';
import {adaptGrobidResult} from './scholarly-adapter';
import type {DocumentRow, Extracted} from './types';

export type ScholarlyMode = 'live-native' | 'captured-native';
type RuntimeHealth = {available:boolean; mode:ScholarlyMode|null; reason?:string; resolver:boolean; max_input_bytes:number; max_output_bytes:number};
// These limits apply only to this opt-in local adapter, never to ordinary imports.
const LOCAL_BODY_LIMIT = 32 * 1024 * 1024;
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;
const headers = {'Cache-Control':'private, no-store'};
function fail(message:string, status=400):never {throw new Response(message,{status,headers});}
function signalFor(signal:AbortSignal, ms:number){return AbortSignal.any([signal,AbortSignal.timeout(ms)]);}
export async function scholarlyHealth(signal:AbortSignal):Promise<RuntimeHealth> {
  const unavailable = (reason:string):RuntimeHealth => ({available:false,mode:null,reason,resolver:false,max_input_bytes:LOCAL_BODY_LIMIT,max_output_bytes:LOCAL_BODY_LIMIT});
  if(!env.SCHOLARLY)return unavailable('No scholarly runtime is connected.');
  try {
    const response=await env.SCHOLARLY.fetch(new Request('http://scholarly/health',{signal:signalFor(signal,5000)}));
    if(!response.ok)return unavailable('The local scholarly runtime is unavailable.');
    const health=JSON.parse(new TextDecoder().decode(await boundedBody(response,16384)));
    if(health.status!=='ready'||!['live-native','captured-native'].includes(health.mode))return unavailable('The local scholarly runtime is not ready.');
    const limit=(value:unknown)=>Number.isSafeInteger(value)&&(value as number)>0?Math.min(value as number,LOCAL_BODY_LIMIT):LOCAL_BODY_LIMIT;
    return {available:true,mode:health.mode,resolver:health.resolver===true,max_input_bytes:limit(health.max_input_bytes),max_output_bytes:limit(health.max_output_bytes)};
  }catch(error){signal.throwIfAborted();return unavailable('The local scholarly runtime could not be reached.');}
}
function pdf(record:Record<string,unknown>){return record.kind==='pdf' || record.mime==='application/pdf';}
export async function scholarlyStatus(record:Record<string,unknown>,signal:AbortSignal){
  if(!pdf(record))return {available:false,mode:null,reason:'Scholarly extraction requires a saved PDF.',resolver:false,baseResultKey:record.result_key??null};
  return {...await scholarlyHealth(signal),baseResultKey:record.result_key??null};
}
async function digest(bytes:Uint8Array<ArrayBuffer>){return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');}
function extracted(value:unknown):value is Extracted {
  if(!value||typeof value!=='object')return false;
  const r=value as Extracted;
  return typeof r.title==='string'&&typeof r.text==='string'&&typeof r.engine==='string'&&['ready','partial','failed'].includes(r.status)&&Array.isArray(r.links)&&Array.isArray(r.warnings)&&r.warnings.every(w=>typeof w==='string');
}
async function runtime(path:string,bytes:Uint8Array<ArrayBuffer>,mode:ScholarlyMode,signal:AbortSignal,limit:number){
  const response=await env.SCHOLARLY!.fetch(new Request('http://scholarly/'+path,{method:'POST',headers:{'Content-Type':'application/pdf'},body:bytes,signal}));
  if(!response.ok){await response.body?.cancel();fail('The local scholarly runtime failed. Your current reading is unchanged.',502);}
  if(response.headers.get('X-TPE-Scholarly-Mode')!==mode){await response.body?.cancel();fail('The scholarly runtime changed mode. Retry after checking the local runtime.',409);}
  return new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(await boundedBody(response,limit));
}
/** One request, one compare-and-swap publication. There is no independently mutable job state. */
export async function attachScholarly(record:Record<string,unknown>,user:string,baseResultKey:string|null,requestSignal:AbortSignal){
  if(!pdf(record))fail('Scholarly extraction requires a saved PDF.');
  if((record.result_key??null)!==baseResultKey)fail('A newer result is saved. Reopen the document and retry.',409);
  const health=await scholarlyHealth(requestSignal);
  if(!health.available||!health.mode)fail(health.reason||'Scholarly runtime unavailable.',503);
  const id=String(record.id),{db,bucket}=storage();
  await assertDocumentNotDeleted(id);
  const original=await bucket.get(`${id}/original`);
  if(!original)fail('The saved original is unavailable.',404);
  if(original.size>health.max_input_bytes){await original.body.cancel();fail('This PDF exceeds the configured local scholarly processing limit. Ordinary reading and the original remain available.',413);}
  const bytes=await boundedBody(new Response(original.body),health.max_input_bytes);
  requestSignal.throwIfAborted();
  if(!/^[a-f0-9]{64}$/i.test(String(record.sha256))||await digest(bytes)!==String(record.sha256).toLowerCase())fail('The saved original does not match its recorded source hash.',409);
  if(new TextDecoder().decode(bytes.slice(0,1024)).indexOf('%PDF-')<0)fail('The saved original is not a PDF.');
  const previous=baseResultKey?await bucket.get(baseResultKey):null;
  const prior=previous?JSON.parse(new TextDecoder().decode(await boundedBody(new Response(previous.body),LOCAL_BODY_LIMIT))):null;
  if(!extracted(prior))fail('Open a saved readable extraction before adding scholarly references.',409);
  const signal=signalFor(requestSignal,150000);
  const grobidJson=await runtime('grobid',bytes,health.mode,signal,health.max_output_bytes);
  let nativeResolutionJson:string|undefined, resolverWarning:string|undefined;
  if(health.resolver){
    try{nativeResolutionJson=await runtime('bibliography',bytes,health.mode,signal,health.max_output_bytes);}
    catch(error){signal.throwIfAborted();resolverWarning='Identifier resolution did not complete. Supplied bibliography remains available without verified resolution.';}
  }
  signal.throwIfAborted();
  const generatedAt=new Date().toISOString();
  const adapted=await adaptGrobidResult({grobidJson,record:record as unknown as DocumentRow,generatedAt,...(nativeResolutionJson?{nativeResolutionJson}:{} )});
  const evidenceId=crypto.randomUUID(), evidenceKey=`${id}/scholarly/${evidenceId}.json`,resultKey=`${id}/results/${crypto.randomUUID()}`;
  adapted.bibliography.source.result_key=resultKey;
  const result:Extracted={...prior,bibliography:adapted.bibliography,warnings:[...new Set([...prior.warnings,...adapted.warnings,...(resolverWarning?[resolverWarning]:[])])],metadata:{...prior.metadata,scholarly_display_name:adapted.naming,scholarly:{schema:'tpe.scholarly-attachment',version:1,evidence_id:evidenceId,mode:health.mode,generated_at:generatedAt,has_resolution:!!nativeResolutionJson}}};
  let committed=false;
  try{
    signal.throwIfAborted();await assertDocumentNotDeleted(id);await ownedRecord(id,user);
    await bucket.put(evidenceKey,JSON.stringify(adapted.evidence),{httpMetadata:{contentType:'application/json'}});
    signal.throwIfAborted();await assertDocumentNotDeleted(id);await ownedRecord(id,user);
    await bucket.put(resultKey,JSON.stringify(result),{httpMetadata:{contentType:'application/json'}});
    signal.throwIfAborted();await assertDocumentNotDeleted(id);
    const updated=await db.prepare('UPDATE documents SET result_key=? WHERE id=? AND owner=? AND result_key IS ?').bind(resultKey,id,user,baseResultKey).run();
    if(!updated.meta.changes)fail('The document changed while scholarly processing ran. Reopen it and retry.',409);
    committed=true;
    // Cancellation after the CAS cannot roll back an already published result.
    await assertDocumentNotDeleted(id);await ownedRecord(id,user);
    return {record:{...record,result_key:resultKey},result};
  }catch(error){
    if(!committed){try{await bucket.delete([evidenceKey,resultKey]);}catch{ /* Orphaned attempt remains under the document deletion prefix. */ }}
    else{
      const deleted=await db.prepare('SELECT id FROM document_deletions WHERE id=? AND owner=?').bind(id,user).first();
      if(deleted)try{await bucket.delete([evidenceKey,resultKey]);}catch{}
    }
    throw error;
  }
}
export async function scholarlyEvidence(record:Record<string,unknown>,format:string){
  if(!['native-json','tei','resolution'].includes(format))fail('Unknown evidence format.');
  const {bucket}=storage(),id=String(record.id);
  const object=record.result_key?await bucket.get(String(record.result_key)):null;
  if(!object)fail('Scholarly evidence unavailable.',404);
  const result=JSON.parse(new TextDecoder().decode(await boundedBody(new Response(object.body),LOCAL_BODY_LIMIT)));
  const attachment=result?.metadata?.scholarly;
  if(attachment?.schema!=='tpe.scholarly-attachment'||attachment.version!==1||typeof attachment.evidence_id!=='string'||!UUID.test(attachment.evidence_id))fail('Scholarly evidence unavailable.',404);
  const raw=await bucket.get(`${id}/scholarly/${attachment.evidence_id}.json`);
  if(!raw)fail('Scholarly evidence unavailable.',404);
  const evidence=JSON.parse(new TextDecoder().decode(await boundedBody(new Response(raw.body),LOCAL_BODY_LIMIT*3)));
  if(evidence?.schema!=='tpe.scholarly-evidence'||evidence.version!==1||evidence.source?.document_id!==id||evidence.source?.sha256!==record.sha256)fail('Scholarly evidence does not match this document.',409);
  const body=format==='native-json'?evidence.grobid?.raw_json:format==='tei'?evidence.grobid?.raw_tei:evidence.native_resolution?.raw_json;
  if(typeof body!=='string')fail('This evidence artifact is unavailable.',404);
  const expected=format==='native-json'?evidence.grobid?.raw_json_sha256:format==='tei'?evidence.grobid?.tei_sha256:evidence.native_resolution?.raw_json_sha256;
  if(typeof expected!=='string'||await digest(new TextEncoder().encode(body))!==expected)fail('The scholarly evidence failed its integrity check.',409);
  const filename=format==='tei'?'grobid-evidence.tei.xml':format==='resolution'?'native-resolution.json':'native-grobid.json';
  return new Response(body,{headers:{...headers,'Content-Type':'application/octet-stream','Content-Disposition':`attachment; filename="${filename}"`,'X-Content-Type-Options':'nosniff','Content-Security-Policy':"default-src 'none'; sandbox"}});
}
