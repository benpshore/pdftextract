/** Actual adapter/CSV with captured native bibliography and synthetic resolver outcomes.
 * These are contract checks, never a successful registry request claim.
 */
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {createRequire,Module} from 'node:module';
import {fileURLToPath} from 'node:url';
const require=createRequire(import.meta.url),web=fileURLToPath(new URL('../',import.meta.url));
const {build}=require(require.resolve('esbuild',{paths:[require.resolve('vite')]}));
const built=await build({stdin:{contents:"export {adaptGrobidResult} from './lib/scholarly-adapter';export {citationCsvRows,serializeCitationCsv} from './lib/citations';",resolveDir:web},bundle:true,write:false,platform:'node',format:'cjs'});
const compiled=new Module(web+'synthetic-scholarly-resolution.cjs');compiled.filename=web+'synthetic-scholarly-resolution.cjs';compiled._compile(built.outputFiles[0].text,compiled.filename);
const {adaptGrobidResult,citationCsvRows,serializeCitationCsv}=compiled.exports;
const grobidJson=await readFile(new URL('../tests/fixtures/scholarly/grobid-pr206-fixed-live.json',import.meta.url),'utf8'),native=JSON.parse(grobidJson),checks=[];
const sha=value=>createHash('sha256').update(value).digest('hex');
const record={id:'synthetic-resolution-contract',sha256:native.source_sha256,original_name:'original source.pdf',source_url:null};
function resolution(attempts,resolved=null){return {path:'synthetic-source.pdf',sha256:record.sha256,backend:{name:'synthetic-native-contract',version:'fixture',config_digest:''},status:'found',extraction_status:'partial',total_pages:2,pages_scanned:2,section_page:2,heading:'References',references:native.citations.map((citation,index)=>({index:index+1,label:null,raw:citation.raw,authors:citation.authors,title:null,year:null,venue:null,volume:null,issue:null,pages:null,doi:citation.identifiers.DOI?.[0]??citation.identifiers.doi?.[0]??null,arxiv_id:null,url:null,page:2,anchor:null,doi_link:null,attempts,resolved})),warnings:['Synthetic resolver contract; no registry request executed.'],assessment:{},plausible:true,paper:null,resolution:{},elapsed_ms:1,error:null};}
async function adapt(value){const raw=JSON.stringify(value);const result=await adaptGrobidResult({grobidJson,record,generatedAt:'2026-10-04T00:00:00Z',nativeResolutionJson:raw});assert.equal(result.evidence.native_resolution.raw_json,raw);assert.equal(result.evidence.native_resolution.raw_json_sha256,sha(raw));assert.equal(result.evidence.grobid.raw_json,grobidJson);assert.equal(result.bibliography.source.original_name,record.original_name);return result;}
const attempt=outcome=>({method:'printed',doi:null,outcome,detail:outcome==='error'?'request failed':null});
for(const outcomes of [['error'],['error','not_found'],['not_found','error']]){
  const result=await adapt(resolution(outcomes.map(attempt)));assert(result.bibliography.references.items.every(reference=>reference.resolution.status==='unavailable'&&reference.resolution.providers.length===0));
  assert(result.bibliography.references.items.every(reference=>/request failed|request.*failed/.test(reference.resolution.note)));
  const baseline=await adaptGrobidResult({grobidJson,record,generatedAt:'2026-10-04T00:00:00Z'});
  for(let i=0;i<result.bibliography.references.items.length;i++){const {resolution:ignored,...actual}=result.bibliography.references.items[i],{resolution:prior,...expected}=baseline.bibliography.references.items[i];assert.deepEqual(actual,expected);}
  const rows=citationCsvRows(result.bibliography),csv=serializeCitationCsv(rows);assert(rows.every(row=>row.cells.includes('unavailable')));assert(csv.includes('unavailable'));assert(!csv.includes('"resolved"'));
  checks.push(outcomes.join('/')+' stays unavailable in saved projection/CSV with exact attempts, extracted identifiers and native evidence retained');
}
for(const outcome of ['not_found','mismatch','ambiguous']){const result=await adapt(resolution([attempt(outcome)]));assert(result.bibliography.references.items.every(reference=>reference.resolution.status==='unresolved'));checks.push(outcome+' remains distinct from failed requests');}
const unrequested=await adapt(resolution([]));assert(unrequested.bibliography.references.items.every(reference=>reference.resolution.status==='not-requested'));checks.push('no attempts remains not-requested');
const recovered=resolution([attempt('error'),attempt('verified')]);for(const reference of recovered.references)reference.resolved={doi:reference.doi,pmid:null,pmcid:null,title:null,authors:[],year:null,venue:null,source:'crossref',method:'printed',score:1};
assert((await adapt(recovered)).bibliography.references.items.every(reference=>reference.resolution.status==='resolved'&&reference.resolution.providers[0]==='crossref'));checks.push('a later explicitly accepted compatible result survives an earlier failed request');
recovered.references[0].resolved.doi='10.0000/conflicting';assert.equal((await adapt(recovered)).bibliography.references.items[0].resolution.status,'unavailable');checks.push('conflicting accepted identifiers remain withheld');
console.log(JSON.stringify({checks,passed:checks.length,scope:'Actual adapter and CSV; real captured bibliography, synthetic source-bound resolver contract outcomes. No new live/native/registry/browser qualification.'},null,2));
