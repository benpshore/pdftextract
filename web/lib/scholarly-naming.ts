/** Derived labels use reported header fields only; filenames remain immutable source evidence. */
export function scholarlyDisplayName(titles:string[],authors:string[],originalName:string){
  const uniqueTitles=[...new Set(titles.map(title=>title.trim()).filter(Boolean))];
  const reportedAuthors=authors.map(author=>author.trim()).filter(Boolean);
  const title=uniqueTitles.length===1?uniqueTitles[0]:null;
  const state=uniqueTitles.length>1?'ambiguous':title?'derived':'fallback';
  return {
    schema:'tpe.scholarly-display-name' as const,version:1 as const,
    value:title?(reportedAuthors.length?reportedAuthors[0]+(reportedAuthors.length>1?' et al.':'')+' — ':'')+title:originalName,
    state,template:'first reported author [et al.] — unique reported title; original filename fallback',
    source:'native-grobid-header' as const,reported_titles:titles,reported_authors:authors,original_name:originalName,
  };
}
