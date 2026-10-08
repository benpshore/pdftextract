// Browser adapter policy, independent of legacy TPE/PDF Oxide ordering.
// Conserves immutable PDF.js text cells. Repeated whitespace gutters separate
// columns; rows crossing those gutters divide the page into horizontal bands.
export function orderTextCells(cells){
  const sorted=cells.filter(c=>c.text.trim()).slice().sort((a,b)=>a.bbox[1]-b.bbox[1]||a.bbox[0]-b.bbox[0]||a.index-b.index);
  const rows=[];
  for(const cell of sorted){
    const last=rows.at(-1),tolerance=Math.max(1,(cell.bbox[3]-cell.bbox[1])*0.35);
    if(last&&Math.abs(last.y-cell.bbox[1])<=tolerance)last.cells.push(cell);
    else rows.push({y:cell.bbox[1],cells:[cell]});
  }
  for(const row of rows)row.cells.sort((a,b)=>a.bbox[0]-b.bbox[0]||a.index-b.index);
  const candidates=[];
  rows.forEach((row,index)=>{for(let i=1;i<row.cells.length;i++){
    const left=row.cells[i-1].bbox[2],right=row.cells[i].bbox[0];
    if(right-left>=Math.max(4,(row.cells[i].bbox[3]-row.cells[i].bbox[1])*0.5))candidates.push({left,right,rows:new Set([index])});
  }});
  const gutters=[];
  for(const candidate of candidates){
    const match=gutters.find(g=>Math.max(g.left,candidate.left)<Math.min(g.right,candidate.right));
    if(match){match.left=Math.max(match.left,candidate.left);match.right=Math.min(match.right,candidate.right);for(const r of candidate.rows)match.rows.add(r);}else gutters.push(candidate);
  }
  const cuts=gutters.filter(g=>g.rows.size>=2).map(g=>(g.left+g.right)/2).sort((a,b)=>a-b);
  if(!cuts.length)return {cells:rows.flatMap(r=>r.cells),columns:1};
  const result=[],band=[];
  const flush=()=>{for(let column=0;column<=cuts.length;column++)for(const row of band)for(const cell of row.cells){const center=(cell.bbox[0]+cell.bbox[2])/2;if(cuts.filter(c=>center>c).length===column)result.push(cell);}band.length=0;};
  for(const row of rows){
    if(row.cells.some(cell=>cuts.some(c=>cell.bbox[0]<c&&cell.bbox[2]>c))){flush();result.push(...row.cells);}else band.push(row);
  }
  flush();return {cells:result,columns:cuts.length+1};
}
export function textCells(items,viewport){
  return items.filter(item=>typeof item.str==='string').map((item,index)=>{
    const x=item.transform[4],y=item.transform[5],height=item.height||Math.hypot(item.transform[2],item.transform[3]);
    const a=viewport.convertToViewportPoint(x,y),b=viewport.convertToViewportPoint(x+item.width,y+height);
    return {id:`pdfjs:${index}`,index,text:item.str,bbox:[Math.min(a[0],b[0]),Math.min(a[1],b[1]),Math.max(a[0],b[0]),Math.max(a[1],b[1])],direction:item.dir,transform:item.transform.slice()};
  });
}
