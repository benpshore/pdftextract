import type {DocumentRow,Extracted} from './types';
export type ScholarlySavedResult={record:DocumentRow;result:Extracted};
/** Call synchronously at the state write, after all response parsing awaits. */
export function currentDocumentResult<T>(id:string,value:T,startedVersion:number|undefined,versions:Map<string,number>,results:Map<string,ScholarlySavedResult>):T|ScholarlySavedResult{
  const latest=results.get(id);
  return latest&&startedVersion!==versions.get(id)?latest:value;
}
/** Compare after body consumption: a scholarly save can finish after GET headers. */
export async function fetchCurrentDocument(id:string,versions:Map<string,number>,results:Map<string,ScholarlySavedResult>):Promise<Response>{
  const version=versions.get(id),response=await fetch('/api/documents/'+encodeURIComponent(id));
  const body=await response.arrayBuffer(),latest=results.get(id);
  return latest&&version!==versions.get(id)?Response.json(latest):new Response(body,{status:response.status,statusText:response.statusText,headers:response.headers});
}
