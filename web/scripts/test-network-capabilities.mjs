/** Functional release-hold regression; no network probes or security clearance. */
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createRequire} from 'node:module';
import {JSDOM} from 'jsdom';
const require=createRequire(import.meta.url);
const {build}=require(require.resolve('esbuild',{paths:[require.resolve('vite')]}));
const temporary=await mkdtemp(join(tmpdir(),'tpe-network-hold-'));
const dom=new JSDOM('',{url:'https://app.fixture.invalid'});
globalThis.window=dom.window;globalThis.document=dom.window.document;globalThis.DOMParser=dom.window.DOMParser;
let requests=0;globalThis.fetch=async()=>{requests++;throw Error('Unexpected network work');};
try{
  const outfile=join(temporary,'hold.mjs');
  await build({stdin:{contents:"export * from './lib/network-capabilities';export * from './lib/source-fetch';export * from './lib/upload-client';export * from './lib/clip';export * from './lib/article-assets';",resolveDir:process.cwd()},bundle:true,format:'esm',platform:'browser',outfile});
  const api=await import(pathToFileURL(outfile));
  assert.equal(api.remoteExtractionEnabled,false);assert.equal(api.metadataResolutionEnabled,false);
  for(const operation of [()=>api.allowed('https://source.fixture.invalid/'),()=>api.fetchPublicSource('https://source.fixture.invalid/','text/html')])await assert.rejects(operation,error=>error instanceof Response&&error.status===503);
  await assert.rejects(()=>api.captureSource('https://source.fixture.invalid/'),/disabled/);
  const local=api.clipHtml('<!doctype html><html lang="fr"><head><title>Local study</title><link rel="canonical" href="https://source.fixture.invalid/study"><link rel="alternate" type="application/rss+xml" href="https://source.fixture.invalid/feed"><link rel="stylesheet" href="https://resource.fixture.invalid/style.css"><meta name="citation_title" content="Local study"><script type="application/ld+json">{"@type":"ScholarlyArticle","headline":"Local study"}</script></head><body><article><h1>Local study</h1><p>Local readable text with a source <a href="https://source.fixture.invalid/">link</a>.</p><img src="https://resource.fixture.invalid/figure.png"></article></body></html>','https://saved.invalid/','local.html');
  assert.equal(local.metadata.language,'fr');assert.equal(local.metadata.canonical,'https://source.fixture.invalid/study');assert.equal(local.metadata.feeds[0].url,'https://source.fixture.invalid/feed');assert.match(local.text,/Local readable text/);assert(!local.html.includes('<img'));assert(local.links.some(link=>link.url==='https://source.fixture.invalid/'));assert.equal(local.metadata.meta.citation_title,'Local study');assert.equal(local.metadata.structuredData[0].headline,'Local study');
  // A complete root declaration must survive inert fragment conversion. Entity
  // decoding and quoted > belong to the browser's attribute parser. Fragments,
  // comments and scripts must not invent document-level language provenance.
  const clip=source=>api.clipHtml(source,'https://saved.invalid/','language.html');
  const article='<head><title>Language fixture</title></head><body><article><p>Local readable language evidence.</p></article></body>';
  assert.equal(clip(`<html>${article}</html>`).metadata.language,null);
  assert.equal(clip(` \n<!-- prologue --><!DOCTYPE html><HTML title="a > b" LANG='fr&#45;CA'>${article}</HTML>`).metadata.language,'fr-CA');
  assert.equal(clip(`<!-- <html lang="de"> -->${article}`).metadata.language,null);
  assert.equal(clip(`<script>"<html lang='de'>"</script>${article}`).metadata.language,null);
  assert.equal(clip('<article lang="es"><p>Declared article language remains.</p></article>').metadata.language,'es');
  const retained=await api.retainArticleImages({id:'local'}, {...local,html:'<p>Reading remains</p><img src="https://resource.fixture.invalid/figure.png"><img src="/api/documents/local/assets/owned">'});
  assert.match(retained.html,/Reading remains/);assert(!retained.html.includes('resource.fixture.invalid'));assert(retained.html.includes('/api/documents/local/assets/owned'));
  assert.equal(requests,0);
  console.log(JSON.stringify({passed:5,checks:['Fixed remote and resolver capability hold','Destination helpers reject before DNS/fetch','Capture client rejects before API request','Local HTML retains text, links and scholarly metadata without external images','Existing owner-local images remain and retention issues zero requests'],scope:'Functional synthetic fixtures only; no SSRF, rebinding, parser RCE or MCP security verification.'},null,2));
}finally{dom.window.close();await rm(temporary,{recursive:true,force:true});}
