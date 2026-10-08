// Deterministic reading order for horizontal LTR text with repeated whitespace
// gutters. A wide vertical gap starts a new band, whose column count may differ.
// Preserve every input cell; model layout is retained separately by the API.
export function orderTextCells(cells) {
  if(cells.length>4096)return {cells,columns:1,supported:false,bands:[],policy:'horizontal-ltr-repeated-gutters-v2',reason:'More than 4096 cells exceed the bounded column-order policy; upstream model order is retained.'};
  const sorted=cells.filter(c=>c.text.trim()).slice().sort((a,b)=>a.bbox[1]-b.bbox[1]||a.bbox[0]-b.bbox[0]||a.index-b.index);
  const unsupported=sorted.some(c=>c.direction&&c.direction!=='ltr'||c.transform&&(Math.abs(c.transform[1])>0.01||Math.abs(c.transform[2])>0.01)||!c.bbox.every(Number.isFinite));
  const rows=[];
  for(const cell of sorted){
    const last=rows.at(-1),height=cell.bbox[3]-cell.bbox[1],tolerance=Math.max(1,height*0.35);
    if(last&&Math.abs(last.y-cell.bbox[1])<=tolerance){last.cells.push(cell);last.height=Math.max(last.height,height);}
    else rows.push({y:cell.bbox[1],height,cells:[cell]});
  }
  for(const row of rows)row.cells.sort((a,b)=>a.bbox[0]-b.bbox[0]);
  const bands=[];
  for(const row of rows){
    const last=bands.at(-1),previous=last?.at(-1);
    if(previous&&row.y-previous.y<=Math.max(row.height,previous.height)*3)last.push(row);else bands.push([row]);
  }
  const result=[],evidence=[];
  for(const band of bands){
    const candidates=[];
    // Pair nearby rows so staggered baselines can establish the same gutter.
    for(let index=0;index<band.length;index++){
      const row=band[index],next=band[index+1];
      for(const group of [row.cells,...(next?[row.cells.concat(next.cells)]:[])]){
        const intervals=[];
        for(const cell of group.slice().sort((a,b)=>a.bbox[0]-b.bbox[0])){
          const last=intervals.at(-1);if(last&&cell.bbox[0]<=last[1])last[1]=Math.max(last[1],cell.bbox[2]);else intervals.push([cell.bbox[0],cell.bbox[2]]);
        }
        for(let i=1;i<intervals.length;i++)if(intervals[i][0]-intervals[i-1][1]>=Math.max(4,row.height*1.5))candidates.push({left:intervals[i-1][1],right:intervals[i][0],rows:new Set([index])});
      }
    }
    const gutters=[];
    for(const candidate of candidates){
      const match=gutters.find(g=>Math.max(g.left,candidate.left)<Math.min(g.right,candidate.right));
      if(match){match.left=Math.max(match.left,candidate.left);match.right=Math.min(match.right,candidate.right);for(const row of candidate.rows)match.rows.add(row);}else gutters.push(candidate);
    }
    const cuts=gutters.filter(g=>g.rows.size>=2).map(g=>(g.left+g.right)/2).sort((a,b)=>a-b);
    const pending=[];
    const flush=()=>{
      for(let column=0;column<=cuts.length;column++)for(const row of pending)for(const cell of row.cells){
        const center=(cell.bbox[0]+cell.bbox[2])/2;if(cuts.filter(c=>center>c).length===column)result.push(cell);
      }
      pending.length=0;
    };
    for(const row of band){
      if(row.cells.some(cell=>cuts.some(c=>cell.bbox[0]<c&&cell.bbox[2]>c))){flush();result.push(...row.cells);}else pending.push(row);
    }
    flush();
    evidence.push({top:band[0].y,bottom:Math.max(...band.flatMap(r=>r.cells.map(c=>c.bbox[3]))),columns:cuts.length+1,gutters:cuts,supportRows:gutters.filter(g=>g.rows.size>=2).map(g=>g.rows.size)});
  }
  return {cells:result,columns:Math.max(1,...evidence.map(b=>b.columns)),supported:!unsupported,bands:evidence,policy:'horizontal-ltr-repeated-gutters-v2'};
}
export function orderedTextBlocks(cells,pageNo){
  const lines=[];
  for(const cell of cells){
    const last=lines.at(-1),height=cell.bbox[3]-cell.bbox[1];
    if(last&&Math.abs(last.box[1]-cell.bbox[1])<height*0.35&&cell.bbox[0]>=last.box[2]-1&&cell.bbox[0]-last.box[2]<height){last.text+=' '+cell.text.trim();last.box[2]=cell.bbox[2];last.box[3]=Math.max(last.box[3],cell.bbox[3]);last.ids.push(cell.id);}
    else lines.push({text:cell.text.trim(),box:cell.bbox.slice(),ids:[cell.id]});
  }
  return lines.map((line,order)=>({id:line.ids.join('+'),cellIds:line.ids,text:line.text,label:'text',source:'native text',order,provenance:[{page_no:pageNo,bbox:{l:line.box[0],t:line.box[1],r:line.box[2],b:line.box[3],coord_origin:'TOPLEFT'}}]}));
}
export function textCells(items,viewport){
  return items.filter(item=>typeof item.str==='string').map((item,index)=>{
    const x=item.transform[4],y=item.transform[5],height=item.height||Math.hypot(item.transform[2],item.transform[3]);
    const a=viewport.convertToViewportPoint(x,y),b=viewport.convertToViewportPoint(x+item.width,y+height);
    return {id:`pdfjs:${index}`,index,text:item.str,bbox:[Math.min(a[0],b[0]),Math.min(a[1],b[1]),Math.max(a[0],b[0]),Math.max(a[1],b[1])],direction:item.dir,transform:item.transform.slice()};
  });
}
