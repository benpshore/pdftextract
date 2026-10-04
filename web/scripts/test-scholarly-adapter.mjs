/** Actual adapter, independently authored PDF and captured/retained live native artifacts. */
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {createRequire,Module} from 'node:module';
import {fileURLToPath} from 'node:url';
const require=createRequire(import.meta.url),web=fileURLToPath(new URL('../',import.meta.url));
const {build}=require(require.resolve('esbuild',{paths:[require.resolve('vite')]}));
const compiled=new Module(web+'synthetic-scholarly-adapter.cjs');compiled.filename=web+'synthetic-scholarly-adapter.cjs';compiled.paths=Module._nodeModulePaths(web);
compiled._compile((await build({entryPoints:[web+'lib/scholarly-adapter.ts'],bundle:true,write:false,platform:'node',format:'cjs',packages:'external'})).outputFiles[0].text,compiled.filename);
const {adaptGrobidResult}=compiled.exports;
const sha=value=>createHash('sha256').update(value).digest('hex');
const captured=await readFile(new URL('../tests/fixtures/scholarly/grobid-pr206-fixed-live.json',import.meta.url),'utf8');
const original=JSON.parse(captured),record={id:'synthetic-id',sha256:original.source_sha256,original_name:'original source.pdf',source_url:null};
const adapt=(raw,source=record)=>adaptGrobidResult({grobidJson:raw,record:source,generatedAt:'2026-10-04T00:00:00Z'});
const namingModule=new Module(web+'synthetic-scholarly-naming.cjs');namingModule.filename=web+'synthetic-scholarly-naming.cjs';namingModule.paths=Module._nodeModulePaths(web);
namingModule._compile((await build({entryPoints:[web+'lib/scholarly-naming.ts'],bundle:true,write:false,platform:'node',format:'cjs'})).outputFiles[0].text,namingModule.filename);
const {scholarlyDisplayName:name}=namingModule.exports;
assert.equal(name([' Unicode 李 title '],['Zoë Example','李 Sample'],'original.pdf').value,'Zoë Example et al. — Unicode 李 title');
assert.equal(name(['Title'],[],'original.pdf').value,'Title');assert.equal(name([],['Author'],'original.pdf').state,'fallback');
assert.equal(name(['Conflicting','Title'],['Author'],'original.pdf').state,'ambiguous');assert.equal(name(['Title','Title'],[],'original.pdf').state,'derived');
const corrected=name(['Corrected title'],['Author'],'original.pdf');assert.equal(corrected.original_name,'original.pdf');assert.equal(name(['Title'],[],'other.pdf').value,name(['Title'],[],'original.pdf').value);
const checks=['metadata-derived Unicode names, missing/conflicting fields, duplicate titles, correction and display collisions preserve original-name provenance'];
for(const [name,raw] of [['captured',captured],['retained live',await readFile(new URL('../../docs/validation/scholarly-local-checkpoint-20261004/native-grobid.json',import.meta.url),'utf8')]]){
  const native=JSON.parse(raw),result=await adapt(raw,{...record,sha256:native.source_sha256});
  assert.equal(result.bibliography.references.items.length,2);assert.equal(result.bibliography.mentions.state,'unavailable');
  assert.equal(result.evidence.grobid.raw_json,raw);assert.equal(result.evidence.grobid.raw_tei,native.raw_tei);
  assert.equal(result.evidence.grobid.raw_json_sha256,sha(raw));assert.deepEqual(result.warnings,native.warnings);
  assert.equal(result.bibliography.source.original_name,record.original_name);
  assert.equal(result.naming.original_name,record.original_name);assert.equal(result.bibliography.document.display_name,result.naming.value);assert.deepEqual(result.naming.reported_authors,native.header.authors);
  for(let i=0;i<native.citations.length;i++){const pages=[...new Set(native.citations[i].coordinates.map(box=>box.page))];assert.equal(result.bibliography.references.items[i].page,pages.length===1?pages[0]:undefined);}
  checks.push(name+' artifact maps actual references, preserves byte-exact evidence, warnings and filename, invents no geometry or relationships');
}
async function reject(label,mutate){const value=structuredClone(original);mutate(value);await assert.rejects(adapt(JSON.stringify(value)),/Scholarly adapter:/);checks.push(label);}
await reject('rejects source-hash mismatch',v=>v.source_sha256='0'.repeat(64));
await reject('rejects raw TEI hash mismatch',v=>v.tei_sha256='0'.repeat(64));
await reject('rejects altered citation title',v=>v.citations[0].titles[0]='Invented title');
await reject('rejects altered header metadata',v=>v.header.titles[0]='Invented header');
await reject('rejects fabricated coordinates on empty attributes',v=>v.elements.find(e=>e.attributes.coords==='').coordinates.push({page:1,x:1,y:1,width:1,height:1}));
await reject('rejects shifted TEI range',v=>v.citations[0].source_range[0]++);
await reject('rejects duplicate native ranges',v=>v.elements.push(v.elements[0]));
await reject('rejects external entities without resolving XML',v=>{v.raw_tei='<!DOCTYPE TEI [<!ENTITY x SYSTEM "http://fixture.invalid/">]>'+v.raw_tei;v.tei_sha256=sha(v.raw_tei);});
await reject('rejects mismatched XML closing tags',v=>{v.raw_tei=v.raw_tei.replace('</TEI>','</WRONG>');v.tei_sha256=sha(v.raw_tei);});
await reject('rejects undeclared XML prefixes',v=>{v.raw_tei=v.raw_tei.replace('<title','<bogus:title');v.tei_sha256=sha(v.raw_tei);});
await reject('rejects malformed entities',v=>{v.raw_tei=v.raw_tei.replace('Synthetic','&unknown;Synthetic');v.tei_sha256=sha(v.raw_tei);});
await reject('rejects duplicate attributes',v=>{v.raw_tei=v.raw_tei.replace('<TEI ','<TEI duplicate="x" duplicate="y" ');v.tei_sha256=sha(v.raw_tei);});
await reject('rejects out-of-range character references',v=>{v.raw_tei=v.raw_tei.replace('Synthetic','&#x110000;Synthetic');v.tei_sha256=sha(v.raw_tei);});
const large=structuredClone(original);large.raw_tei=large.raw_tei.replace('</TEI>','<!--'+'x'.repeat(2*1024*1024)+'--></TEI>');large.tei_sha256=sha(large.raw_tei);
const baseline=process.memoryUsage().heapUsed;await adapt(JSON.stringify(large));assert(process.memoryUsage().heapUsed-baseline<64*1024*1024);checks.push('2 MiB inert XML text uses sparse boundary offsets within a 64 MiB additional heap budget');
const crlf=structuredClone(original),bytes=Buffer.from(original.raw_tei);
const shifted=offset=>offset+bytes.subarray(0,offset).filter(byte=>byte===10).length;
for(const item of [...crlf.elements,...crlf.citations])item.source_range=item.source_range.map(shifted);
crlf.raw_tei=original.raw_tei.replaceAll('\n','\r\n');crlf.tei_sha256=sha(crlf.raw_tei);
const crlfResult=await adapt(JSON.stringify(crlf));assert.deepEqual(crlfResult.bibliography,(await adapt(captured)).bibliography);assert.equal(crlfResult.evidence.grobid.raw_tei,crlf.raw_tei);checks.push('CRLF canonical byte ranges retain raw evidence while XML text normalizes like the native parser');
console.log(JSON.stringify({checks,passed:checks.length,scope:'Actual adapter; captured and previously retained live artifacts. No native/resolver/network execution.'},null,2));
