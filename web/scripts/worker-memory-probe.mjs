// Test-only instrumentation, prepended by the browser harness. It observes the
// actual two WASM runtimes without changing model feeds, outputs or allocation
// flags. Capacities are not committed/resident memory and must not be added to
// process RSS. Holding these memories is harmless only in this bounded test.
export function workerMemoryProbe(){
  const OriginalMemory=WebAssembly.Memory,memories=new Map();
  WebAssembly.Memory=new Proxy(OriginalMemory,{construct(Target,args){const memory=Reflect.construct(Target,args);memories.set(memory,(new Error().stack||'').includes('/ort/')?'onnxruntime (constructor location)':'constructed memory');return memory;}});
  function remember(value,hint){
    const exports=value?.instance?.exports||value?.exports;
    if(!exports)return;
    const label=exports.digitalconverter_new?'docling.rs':hint.includes('ort-')?'onnxruntime':hint.includes('docling')?'docling.rs':null;
    for(const memory of Object.values(exports))if(memory instanceof OriginalMemory)memories.set(memory,label||memories.get(memory)||'exported memory');
  }
  const instantiate=WebAssembly.instantiate.bind(WebAssembly);
  WebAssembly.instantiate=async(...args)=>{const result=await instantiate(...args);remember(result,'');return result;};
  const streaming=WebAssembly.instantiateStreaming.bind(WebAssembly);
  WebAssembly.instantiateStreaming=async(input,...args)=>{const response=await input,result=await streaming(response,...args);remember(result,response.url||'');return result;};
  const post=self.postMessage.bind(self);
  self.postMessage=(message,...args)=>{
    const observation={method:'test-only WebAssembly constructor/instance observation; linear capacities, not resident bytes',memories:[...memories].map(([memory,runtime],index)=>({index,runtime,capacityBytes:memory.buffer.byteLength,shared:typeof SharedArrayBuffer!=='undefined'&&memory.buffer instanceof SharedArrayBuffer}))};
    if(message.event)message.event.testOnlyWasmMemory=observation;
    if(message.result?.resources)message.result.resources.testOnlyWasmMemory=observation;
    return post(message,...args);
  };
}
