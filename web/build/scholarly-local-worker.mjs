// Private local service binding only. Never included in a production build.
export default {async fetch(request,env){
  const path=new URL(request.url).pathname;
  if(!['/health','/grobid','/bibliography'].includes(path))return new Response('Not found',{status:404});
  if(path==='/bibliography')return new Response('Automatic metadata lookup is disabled',{status:503});
  let base;
  try{base=new URL(env.SCHOLARLY_LOCAL_URL);}catch{return new Response('Local scholarly runtime is not configured',{status:503});}
  if(base.protocol!=='http:'||base.hostname!=='127.0.0.1'||base.username||base.password||base.search||base.hash||base.pathname!=='/')return new Response('Invalid local runtime configuration',{status:503});
  const response=await fetch(new URL(path,base),{method:request.method,headers:{'Content-Type':'application/pdf','X-TPE-Local-Bridge':'1'},body:request.method==='POST'?request.body:undefined,signal:request.signal,redirect:'manual'});
  if(response.status>=300&&response.status<400){await response.body?.cancel();return new Response('Local runtime redirects are not allowed',{status:502});}
  return response;
}};
