'use client';

import {useCallback, useEffect, useRef, useState} from 'react';
import {LoaderCircle} from 'lucide-react';
import {Button} from '@/components/ui/button';
import type {DocumentRow, Extracted} from '@/lib/types';

type Capability = {available:boolean;mode:'live-native'|'captured-native'|null;reason?:string;resolver:boolean;baseResultKey:string|null};
export type ScholarlyOperation = {phase:'processing'|'cancelling'|'reconciling'|'done'|'failed'|'cancelled'|'uncertain';message:string};
export const scholarlyBusy = (operation?:ScholarlyOperation) => !!operation&&['processing','cancelling','reconciling'].includes(operation.phase);
const modeLabel = (mode:string) => mode==='captured-native'?'Captured runtime replay':'Local native/GROBID';
const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function evidence(value:unknown) {
  if(!value||typeof value!=='object')return null;
  const item=value as Record<string,unknown>;
  return item.schema==='tpe.scholarly-attachment'&&item.version===1&&typeof item.evidence_id==='string'&&uuid.test(item.evidence_id)&&['live-native','captured-native'].includes(item.mode as string)&&typeof item.generated_at==='string'&&typeof item.has_resolution==='boolean'?item:null;
}

export function ScholarlyControls({record,result,disabled,operation,onExtract,onCancel,onReconcile}:{record?:DocumentRow;result:Extracted|null;disabled:boolean;operation?:ScholarlyOperation;onExtract:(baseResultKey:string|null)=>void;onCancel:()=>void;onReconcile:()=>void}) {
  const [capability,setCapability]=useState<Capability|null>(null),[checking,setChecking]=useState(false),[error,setError]=useState('');
  const request=useRef<AbortController|null>(null);
  const eligible=!!record&&(record.kind==='pdf'||record.mime==='application/pdf')&&!!result;
  const endpoint=record?'/api/documents/'+encodeURIComponent(record.id)+'/scholarly':'';
  const busy=scholarlyBusy(operation),attachment=evidence(result?.metadata?.scholarly);
  const extractionPending=useRef(false),current=useRef({endpoint,allowed:false,onExtract});
  current.current={endpoint,allowed:eligible&&!disabled&&!busy&&operation?.phase!=='uncertain',onExtract};
  const check=useCallback(async()=>{
    request.current?.abort();const controller=new AbortController();request.current=controller;
    setChecking(true);setError('');
    try{
      const response=await fetch(endpoint,{signal:controller.signal,cache:'no-store'});
      if(!response.ok)throw new Error('Reference extraction availability could not be checked.');
      const value=await response.json() as Capability;
      if(typeof value.available!=='boolean'||typeof value.resolver!=='boolean'||!['live-native','captured-native',null].includes(value.mode)||(value.baseResultKey!==null&&typeof value.baseResultKey!=='string')||(value.available&&value.mode===null))throw new Error('Reference extraction availability could not be checked.');
      if(controller.signal.aborted)return null;
      setCapability(value);return value;
    }catch(reason){if(!controller.signal.aborted){setCapability(null);setError(reason instanceof Error?reason.message:String(reason));}return null;}
    finally{if(!controller.signal.aborted)setChecking(false);}
  },[endpoint]);
  useEffect(()=>{setCapability(null);setError('');if(eligible)void check();return()=>{request.current?.abort();};},[eligible,record?.result_key,check]);
  async function extract(){
    if(extractionPending.current||!current.current.allowed)return;
    extractionPending.current=true;const startedEndpoint=endpoint;
    try{const capability=await check();if(capability?.available&&current.current.allowed&&current.current.endpoint===startedEndpoint)current.current.onExtract(capability.baseResultKey);}
    finally{extractionPending.current=false;}
  }
  return <section className="scholarly-controls" aria-label="Reference extraction">
    {!eligible?<p className="help">Reference extraction needs a saved, readable PDF.</p>:<>
      {capability?.available&&<p className="help">{modeLabel(capability.mode!)}{capability.resolver?' · Metadata resolution available':''}</p>}
      {!busy&&checking&&<p className="help" role="status">Checking reference extraction…</p>}
      {!checking&&capability&&!capability.available&&<p className="help" role="status">{capability.reason||'Reference extraction is unavailable.'}</p>}
      {error&&<p role="alert">{error}</p>}
      {operation&&<div className={busy?'processing-status':'scholarly-status'} role={operation.phase==='failed'||operation.phase==='uncertain'?'alert':'status'}>{busy&&<LoaderCircle className="processing-spinner" aria-hidden="true"/>}<p>{operation.message}</p></div>}
      <div className="citation-controls">
        {capability?.available&&!busy&&operation?.phase!=='uncertain'&&<Button variant="outline" disabled={disabled||checking} onClick={()=>void extract()}>{operation?.phase==='failed'?'Retry extraction':'Extract references'}</Button>}
        {!busy&&!checking&&(error||capability&&!capability.available)&&<Button variant="outline" onClick={()=>void check()}>Check availability</Button>}
        {busy&&<Button variant="outline" disabled={operation?.phase!=='processing'} onClick={onCancel}>Cancel reference extraction</Button>}
        {!busy&&(operation?.phase==='cancelled'||operation?.phase==='uncertain')&&<Button variant="outline" disabled={disabled||checking} onClick={onReconcile}>Check saved result</Button>}
      </div>
      {disabled&&!busy&&<p className="help">Wait for imports or storage changes to finish.</p>}
    </>}
    {record&&attachment&&<details className="scholarly-evidence"><summary>Scholarly evidence</summary><p className="help">{modeLabel(attachment.mode as string)}</p><div className="export-actions"><a download href={endpoint+'/evidence?format=native-json'}>Download native JSON</a><a download href={endpoint+'/evidence?format=tei'}>Download raw TEI</a>{attachment.has_resolution===true&&<a download href={endpoint+'/evidence?format=resolution'}>Download resolver JSON</a>}</div><details><summary>Details</summary><dl className="citation-fields"><div><dt>Evidence ID</dt><dd>{attachment.evidence_id as string}</dd></div><div><dt>Generated</dt><dd>{attachment.generated_at as string}</dd></div></dl></details></details>}
  </section>;
}
