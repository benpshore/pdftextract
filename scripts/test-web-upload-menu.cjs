#!/usr/bin/env node
/** Upload disclosure and ordinary paste regressions against the real component.
 * Synthetic React/jsdom fixtures mock extraction, persistence and network.
 * Run: node scripts/test-web-upload-menu.cjs
 * Browser layout/device/storage behavior is covered separately, not inferred here.
 */
const assert = require('node:assert/strict');
const {createRequire} = require('node:module');
const Module = require('node:module');
const fs = require('node:fs');
const path = require('node:path');
const {File} = require('node:buffer');
const web = path.resolve(__dirname, '../web');
const req = createRequire(path.join(web, 'package.json'));
const {JSDOM} = req('jsdom');
const dom = new JSDOM('<div id="root"></div>', {url:'https://reader.test/'});
for (const key of ['window','document','DOMParser','HTMLElement','Element','NodeFilter','HTMLInputElement','CustomEvent','MutationObserver','getComputedStyle','Node','Event','MouseEvent','history','location']) global[key] = key === 'window' ? dom.window : dom.window[key];
Object.defineProperty(global, 'navigator', {value:dom.window.navigator, configurable:true});
global.File = File;
global.IS_REACT_ACT_ENVIRONMENT = true;
global.requestAnimationFrame = callback => { queueMicrotask(callback); return 1; };
window.scrollTo = () => {};
Object.defineProperty(dom.window.HTMLInputElement.prototype,'webkitdirectory',{value:false,writable:true,configurable:true});
const React = req('react'), {createRoot} = req('react-dom/client'), {act} = React;
const ts = req('typescript');
const row = (id, name) => ({id,title:name,kind:'text',original_name:name,status:'ready',engine:'Synthetic',created_at:'2026-10-04T00:00:00Z',sha256:'private-hash',bytes:32,source_url:null});
const result = (title, text) => ({title,text,markdown:text,links:[],warnings:[],engine:'Synthetic',status:'ready'});
const remote = row('remote','Saved elsewhere');
const rows = new Map([[remote.id,remote]]), savedResults = new Map([[remote.id,result(remote.title,'Remote reading')]]);
const holds = new Map();
function hold(key) { let release; const promise = new Promise(resolve => { release = resolve; }); const control = {promise,release}; holds.set(key,control); return control; }
let stored = null, copiedText = '', rejectCopy = false, holdRemote = null, staleGet = null, raceFetches = 0, clipboardReads = 0;
Object.defineProperty(navigator,'clipboard',{value:{readText:async()=>{clipboardReads++;return 'https://clipboard.invalid/surprise';},writeText:async text=>{if(rejectCopy)throw new Error('denied');copiedText=text;}},configurable:true});
const helpers = {
  '@/components/ui/button':{Button:({asChild,children,variant,...props})=>asChild?React.cloneElement(children,props):React.createElement('button',props,children)},
  '@/lib/clip':{clipHtml:()=>{throw Error('Unexpected HTML path');},parseFeed:()=>{throw Error('Unexpected feed');},textDois:()=>[],doiFrom:()=>undefined,safeUrl:value=>{try{const url=new URL(value);return /^https?:$/.test(url.protocol)?url.href:null;}catch{return null;}}},
  '@/lib/imports':{expandUploads:async function*(){throw Error('Unexpected archive');}},
  '@/lib/image-ocr':{recognizeImage:async()=>{throw Error('Unexpected OCR');}},
  '@/lib/office':{extractOffice:async()=>{throw Error('Unexpected Office');}},
  '@/lib/article-assets':{retainArticleImages:async(_record,value)=>value},
  '@/lib/workspace-storage':{readWorkspace:async()=>stored,writeWorkspace:async(_owner,value)=>{stored=value;}},
  '@/lib/upload-client':{
    decodeSource:async file=>file.text(),
    uploadOriginal:async(file,{onProgress,signal})=>{
      const value=row('doc'+rows.size,file.name);rows.set(value.id,value);onProgress?.(.4);
      await holds.get('upload:'+file.name)?.promise;signal?.throwIfAborted();onProgress?.(1);return value;
    },
    saveExtracted:async(record,value,{onProgress,signal}={})=>{
      onProgress?.(.5);await holds.get('save:'+record.original_name)?.promise;signal?.throwIfAborted();savedResults.set(record.id,value);rows.set(record.id,{...record,status:value.status});onProgress?.(1);
    },
    captureSource:async()=>{throw Error('Unexpected URL');},uploadAssetFile:async()=>{throw Error('Unexpected asset');}
  }
};
global.fetch = async value=>{
  const url=new URL(value,'https://reader.test');
  if(url.pathname==='/api/documents')return Response.json({documents:[...rows.values()]});
  const id=url.pathname.split('/')[3];
  if(url.pathname.endsWith('/original'))return new Response('Fresh recovered text');
  if(id==='remote'&&holdRemote)await holdRemote.promise;
  if(id==='race'&&staleGet&&++raceFetches===1){await staleGet.promise;return Response.json({record:row('race','Stale title'),result:result('Stale title','Stale text')});}
  return Response.json({record:rows.get(id),result:savedResults.get(id)||null});
};
const localModules = new Map();
function loadLocal(name, parent = path.join(web, 'app/workspace.tsx')) {
    const base = name.startsWith('@/') ? path.join(web, name.slice(2)) : path.resolve(path.dirname(parent), name);
    const filename = [base, base + '.ts', base + '.tsx'].find(value => fs.existsSync(value) && fs.statSync(value).isFile());
    if (!filename) throw new Error('Unknown local module: ' + name);
    if (localModules.has(filename)) return localModules.get(filename).exports;
    const child = new Module(filename); child.filename = filename; child.paths = Module._nodeModulePaths(web); localModules.set(filename, child);
    child.require = dependency => helpers[dependency] || (dependency.startsWith('@/') || dependency.startsWith('.') ? loadLocal(dependency, filename) : req(dependency));
    child._compile(ts.transpileModule(fs.readFileSync(filename, 'utf8'), {compilerOptions:{module:ts.ModuleKind.CommonJS,jsx:ts.JsxEmit.ReactJSX,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText, filename);
    return child.exports;
}
const source=path.join(web,'app/workspace.tsx'), mod=new Module(source);
mod.filename=source;mod.paths=Module._nodeModulePaths(web);mod.require=name=>helpers[name]||(name.startsWith('@/')?loadLocal(name):req(name));
mod._compile(ts.transpileModule(fs.readFileSync(source,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,jsx:ts.JsxEmit.ReactJSX,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText,source);
const Workspace=mod.exports.default;
const checks=[];
const pass=(name,condition=true)=>{assert(condition,name);checks.push(name);};
const flush=async(callback=()=>{},delay=20)=>act(async()=>{await callback();await new Promise(resolve=>setTimeout(resolve,delay));});
const button=name=>[...document.querySelectorAll('button')].find(node=>node.textContent.trim()===name);
const queueRow=name=>[...document.querySelectorAll('.queue-item')].find(node=>node.querySelector('strong').textContent===name);
const pick=async(files,label='Choose source files')=>flush(()=>{const input=document.querySelector('input[aria-label="'+label+'"]');Object.defineProperty(input,'files',{value:files,configurable:true});input.dispatchEvent(new Event('change',{bubbles:true}));assert.equal(input.value,'');});
let root;
(async()=>{
  root=createRoot(document.getElementById('root'));
  await flush(()=>root.render(React.createElement(Workspace,{userId:'synthetic-menu-owner'})));
  const menu=document.querySelector('.add-menu'),trigger=menu.querySelector('summary'),textarea=document.querySelector('#source-paste');
  const changeDraft=async value=>flush(()=>{Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype,'value').set.call(textarea,value);textarea.dispatchEvent(new Event('input',{bubbles:true}));});
  const open=async()=>flush(()=>{trigger.focus();trigger.click();assert(menu.open);});
  await flush(()=>textarea.focus());
  assert.equal(clipboardReads,0);assert.equal(textarea.value,'');
  pass('Focusing an empty composer never reads the clipboard or inserts a URL');
  await changeDraft('Before after');
  await flush(()=>{textarea.blur();textarea.focus();textarea.setSelectionRange(7,7);});
  const pasteEvent=new Event('paste',{bubbles:true,cancelable:true});
  Object.defineProperty(pasteEvent,'clipboardData',{value:{files:[],getData:type=>type==='text/plain'?'pasted ':'<b>pasted </b>'}});
  await flush(()=>textarea.dispatchEvent(pasteEvent));
  assert.equal(pasteEvent.defaultPrevented,false);assert.equal(document.querySelectorAll('.queue-item').length,0);
  // jsdom does not implement native paste insertion; emulate that browser step.
  await changeDraft('Before pasted after');
  await flush(()=>{textarea.setSelectionRange(14,14);trigger.focus();textarea.focus();});
  assert.equal(textarea.value,'Before pasted after');assert.equal(textarea.selectionStart,14);assert.equal(clipboardReads,0);
  pass('Ordinary textarea paste keeps native editing, existing text and selection without starting an import');
  const mixedPaste=new Event('paste',{bubbles:true,cancelable:true});
  Object.defineProperty(mixedPaste,'clipboardData',{value:{files:[new File(['pixels'],'clipboard.png',{type:'image/png'})],getData:()=> 'clipboard text'}});
  await flush(()=>textarea.dispatchEvent(mixedPaste));
  assert.equal(mixedPaste.defaultPrevented,false);assert.equal(document.querySelectorAll('.queue-item').length,0);
  pass('An editable-field paste is not diverted into the intake queue even if clipboard files accompany text');

  await open();
  for(const label of ['Add files','Add folder','Add photos'])assert(button(label),label+' has a visible accessible name');
  assert.equal(button('Add folder').disabled,false);
  await flush(()=>button('Add files').focus());
  const escape=new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true});
  await flush(()=>document.activeElement.dispatchEvent(escape));
  assert.equal(menu.open,false);assert.equal(document.activeElement,trigger);assert(escape.defaultPrevented);
  pass('Escape from an Upload action closes the disclosure and returns focus to Upload');

  await open();await flush(()=>button('Add files').focus());
  await flush(()=>button('Add photos').dispatchEvent(new Event('pointerdown',{bubbles:true})));
  assert.equal(menu.open,true);
  await flush(()=>textarea.dispatchEvent(new Event('pointerdown',{bubbles:true})));
  assert.equal(menu.open,false);assert.notEqual(document.activeElement,trigger);
  await flush(()=>textarea.focus());assert.equal(document.activeElement,textarea);
  pass('Outside pointer dismissal leaves focus to the clicked target and inside pointers keep Upload open');

  await open();await flush(()=>button('Add files').focus());await flush(()=>button('Add photos').focus());
  assert(menu.open);
  const search=document.querySelector('#search');await flush(()=>search.focus());
  assert.equal(menu.open,false);assert.equal(document.activeElement,search);
  pass('Focus can move through Upload actions, and leaving them closes the disclosure without trapping focus');

  const picks=[];
  for(const [label,inputLabel] of [['Add files','Choose source files'],['Add folder','Choose a folder'],['Add photos','Choose photos']]){
    const input=document.querySelector('input[aria-label="'+inputLabel+'"]');
    assert(input.multiple);if(label==='Add folder')assert(input.hasAttribute('webkitdirectory'));
    if(label==='Add photos')assert.equal(input.accept,'image/*');
    input.click=()=>{assert.equal(menu.open,false);assert.equal(document.activeElement,trigger);picks.push(inputLabel);};
    await open();await flush(()=>button(label).click());
    assert.equal(menu.open,false);assert.equal(document.activeElement,trigger);
    // The native cancel event is not available in every browser. When emitted,
    // the component must handle it without claiming a completed import.
    await flush(()=>textarea.focus());await flush(()=>input.dispatchEvent(new Event('cancel')));
    assert.equal(document.activeElement,trigger);assert.equal(document.querySelectorAll('.queue-item').length,0);
  }
  assert.deepEqual(picks,['Choose source files','Choose a folder','Choose photos']);
  pass('Each picker closes Upload before native activation and restores trigger focus on supported cancel events');

  await changeDraft('some text');
  await flush(()=>textarea.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Enter',ctrlKey:true,bubbles:true,cancelable:true})),40);
  assert.equal(document.querySelector('.reading').textContent,'some text');assert.equal(textarea.value,'');assert.equal(clipboardReads,0);
  pass('An explicit submit still imports the ordinary pasted draft without clipboard permission access');
  const citations=[...document.querySelectorAll('.reader-secondary > details')].find(node=>node.querySelector('summary')?.textContent==='Citations');
  assert(citations);assert.equal(citations.open,false);
  await flush(()=>citations.querySelector('summary').click());
  assert.match(citations.textContent,/Bibliography unavailable/);assert.equal(citations.querySelector('[aria-label="Citation view"]'),null);
  assert.equal(document.querySelector('.reading').textContent,'some text');
  pass('A result without a typed citation payload exposes an honest secondary unavailable state and keeps reading intact');
  await changeDraft('Keep this draft');
  const fileClipboard=()=>{const event=new Event('paste',{bubbles:true,cancelable:true});Object.defineProperty(event,'clipboardData',{value:{files:[new File(['Clipboard source'],'clipboard-only.txt',{type:'text/plain'})],getData:()=>''}});return event;};
  const fileOnly=fileClipboard();await flush(()=>textarea.dispatchEvent(fileOnly),40);
  assert(fileOnly.defaultPrevented);assert(queueRow('clipboard-only.txt'));assert.equal(textarea.value,'Keep this draft');assert.equal(document.querySelector('.reading').textContent,'some text');
  pass('A file-only composer paste joins intake while keeping its draft and the current reading');
  const before=document.querySelectorAll('.queue-item').length,searchPaste=fileClipboard();await flush(()=>search.dispatchEvent(searchPaste));
  const editor=document.createElement('div');editor.setAttribute('contenteditable','');document.querySelector('main').append(editor);
  const editorPaste=fileClipboard();await flush(()=>editor.dispatchEvent(editorPaste));editor.remove();
  assert.equal(searchPaste.defaultPrevented,false);assert.equal(editorPaste.defaultPrevented,false);assert.equal(document.querySelectorAll('.queue-item').length,before);
  pass('File-only pastes in unrelated search or editable fields are never captured by intake');
  await flush(()=>root.unmount());root=null;dom.window.close();
  console.log(JSON.stringify({checks,limitations:'React/jsdom with real workspace event handlers and synthetic picker activation/cancel. Native picker dialogs, OS paste insertion and device behavior require browser QA.'},null,2));
})().catch(async error=>{console.error(error);if(root)await flush(()=>root.unmount());dom.window.close();process.exitCode=1;});
