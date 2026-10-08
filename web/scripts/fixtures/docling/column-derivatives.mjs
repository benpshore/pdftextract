// Derivatives only of the verified native and PR269 witnesses. The expected
// sequence is authored here before extraction, independent of model/geometry.
export async function columnDerivatives(PDFDocument,StandardFonts,fixtures,expectedNative){
  const results=[];
  const two=['HEADING','left first row','left second row','right first row','right second row'];
  const three=['CENTER HEADING','L1','L2','M1','M2','R1','R2'];
  const bands=await PDFDocument.create(),page=bands.addPage([612,792]);
  page.drawPage((await bands.embedPdf(fixtures.get('semantic-two.pdf')))[0],{x:30,y:430,width:480,height:360});
  page.drawPage((await bands.embedPdf(fixtures.get('semantic-three.pdf')))[0],{x:30,y:60,width:480,height:360});
  results.push({name:'columns-bands.pdf',bytes:await bands.save(),expected:[...two,...three],purpose:'Separated bands change from two to three columns'});
  for(const footer of [false,true]){
    const pdf=await PDFDocument.create(),p=pdf.addPage([480,360]),font=await pdf.embedFont(StandardFonts.Courier);
    // Scramble stream order and stagger the two columns' baselines.
    const lines=[['right second row',260,208],['left first row',24,248],[footer?expectedNative[0]:'HEADING',footer?24:210,footer?276:288],['right first row',260,232],['left second row',24,224]];
    if(footer)lines.unshift([expectedNative[2],24,180]);
    for(const [text,x,y] of lines)p.drawText(text,{x,y,size:12,font});
    results.push({name:footer?'columns-spanning.pdf':'columns-staggered.pdf',bytes:await pdf.save(),expected:footer?[expectedNative[0],...two.slice(1),expectedNative[2]]:two,purpose:footer?'Full-width heading/footer around staggered columns':'Unequal, staggered column baselines'});
  }
  return results;
}
