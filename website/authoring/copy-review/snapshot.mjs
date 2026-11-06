// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import fs from 'node:fs';
import path from 'node:path';
import {parse, serialize} from 'parse5';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
// This script writes only the local review snapshot. It is not an Astro integration.
const tool=path.dirname(fileURLToPath(import.meta.url));
const website=path.resolve(tool,'../..');
const root=path.join(tool,'.local');
fs.mkdirSync(root,{recursive:true});
const dist=path.join(website,'dist');
const doc=parse(fs.readFileSync(path.join(dist,'index.html'),'utf8'));
const attr=(n,k)=>n.attrs?.find(a=>a.name===k)?.value;
const set=(n,k,v)=>{n.attrs??=[];const a=n.attrs.find(a=>a.name===k);if(a)a.value=v;else n.attrs.push({name:k,value:v})};
const has=(n,c)=>(attr(n,'class')||'').split(' ').includes(c);
function all(n,test,out=[]){if(test(n))out.push(n);for(const c of n.childNodes||[])all(c,test,out);return out}
const text=n=>n.nodeName==='#text'?n.value:n.tagName==='br'?'\n':(n.childNodes||[]).map(text).join('');
const remove=n=>{n.parentNode.childNodes=n.parentNode.childNodes.filter(c=>c!==n)};
for(const n of all(doc,n=>n.tagName==='script'||n.tagName==='astro-dev-toolbar'||(n.tagName==='link'&&attr(n,'rel')==='canonical')||(n.tagName==='meta'&&['og:url','og:title','og:description'].includes(attr(n,'property')))))remove(n);
const head=all(doc,n=>n.tagName==='head')[0];
const extra=parse('<meta name="robots" content="noindex,nofollow"><link rel="stylesheet" href="/review.css"><script type="module" src="/review.js"></script>').childNodes.find(n=>n.tagName==='html').childNodes.find(n=>n.tagName==='head').childNodes;
head.childNodes.push(...extra);all(doc,n=>n.tagName==='title')[0].childNodes[0].value='Day landing page · Copy review';
set(all(doc,n=>n.tagName==='html')[0],'data-theme','light');
const main=all(doc,n=>n.tagName==='main')[0];
const sections=(main.childNodes||[]).filter(n=>n.tagName==='section');
const names=['Hero','Shared app code','Rust and native controls','Development workflow','Existing Rust libraries','Languages and accessibility','Extensions and open source','Getting started'];
const ids=['hero','shared','native','workflow','library','languages','extensions','start'];
// Field order is part of the draft schema. If the landing page changes structure,
// update the section names/IDs and drafts.py together; build.py checks field coverage.
if(sections.length!==ids.length)throw new Error('Landing section count changed; update the review schema.');
const inventory=[];
for(let i=0;i<sections.length;i++){
 const section=sections[i];set(section,'data-review-segment',ids[i]);set(section,'id',`review-${ids[i]}`);
 const candidates=all(section,n=>['h1','h2','h3','p','a'].includes(n.tagName));
 const fields=[];
 for(const n of candidates){
  let excluded=false;
  for(let p=n;p&&p!==section;p=p.parentNode){if(['hero-media','platform-strip','support-note'].some(c=>has(p,c)))excluded=true;}
  if(excluded || (n.tagName==='h3'&&all(n,c=>c.tagName==='a').length))continue;
  if(n.tagName==='a'){
   let skip=false;for(let p=n.parentNode;p&&p!==section;p=p.parentNode){if(['p','h1','h2'].includes(p.tagName))skip=true;}
   if(skip)continue;
  }
  const id=`f${fields.length}`;set(n,'data-copy-field',id);
  fields.push({id,kind:has(n,'eyebrow')?'eyebrow':n.tagName,text:text(n).trim(),html:serialize(n)});
 }
 inventory.push({id:ids[i],name:names[i],fields});
}
// Freeze one real screenshot so comparisons aren't interrupted by animation.
const slides=all(doc,n=>has(n,'hc-slide'));
for(let i=0;i<slides.length;i++)if(i!==1)remove(slides[i]);
if(slides[1])set(slides[1],'class','hc-slide is-active');
for(const n of all(doc,n=>has(n,'hc-track')||has(n,'hc-icons')))remove(n);
for(const n of all(doc,n=>has(n,'hc-os')))n.childNodes=[{nodeName:'#text',value:'macOS',parentNode:n}];
for(const n of all(doc,n=>has(n,'hc-toolkit')))n.childNodes=[{nodeName:'#text',value:'AppKit · Day Showcase',parentNode:n}];
fs.mkdirSync(path.join(root,'assets'),{recursive:true});
for(const n of all(doc,n=>n.tagName==='link'||n.tagName==='img')){
 const key=n.tagName==='img'?'src':'href';const src=attr(n,key);
 if(!src?.startsWith('/')||src.startsWith('//'))continue;
 const file=path.join(dist,src);if(!fs.existsSync(file))continue;
 const dest='assets/'+src.slice(1).replaceAll('/','_');fs.copyFileSync(file,path.join(root,dest));set(n,key,'/'+dest);
}
// Documentation links open the live site without losing the review.
for(const n of all(doc,n=>n.tagName==='a')){
 const href=attr(n,'href');if(href?.startsWith('/')&&!href.startsWith('//')){set(n,'href','https://daybrite.dev'+href);set(n,'target','_blank');set(n,'rel','noopener');}
}
fs.writeFileSync(path.join(root,'index.html'),serialize(doc));
fs.writeFileSync(path.join(root,'inventory.json'),JSON.stringify(inventory,null,2)+'\n');
const source=fs.readFileSync(path.join(website,'src/pages/index.astro'));
const meta={source_commit:execFileSync('git',['rev-parse','HEAD'],{cwd:website,encoding:'utf8'}).trim(),source_sha256:createHash('sha256').update(source).digest('hex')};
fs.writeFileSync(path.join(root,'source.json'),JSON.stringify(meta,null,2)+'\n');
console.log(`Prepared ${inventory.length} sections for local copy review.`);
