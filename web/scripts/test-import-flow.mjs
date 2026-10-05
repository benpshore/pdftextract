import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {build}=require(require.resolve('esbuild',{paths:[require.resolve('vite')]}));
const built=await build({entryPoints:['lib/upload-client.ts'],bundle:true,write:false,format:'esm',platform:'node'});
const {detectFileKind,uploadOriginal,saveExtracted}=await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}`);
for(const [content,name,mime,kind] of [
 ['<!DOCTYPE html><html><body>Readable article</body></html>','renamed.pdf','application/pdf','html'],
 ['%PDF-1.7\n','renamed.html','text/html','pdf'],
 ['<?php echo "text";','source.php','','text'],
 ['<html>PHP-generated article</html>','index.php','','html'],
 ['body { color: red; }','style.css','','css'],
 ['<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"/>','feed.xml','','feed']
])assert.equal(await detectFileKind(new File([content],name,{type:mime})),kind);
const record={id:'record',title:'Large page',kind:'html',source_url:null,original_name:'large.html',status:'uploaded',engine:'',created_at:new Date().toISOString(),sha256:'abc',bytes:9*1024*1024};
const calls=[];const realFetch=globalThis.fetch;
try{
 globalThis.fetch=async(url,options={})=>{
  calls.push({url,method:options.method,bytes:options.body?.size});
  if(url==='/api/uploads')return Response.json({session:'session',chunkSize:8*1024*1024});
  if(options.method==='PUT')return Response.json({partNumber:Number(new URL(url,'https://test').searchParams.get('part')),etag:'verified'});
  if(options.method==='POST')return Response.json(record);
  throw new Error('Unexpected request');
 };
 const large=new File(['<!DOCTYPE html>',new Uint8Array(9*1024*1024)],'large.html',{type:'text/html'});
 let progress=0;assert.equal((await uploadOriginal(large,{onProgress:value=>progress=value})).id,'record');
 assert.equal(progress,1);assert.deepEqual(calls.filter(c=>c.method==='PUT').map(c=>c.bytes),[8*1024*1024,1024*1024+15]);
 assert.equal(calls.filter(c=>c.method==='POST').length,2);
 calls.length=0;const abort=new AbortController();
 globalThis.fetch=async(url,options={})=>{
  calls.push({url,method:options.method});
  if(url==='/api/uploads')return Response.json({session:'cancelled',chunkSize:8*1024*1024});
  if(options.method==='PUT'){abort.abort();return Response.json({partNumber:1,etag:'verified'});}
  if(options.method==='DELETE')return new Response(null,{status:204});
  throw new Error('An aborted transfer attempted to commit');
 };
 await assert.rejects(uploadOriginal(large,{signal:abort.signal}),{name:'AbortError'});
 assert.equal(calls.at(-1).method,'DELETE');
 calls.length=0;let commits=0;
 globalThis.fetch=async(url,options={})=>{
  calls.push({url,method:options.method});
  if(url==='/api/uploads')return Response.json({session:'recovery',chunkSize:8*1024*1024});
  if(options.method==='PUT')return Response.json({partNumber:1,etag:'verified'});
  if(options.method==='POST'){commits++;throw new TypeError('Response connection lost after commit');}
  return Response.json({completed:{saved:true}});
 };
 await saveExtracted(record,{title:'Result',text:'saved',links:[],warnings:[],engine:'HTML extractor',status:'partial'});
 assert.equal(commits,1);assert.equal(calls.at(-1).method,undefined);
 console.log('Import routing, >8 MiB multipart transfer, cancellation, and lost-save-response recovery passed.');
}finally{globalThis.fetch=realFetch;}

// Folder traversal: the three browser sources and the shared import policy, with no DOM.
{
 const folderBuild=await build({entryPoints:['lib/folder-traversal.ts'],bundle:true,write:false,format:'esm',platform:'node'});
 const T=await import(`data:text/javascript;base64,${Buffer.from(folderBuild.outputFiles[0].text).toString('base64')}`);
 const named=(path,content='x',type='')=>{const file=new File([content],path.split('/').pop(),{type});Object.defineProperty(file,'webkitRelativePath',{value:path});return file;};
 for(const [name,hidden] of [['.DS_Store',true],['._paper.pdf',true],['Thumbs.db',true],['thumbs.DB',true],['desktop.ini',true],['__MACOSX',true],['~$draft.docx',true],['$RECYCLE.BIN',true],['paper.pdf',false],['notes.txt',false]])assert.equal(T.isHiddenName(name),hidden,name);
 for(const [path,kind] of [['a/b.PDF','pdf'],['a/b.tar.gz','archive'],['x.md','text'],['x.jpeg','image'],['x.docx','office'],['x.mp4',null],['noext',null],['trailing.',null],['.hidden',null]])assert.equal(T.classify(path),kind,path);
 const pdf='%PDF-1.7\n1 0 obj<<>>endobj\n%%EOF';
 const list=T.enumerateFileList([named('root/notes.txt','notes'),named('root/.DS_Store'),named('root/sub/._b.pdf'),named('root/sub/b.pdf',pdf+'b'),named('root/sub/deep/c.pdf',pdf+'c'),named('root/a.pdf',pdf),named('root/movie.mp4','mp4 bytes'),named('root/dup/a-copy.pdf',pdf),named('root/Thumbs.db'),named('root/same-size.pdf','%PDF-1.7\n1 0 obj<<>>endobj\n%%EOG')]);
 assert.deepEqual(list.hidden.map(h=>h.path),['root/.DS_Store','root/sub/._b.pdf','root/Thumbs.db']);
 assert.deepEqual(list.candidates.map(c=>c.path),['root/a.pdf','root/dup/a-copy.pdf','root/movie.mp4','root/notes.txt','root/same-size.pdf','root/sub/b.pdf','root/sub/deep/c.pdf']);
 const events=[];const result=await T.collectFolder('root',list,{onProgress:event=>events.push(event)});
 assert.deepEqual(result.files.map(f=>f.path),['root/a.pdf','root/same-size.pdf','root/sub/b.pdf','root/sub/deep/c.pdf','root/notes.txt'],'PDFs first, then scan order');
 assert.deepEqual(result.skipped.map(s=>[s.kind,s.path]),[['hidden','root/.DS_Store'],['hidden','root/sub/._b.pdf'],['hidden','root/Thumbs.db'],['duplicate','root/dup/a-copy.pdf'],['unsupported','root/movie.mp4']]);
 assert.match(result.skipped.find(s=>s.kind==='duplicate').reason,/Same content as root\/a\.pdf/);
 assert.match(result.skipped.find(s=>s.kind==='unsupported').reason,/Unsupported file type/);
 assert.equal(result.files.find(f=>f.path==='root/same-size.pdf').digest.startsWith('sha256:'),true,'same-size files are hashed and kept when they differ');
 assert.equal(result.files.find(f=>f.path==='root/notes.txt').digest,undefined,'unique sizes are never hashed');
 assert.equal(result.bytes,result.files.reduce((sum,f)=>sum+f.size,0));assert.equal(result.truncated,false);
 assert.equal(events.at(-1).phase,'preparing');assert.equal(events.at(-1).total,7);assert.equal(events.at(-1).prepared,7);assert.match(events.at(-1).label,/7 of 7 files \(\d+ B\)/);
 assert.match(result.message,/^5 files queued from root \(\d+ B\); 4 PDFs first; 1 unsupported, 1 duplicate skipped, 3 hidden skipped\.$/);
 const limited=await T.collectFolder('root',list,{limits:{maxFileBytes:10,maxFiles:3}});
 assert.deepEqual(limited.skipped.filter(s=>s.kind==='too-large').map(s=>s.path),['root/a.pdf','root/dup/a-copy.pdf']);
 assert.match(limited.skipped.find(s=>s.kind==='too-large').reason,/up to 10 B/);
 assert.equal(limited.truncated,true);assert.match(limited.message,/Stopped at the 3-file limit after 3 of 7 files/);
 const sameSize=await T.collectFolder('root',T.enumerateFileList([named('root/x.pdf',pdf),named('root/y.pdf',pdf.slice(0,-1)+'!')]));
 assert.equal(sameSize.files.length,2,'equal size, different bytes: both kept');
 const samePath=await T.collectFolder('two roots',{candidates:[...T.enumerateFileList([named('root/a.pdf',pdf)]).candidates,...T.enumerateFileList([named('root/a.pdf',pdf+'2')]).candidates]});
 assert.deepEqual(samePath.skipped.map(s=>[s.kind,s.reason]),[['duplicate','Same relative path was already queued.']]);
 const controller=new AbortController();
 await assert.rejects(T.collectFolder('root',list,{signal:controller.signal,onProgress:()=>controller.abort()}),error=>error.name==='AbortError');
 const unreadable=await T.collectFolder('root',{candidates:[{path:'root/gone.pdf',name:'gone.pdf',open:async()=>{throw new Error('NotFoundError');}}]});
 assert.deepEqual(unreadable.skipped.map(s=>[s.kind,s.reason]),[['unreadable','Could not read this file: NotFoundError']]);
 assert.equal(await T.digestFile(new File([new Uint8Array(9*1024*1024)],'big'),8*1024*1024).then(d=>d.startsWith('sampled-sha256:9437184:')),true);
 // File System Access handles (Chrome/Edge): recursive, sorted, hidden directories pruned, permission re-use.
 const fileHandle=(name,content)=>({kind:'file',name,getFile:async()=>new File([content],name)});
 const directory=(name,children)=>({kind:'directory',name,values:async function*(){yield* children;}});
 const tree=directory('Papers',[fileHandle('z.txt','z'),directory('.git',[fileHandle('HEAD','ref')]),directory('2024',[fileHandle('second.pdf',pdf+'2'),directory('nested',[fileHandle('third.pdf',pdf+'3')]),fileHandle('.DS_Store','')]),fileHandle('first.pdf',pdf)]);
 const walked=await T.enumerateDirectoryHandle(tree);
 assert.deepEqual(walked.candidates.map(c=>c.path),['Papers/first.pdf','Papers/z.txt','Papers/2024/second.pdf','Papers/2024/nested/third.pdf']);
 assert.deepEqual(walked.hidden.map(h=>h.path),['Papers/.git','Papers/2024/.DS_Store']);
 const collected=await T.collectFolder('Papers',walked);assert.deepEqual(collected.files.map(f=>f.path),['Papers/first.pdf','Papers/2024/second.pdf','Papers/2024/nested/third.pdf','Papers/z.txt']);
 const capped=await T.enumerateDirectoryHandle(tree,{limits:{maxEntries:2}});assert.equal(capped.truncated,true);assert.match((await T.collectFolder('Papers',capped)).message,/scan stopped after 2 entries/);
 const prompts=[];const granted={...tree,queryPermission:async()=>'prompt',requestPermission:async()=>{prompts.push('asked');return 'granted';}};
 assert.equal(await T.ensureReadable(granted),true);assert.deepEqual(prompts,['asked']);
 assert.equal(await T.ensureReadable({...tree,queryPermission:async()=>'granted',requestPermission:async()=>{throw new Error('must not re-prompt');}}),true);
 assert.equal(await T.ensureReadable({...tree,queryPermission:async()=>'prompt',requestPermission:async()=>'denied'}),false);
 const abortWalk=new AbortController();abortWalk.abort();await assert.rejects(T.enumerateDirectoryHandle(tree,{signal:abortWalk.signal}),error=>error.name==='AbortError');
 // Drag-and-drop entries: readEntries is called until an empty batch; a failing directory is reported, not fatal.
 const fileEntry=(name,content)=>({isFile:true,isDirectory:false,name,file:done=>done(new File([content],name))});
 const batches=[];const dirEntry=(name,children,fail=false)=>({isFile:false,isDirectory:true,name,createReader:()=>{let index=0;return {readEntries:(done,reject)=>{if(fail){reject(new DOMException('Not readable','NotReadableError'));return;}const batch=children.slice(index,index+2);index+=2;batches.push(`${name}:${batch.length}`);done(batch);}};}});
 const dropped=dirEntry('Drop',[fileEntry('c.pdf',pdf+'c'),fileEntry('.hidden','x'),dirEntry('broken',[],true),dirEntry('sub',[fileEntry('e.txt','e'),fileEntry('d.pdf',pdf+'d')]),fileEntry('a.pdf',pdf)]);
 const scanned=await T.enumerateDropEntries([dropped]);
 assert.deepEqual(scanned.candidates.map(c=>c.path),['Drop/a.pdf','Drop/c.pdf','Drop/sub/d.pdf','Drop/sub/e.txt']);
 assert.deepEqual(scanned.hidden.map(h=>h.path),['Drop/.hidden']);assert.deepEqual(scanned.unreadable.map(u=>u.path),['Drop/broken']);
 assert.deepEqual(batches,['Drop:2','Drop:2','Drop:1','Drop:0','sub:2','sub:0'],'every reader is drained to its empty batch');
 const fromDrop=await T.collectFolder('Drop',scanned);assert.deepEqual(fromDrop.files.map(f=>f.path),['Drop/a.pdf','Drop/c.pdf','Drop/sub/d.pdf','Drop/sub/e.txt']);
 assert.equal(fromDrop.skipped.find(s=>s.kind==='unreadable').reason,'Folder could not be read: Not readable');
 const items=T.dropEntries([{kind:'file',webkitGetAsEntry:()=>dropped},{kind:'file',webkitGetAsEntry:()=>null,getAsFile:()=>new File(['loose'],'loose.txt')},{kind:'string'}]);
 assert.equal(items.hasDirectory,true);assert.equal(items.entries.length,1);assert.deepEqual(items.files.map(f=>f.name),['loose.txt']);
 console.log('Folder traversal: FileList, directory handles, drop entries, hidden/unsupported/size/count/duplicate policy, progress and cancellation passed.');
}
