// Use PDF.js's own clipped, transformed rendering bounds. These are quantized
// outward to 1/256 of the rendered page; never infer an image from model labels.
export function imageRegions(operators, bounds, OPS, width, height, limit) {
  if (!bounds || operators.fnArray.length !== bounds.length) throw Object.assign(new Error('Rendered PDF image operations cannot be mapped to their bounds.'), {code:'REGION_MAP_UNSUPPORTED'});
  const imageOps = new Set(['paintImageXObject','paintInlineImageXObject','paintInlineImageXObjectGroup','paintImageXObjectRepeat','paintImageMaskXObject','paintImageMaskXObjectGroup','paintImageMaskXObjectRepeat'].map(name=>OPS[name]).filter(Number.isInteger));
  const regions=[];
  operators.fnArray.forEach((op,index)=>{
    if(!imageOps.has(op)||bounds.isEmpty(index))return;
    const box=[Math.max(0,Math.floor(bounds.minX(index)*width)),Math.max(0,Math.floor(bounds.minY(index)*height)),Math.min(width,Math.ceil(bounds.maxX(index)*width)),Math.min(height,Math.ceil(bounds.maxY(index)*height))];
    if(box[2]>box[0]&&box[3]>box[1])regions.push({box,operations:[index]});
    if(regions.length>limit*16)throw Object.assign(new Error(`More than ${limit*16} rendered image operations exceed the bounded region-mapping budget.`),{code:'REGION_LIMIT'});
  });
  // Merge intersecting image footprints before recognition so each pixel is
  // routed once, even for tiled scans, repeated image operations and masks.
  for(let i=0;i<regions.length;i++)for(let j=i+1;j<regions.length;){
    const a=regions[i],b=regions[j];
    if(Math.max(a.box[0],b.box[0])<=Math.min(a.box[2],b.box[2])&&Math.max(a.box[1],b.box[1])<=Math.min(a.box[3],b.box[3])){
      a.box=[Math.min(a.box[0],b.box[0]),Math.min(a.box[1],b.box[1]),Math.max(a.box[2],b.box[2]),Math.max(a.box[3],b.box[3])];a.operations.push(...b.operations);regions.splice(j,1);i=-1;break;
    }else j++;
  }
  if(regions.length>limit)throw Object.assign(new Error(`PDF has ${regions.length} raster regions; the limit is ${limit}.`),{code:'REGION_LIMIT'});
  return regions.sort((a,b)=>a.box[1]-b.box[1]||a.box[0]-b.box[0]).map((region,index)=>({id:`raster:${index}`,...region,boundsSource:'pdfjs rendered image operation; outward 1/256 page quantization'}));
}

export function cropForOcr(rgba,width,region,cells,scale) {
  const [left,top,right,bottom]=region.box,w=right-left,h=bottom-top;
  const pixels=new Uint8Array(w*h*4);
  for(let y=0;y<h;y++)pixels.set(rgba.subarray(((y+top)*width+left)*4,((y+top)*width+right)*4),y*w*4);
  const masked=[];
  for(const cell of cells){
    if(!cell.text.trim())continue;
    // Only remove pixels already represented by native text. Text equality is
    // never a deduplication key: the same words elsewhere must survive.
    const a=cell.bbox.map(v=>v*scale),pad=2;
    const x0=Math.max(0,Math.floor(a[0]-left-pad)),y0=Math.max(0,Math.floor(a[1]-top-pad));
    const x1=Math.min(w,Math.ceil(a[2]-left+pad)),y1=Math.min(h,Math.ceil(a[3]-top+pad));
    if(x1<=x0||y1<=y0)continue;
    for(let y=y0;y<y1;y++)pixels.fill(255,(y*w+x0)*4,(y*w+x1)*4);
    masked.push(cell.id);
  }
  return {pixels,width:w,height:h,maskedNativeCells:masked};
}

export function mappedRegionBlocks(blocks,region,pageNo,scale) {
  return blocks.map(block=>({...block,id:`${region.id}:${block.id}`,source:'raster-region OCR',region:region.id,provenance:block.provenance.map(prov=>{
    const b=prov.bbox,h=(region.box[3]-region.box[1])/scale;
    const top=b.coord_origin==='BOTTOMLEFT'?h-b.t:b.t,bottom=b.coord_origin==='BOTTOMLEFT'?h-b.b:b.b;
    return {...prov,page_no:pageNo,bbox:{l:b.l+region.box[0]/scale,t:top+region.box[1]/scale,r:b.r+region.box[0]/scale,b:bottom+region.box[1]/scale,coord_origin:'TOPLEFT'}};
  })}));
}

export function topLeftBox(block,height){
  const b=block.provenance[0]?.bbox;if(!b)return null;
  return b.coord_origin==='BOTTOMLEFT'?[b.l,height-b.t,b.r,height-b.b]:[b.l,b.t,b.r,b.b];
}
