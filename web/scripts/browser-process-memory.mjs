// Linux-only observation of the launched test browser's descendants. RSS adds
// shared mappings across processes: it is not unique memory, a heap bound, or
// a portable/mobile measurement. No unrelated process data is retained.
import { readdir, readFile } from 'node:fs/promises';
export function browserMemorySampler(rootPid) {
  let current, sampling = false;
  const results = [];
  const processKeys=new Map();
  async function sample() {
    if (sampling || !current) return;
    sampling = true;
    const owner = current;
    try {
      const names = await readdir('/proc');
      const processes = (await Promise.all(names.filter(n => /^\d+$/.test(n)).map(async id => {
        try {
          const status = await readFile(`/proc/${id}/status`, 'utf8');
          return { pid: Number(id), parent: Number(status.match(/^PPid:\s+(\d+)/m)?.[1]), rss: Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1] || 0) * 1024 };
        } catch { return null; }
      }))).filter(Boolean);
      const owned = new Set([rootPid]);
      let changed = true;
      while (changed) { changed = false; for (const p of processes) if (owned.has(p.parent) && !owned.has(p.pid)) { owned.add(p.pid); changed = true; } }
      const details=await Promise.all(processes.filter(p=>owned.has(p.pid)).map(async p=>{
        let type=p.pid===rootPid?'browser':'child',pss=null,privateBytes=null;
        try{const cmd=await readFile(`/proc/${p.pid}/cmdline`,'utf8');type=cmd.match(/--type=([^\0 ]+)/)?.[1]||type;}catch{}
        try{const smaps=await readFile(`/proc/${p.pid}/smaps_rollup`,'utf8');pss=Number(smaps.match(/^Pss:\s+(\d+)/m)?.[1])*1024;privateBytes=[...smaps.matchAll(/^Private_(?:Clean|Dirty):\s+(\d+)/gm)].reduce((n,m)=>n+Number(m[1])*1024,0);}catch{}
        if(!processKeys.has(p.pid))processKeys.set(p.pid,`process-${processKeys.size+1}`);
        return {process:processKeys.get(p.pid),type,rssBytes:p.rss,pssBytes:pss,privateBytes};
      }));
      const rss=details.reduce((n,p)=>n+p.rssBytes,0),pss=details.map(p=>p.pssBytes);
      if (owner !== current) return;
      owner.samples++;
      owner.startRssBytes ??= rss;
      owner.endRssBytes = rss;
      owner.endProcesses=details;
      if(rss>=(owner.peakRssBytes||0))owner.processesAtRssPeak=details;
      owner.peakRssBytes = Math.max(owner.peakRssBytes || 0, rss);
      const phase=owner.phase||'unspecified';
      const phaseSample=owner.phases[phase]||={samples:0,peakRssBytes:0};phaseSample.samples++;phaseSample.peakRssBytes=Math.max(phaseSample.peakRssBytes,rss);
      if (pss.every(n => n !== null && Number.isFinite(n))) {
        const totalPss = pss.reduce((a, b) => a + b, 0);
        owner.peakPssBytes = Math.max(owner.peakPssBytes || 0, totalPss);
        owner.endPssBytes = totalPss;
        phaseSample.peakPssBytes=Math.max(phaseSample.peakPssBytes||0,totalPss);
      }
    } catch { owner.unavailable = 'Linux /proc process RSS unavailable'; }
    finally { sampling = false; }
  }
  const timer = setInterval(() => void sample(), 100);
  return {
    async begin(label) { current = { label, samples: 0,phases:{}, started: performance.now() }; await sample(); },
    mark(event){if(current){current.phase=event.phase+(event.model?':'+event.model:'');void sample();}},
    checkpoint(){return current?{rssBytes:current.endRssBytes,pssBytes:current.endPssBytes,samples:current.samples,elapsedMs:performance.now()-current.started}:null;},
    async end() { await sample(); if (current) { const { started, ...result } = current; result.milliseconds = Math.round(performance.now() - started); results.push(result); current = null; return result; } },
    stop() { clearInterval(timer); },
    results,
    method: '100 ms samples of the owned test-browser process tree: Linux VmRSS and, when readable, smaps_rollup Pss. RSS counts shared mappings in each process; PSS allocates shared pages proportionally. Excludes fixture server/Node; peaks between samples can be missed. Neither measure is an in-browser memory bound.',
  };
}
