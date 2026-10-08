import type {Extracted} from './types';
export type PdfOptions={engine:'docling'|'fast-text';ocr:'auto'|'always'|'off'};
type Progress={phase:string;message:string;completed?:number;total?:number};
type Result={status:string;text:string;engine:{name:string;version:string};pages:{page:number;diagnostics:{code:string;message:string}[];links?:{url:string;label?:string}[]}[];diagnostics:{code:string;message:string}[];source:unknown};
type Api={createPdfJob:(file:File,options:{engine:string;ocr:string;signal:AbortSignal;onProgress:(event:Progress)=>void})=>{result:Promise<Result>}};
export async function extractPdfWithOptions(file:File,options:PdfOptions,signal:AbortSignal,onProgress:(event:Progress)=>void):Promise<Extracted>{
  const entry='/pdf-api/v1/api.js';
  const api=await import(/* @vite-ignore */ entry) as Api;
  return toExtracted(file,await api.createPdfJob(file,{...options,signal,onProgress}).result);
}
function toExtracted(file:File,output:Result):Extracted{
  if(output.status==='cancelled')throw new DOMException('PDF extraction cancelled.','AbortError');
  if(output.status==='failed')throw new Error(output.diagnostics.map(d=>`${d.code}: ${d.message}`).join('\n'));
  return {title:file.name,text:output.text,markdown:output.text,pages:output.pages,links:output.pages.flatMap(p=>(p.links||[]).map(link=>({...link,page:p.page,kind:'embedded PDF link'}))),warnings:[...output.diagnostics,...output.pages.flatMap(p=>p.diagnostics)].map(d=>`${d.code}: ${d.message}`),engine:output.engine.name+' '+output.engine.version+' (CPU/WASM)',status:'partial',metadata:{pdfApi:output}};
}
