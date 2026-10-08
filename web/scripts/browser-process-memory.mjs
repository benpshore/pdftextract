// Linux-only observation of the launched test browser's descendants. RSS adds
// shared mappings across processes: it is not unique memory, a heap bound, or
// a portable/mobile measurement. No unrelated process data is retained.
import { readdir, readFile } from 'node:fs/promises';
export function browserMemorySampler(rootPid) {
  let current, sampling = false;
  const results = [];
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
      const rss = processes.filter(p => owned.has(p.pid)).reduce((n, p) => n + p.rss, 0);
      const pss = await Promise.all([...owned].map(async pid => {
        try { return Number((await readFile(`/proc/${pid}/smaps_rollup`, 'utf8')).match(/^Pss:\s+(\d+)/m)?.[1]) * 1024; }
        catch { return null; }
      }));
      if (owner !== current) return;
      owner.samples++;
      owner.startRssBytes ??= rss;
      owner.endRssBytes = rss;
      owner.peakRssBytes = Math.max(owner.peakRssBytes || 0, rss);
      if (pss.every(n => n !== null && Number.isFinite(n))) {
        const totalPss = pss.reduce((a, b) => a + b, 0);
        owner.peakPssBytes = Math.max(owner.peakPssBytes || 0, totalPss);
        owner.endPssBytes = totalPss;
      }
    } catch { owner.unavailable = 'Linux /proc process RSS unavailable'; }
    finally { sampling = false; }
  }
  const timer = setInterval(() => void sample(), 100);
  return {
    async begin(label) { current = { label, samples: 0, started: performance.now() }; await sample(); },
    async end() { await sample(); if (current) { const { started, ...result } = current; result.milliseconds = Math.round(performance.now() - started); results.push(result); current = null; return result; } },
    stop() { clearInterval(timer); },
    results,
    method: '100 ms samples of owned Chromium process-tree Linux VmRSS and, when readable, smaps_rollup Pss. RSS counts shared mappings in each process; PSS allocates shared pages proportionally. Excludes fixture server/Node; peaks between samples can be missed. Neither measure is an in-browser memory bound.',
  };
}
