// Explicit pinned static artifact staging. No package hooks, policy changes, or per-job downloads.
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {gunzipSync} from 'node:zlib';
import {ROOT,MODELS} from '../public/pdf-api/v1/manifest.js';
const root=new URL('../public'+ROOT,import.meta.url);
const packages=[
  {name:'docling.rs-wasm',version:'1.104.2',integrity:'EkyUVQg6bfsLfZ+j467hs50lIjrpzpJV+C953hVPn+xaW+Au24M3YL+fOkadpCzNZSL2pxlJNnS1HxByJHaDeA==',select:p=>p.startsWith('web/')?p.slice(4):p==='LICENSE'?'licenses/docling-LICENSE':null},
  {name:'onnxruntime-web',version:'1.24.3',integrity:'41dDq7fxtTm0XzGE7N0d6m8FcOY8EWtUA65GkOixJPB/G7DGzBmiDAnVVXHznRw9bgUZpb+4/1lQK/PNxGpbrQ==',select:p=>['dist/ort.wasm.min.mjs','dist/ort-wasm-simd-threaded.mjs','dist/ort-wasm-simd-threaded.wasm'].includes(p)?'ort/'+p.slice(5):p==='LICENSE'?'licenses/ort-LICENSE':null},
  {name:'pdfjs-dist',version:'5.4.624',integrity:'sm6TxKTtWv1Oh6n3C6J6a8odejb5uO4A4zo/2dgkHuC0iu8ZMAXOezEODkVaoVp8nX1Xzr+0WxFJJmUr45hQzg==',select:p=>['build/pdf.mjs','build/pdf.worker.mjs'].includes(p)?'pdfjs/'+p.slice(6):['cmaps/','standard_fonts/','wasm/'].some(prefix=>p.startsWith(prefix))?'pdfjs/'+p:p==='LICENSE'?'licenses/pdfjs-LICENSE':null},
];
async function download(url,limit){
  const r=await fetch(url,{signal:AbortSignal.timeout(60000)});if(!r.ok)throw new Error(`${url}: HTTP ${r.status}`);
  const chunks=[];let n=0;for await(const chunk of r.body){n+=chunk.length;if(n>limit)throw new Error('Asset exceeds bounded download budget: '+url);chunks.push(chunk);}return Buffer.concat(chunks);
}
const hash=(bytes,algorithm='sha256',encoding='hex')=>createHash(algorithm).update(bytes).digest(encoding);
for(const pkg of packages){
  const stamp=new URL(`.verified-${pkg.name}-${pkg.version}`,root);
  try{if(await readFile(stamp,'utf8')===pkg.integrity){console.log('Cached static artifact: '+pkg.name);continue;}}catch{/* first staging */}
  const compressed=await download(`https://registry.npmjs.org/${pkg.name}/-/${pkg.name}-${pkg.version}.tgz`,64*1024*1024);
  if(hash(compressed,'sha512','base64')!==pkg.integrity)throw new Error('Package archive integrity mismatch: '+pkg.name);
  const tar=gunzipSync(compressed,{maxOutputLength:160*1024*1024});
  let count=0;
  for(let offset=0;offset+512<=tar.length;){
    const header=tar.subarray(offset,offset+512);if(header.every(b=>b===0))break;
    const raw=header.subarray(0,100).toString().split('\0')[0];
    const size=parseInt(header.subarray(124,136).toString().replace(/\0/g,'').trim(),8);
    if(!Number.isSafeInteger(size)||size<0||offset+512+size>tar.length)throw new Error('Invalid tar archive');
    const name=raw.startsWith('package/')?pkg.select(raw.slice(8)):null;
    if(name&&(!header[156]||header[156]===48)){
      if(name.split('/').some(p=>p==='..'||p==='')||name.startsWith('/'))throw new Error('Invalid selected asset path');
      const target=new URL(name,root);await mkdir(new URL('.',target),{recursive:true});await writeFile(target,tar.subarray(offset+512,offset+512+size));count++;
    }
    offset+=512+Math.ceil(size/512)*512;
  }
  if(!count)throw new Error('No assets extracted: '+pkg.name);
  await writeFile(stamp,pkg.integrity);console.log(`Staged ${pkg.name}@${pkg.version}: ${count} verified static files`);
}
if(process.argv.includes('--models')){
  for(const model of Object.values(MODELS)){
    const target=new URL('models/'+model.file,root);let data;try{data=await readFile(target);}catch{/* not staged */}
    if(!data||hash(data)!==model.sha256){data=await download('https://github.com/docling-project/docling.rs/releases/download/models-v1/'+model.file,model.bytes+1);}
    if(data.length!==model.bytes||hash(data)!==model.sha256)throw new Error('Model identity mismatch: '+model.file);
    await mkdir(new URL('.',target),{recursive:true});await writeFile(target,data);console.log(`Verified ${model.file}: ${model.bytes} bytes ${model.sha256}`);
  }
}else console.log('Models omitted. Run explicitly with --models for layout/OCR (~84 MiB).');
