/** Parse local markup through an inert template before any resource-bearing DOM.
 * Links remain evidence/navigation; resources are never loaded from source URLs.
 */
export function passiveHtmlDocument(source:string, allowImage:(src:string)=>boolean = ()=>false):Document {
  const template=document.createElement('template');
  template.innerHTML=source;
  for(const node of Array.from(template.content.querySelectorAll('[hidden],[aria-hidden="true"],[inert],[style]'))){
    const style=(node.getAttribute('style')||'').replace(/\s+/g,'').toLowerCase();
    if(node.hasAttribute('hidden')||node.getAttribute('aria-hidden')==='true'||node.hasAttribute('inert')||/(?:^|;)display:none(?:!important)?(?:;|$)|(?:^|;)visibility:hidden(?:!important)?(?:;|$)/.test(style))node.remove();
  }
  for(const node of Array.from(template.content.querySelectorAll('iframe,object,embed,link,style,svg,audio,video,source,track,picture,noscript,form,input,button,select,textarea')))node.remove();
  for(const script of Array.from(template.content.querySelectorAll('script')))if(script.getAttribute('type')!=='application/ld+json')script.remove();
  for(const node of Array.from(template.content.querySelectorAll('*'))){
    const image=node.tagName==='IMG'&&allowImage(node.getAttribute('src')||'');
    if(node.tagName==='IMG'&&!image){node.remove();continue;}
    for(const attr of Array.from(node.attributes))if(/^on/i.test(attr.name)||['srcset','background','poster','ping','action','formaction','srcdoc','style','http-equiv'].includes(attr.name)||(attr.name==='src'&&!image))node.removeAttribute(attr.name);
  }
  return new DOMParser().parseFromString(template.innerHTML,'text/html');
}
