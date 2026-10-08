import {createPdfJob,discoverPdf,cancelPdfJob,disposePdfRuntime} from './api.js';
const el=id=>document.getElementById(id);let job,result;
el('extract').addEventListener('submit',async event=>{
  event.preventDefault();el('run').disabled=true;el('cancel').disabled=false;el('download').disabled=true;el('text').textContent='';
  try{
    const pages=el('pages').value.trim();
    job=createPdfJob(el('file').files[0],{engine:el('layout').value,ocr:el('ocr').value,...(pages?{pages:pages.split(',').map(p=>Number(p.trim()))}:{}),onProgress:e=>el('status').textContent=e.message});
    result=await job.result;el('text').textContent=result.text;el('result').textContent=JSON.stringify(result,null,2);el('status').textContent=result.status+': '+result.diagnostics.map(d=>d.code+' — '+d.message).join('; ');el('download').disabled=false;
  }catch(error){el('status').textContent=error.code+': '+error.message;}
  finally{el('run').disabled=false;el('cancel').disabled=true;}
});
el('cancel').addEventListener('click',()=>job&&cancelPdfJob(job.id));
el('layout').addEventListener('change',()=>{const fast=el('layout').value==='fast-text';el('ocr').disabled=fast;el('pages').disabled=fast;if(fast){el('ocr').value='off';el('pages').value='';}});
el('capabilities').addEventListener('click',async()=>{el('result').textContent=JSON.stringify(await discoverPdf(),null,2);});
el('release').addEventListener('click',()=>{try{disposePdfRuntime();el('status').textContent='Model memory released.';}catch(error){el('status').textContent=error.message;}});
el('download').addEventListener('click',()=>{const url=URL.createObjectURL(new Blob([JSON.stringify(result,null,2)],{type:'application/json'}));const a=document.createElement('a');a.href=url;a.download='pdf-result.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);});
