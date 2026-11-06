// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import {res} from './res.mjs';
const s=res.str;
const el=(tag,cls,text)=>{const n=document.createElement(tag);if(cls)n.className=cls;if(text!==undefined)n.textContent=text;return n};
const button=(text,fn,cls='')=>{const n=el('button',cls,text);n.type='button';n.addEventListener('click',fn);return n};
const note=el('div','review-page-note',s.pageNote());document.body.prepend(note);
const status=el('p','review-status',s.loading());status.setAttribute('role','status');status.setAttribute('aria-live','polite');
const toggle=button(s.review(),()=>{bar.hidden=!bar.hidden;toggle.setAttribute('aria-expanded',String(!bar.hidden))},'review-toggle');
const bar=el('aside','review-bar');bar.setAttribute('aria-label',s.reviewTitle());bar.id='review-bar';
toggle.setAttribute('aria-controls',bar.id);toggle.setAttribute('aria-expanded','true');
const dialog=el('dialog','review-dialog');
document.body.append(toggle,bar,dialog);
let data,current=0,previews={},saved={version:0,records:{}},token='',busy=false,storageOK=true;
const storageKey='day-landing-review-v1';
const optionName=id=>id==='original'?s.original():id;
const valuesFor=(seg,p)=>p?.values|| (p?.option&&p.option!=='original'?seg.options.find(o=>o.id===p.option)?.values:null)||Object.fromEntries(seg.fields.map(f=>[f.id,f.text]));
const recordFor=seg=>previews[seg.id]||saved.records[seg.id]||{option:'original',values:valuesFor(seg)};
function persist(){try{localStorage.setItem(storageKey,JSON.stringify({revision:data.revision,previews,current}));}catch{storageOK=false;status.textContent=s.storageUnavailable()}}
function updatePage(seg){
 const p=recordFor(seg), values=valuesFor(seg,p);
 for(const f of seg.fields){
  const n=document.querySelector(`[data-review-segment="${seg.id}"] [data-copy-field="${f.id}"]`);
  const text=values[f.id];
  n.hidden=f.kind==='eyebrow'&&!text.trim();
  if(text===f.text){n.innerHTML=f.html;}
  else if(f.kind==='h1'){
   n.replaceChildren();const lines=text.split('\n');
   lines.forEach((line,i)=>{if(i)n.append(document.createElement('br'));n.append(i===lines.length-1?el('span','hero-day',line):document.createTextNode(line))});
  }else{n.textContent=text;n.style.whiteSpace='pre-line'}
 }
 const chip=document.querySelector(`[data-review-button="${seg.id}"]`);
 if(chip)chip.textContent=saved.records[seg.id]?s.savedOption({option:saved.records[seg.id].option+(saved.records[seg.id].edited?' · '+s.custom():'')}):s.reviewSection();
}
function setCurrent(index,scroll=true){
 current=(index+data.segments.length)%data.segments.length;
 bar.hidden=false;toggle.setAttribute('aria-expanded','true');
 drawBar();persist();
 if(scroll)document.querySelector(`[data-review-segment="${data.segments[current].id}"]`).scrollIntoView({behavior:'smooth',block:'start'});
}
function preview(seg,option){
 previews[seg.id]={option,values:{...valuesFor(seg,{option})}};
 updatePage(seg);drawBar();persist();status.textContent=s.previewing({option:optionName(option)});
}
async function choose(seg){
 if(busy)return;
 busy=true;drawBar();status.textContent=s.saving();
 const p=recordFor(seg);const base=valuesFor(seg,{option:p.option});
 const record={option:p.option,values:{...valuesFor(seg,p)},edited:seg.fields.some(f=>p.values?.[f.id]!==undefined&&p.values[f.id]!==base[f.id])};
 try{
  const response=await fetch('/api/choice',{method:'POST',headers:{'Content-Type':'application/json','X-Review-Token':token},body:JSON.stringify({revision:data.revision,base_version:saved.version,segment:seg.id,record})});
  if(!response.ok)throw new Error(response.status===409?'conflict':'save');
  saved=(await response.json()).state;status.textContent=s.saved();updatePage(seg);
 }catch(error){status.textContent=error.message==='conflict'?s.conflict():s.saveFailed()}
 finally{busy=false;drawBar();persist()}
}
function drawBar(){
 const seg=data.segments[current],p=recordFor(seg);
 bar.replaceChildren();
 const top=el('div','review-bar-top');
 const progress=el('strong','',s.progress({count:Object.keys(saved.records).length,total:data.segments.length}));
 const select=el('select');select.setAttribute('aria-label',s.section());
 data.segments.forEach((x,i)=>{const o=el('option','',`${i+1}. ${x.name}${saved.records[x.id]?' ✓':''}`);o.value=String(i);select.append(o)});select.value=String(current);select.onchange=()=>setCurrent(Number(select.value));
 top.append(progress,button('←',()=>setCurrent(current-1)),select,button('→',()=>setCurrent(current+1)),button('×',()=>{bar.hidden=true;toggle.setAttribute('aria-expanded','false')}));
 top.children[1].setAttribute('aria-label',s.previous());top.children[3].setAttribute('aria-label',s.next());top.children[4].setAttribute('aria-label',s.hide());
 const options=el('div','review-options');options.setAttribute('role','group');options.setAttribute('aria-label',s.preview());
 for(const id of ['original',...seg.options.map(o=>o.id)]){
  const b=button(optionName(id),()=>preview(seg,id));b.setAttribute('aria-pressed',String(p.option===id));
  b.title=id==='original'?s.original():seg.options.find(o=>o.id===id).label;options.append(b);
 }
 const chooseButton=button(s.choose(),()=>choose(seg),'review-primary');chooseButton.disabled=busy;options.append(chooseButton);
 const actions=el('div','review-actions');
 actions.append(button(s.compare(),()=>compare(seg)),button(s.edit(),()=>edit(seg)),button(s.summary(),summary),button(s.export(),exportChoices),button('◐',()=>{document.documentElement.dataset.theme=document.documentElement.dataset.theme==='dark'?'light':'dark'}));
 actions.lastChild.setAttribute('aria-label',s.theme());
 bar.append(top,options,actions,status);
 toggle.textContent=s.review()+' · '+Object.keys(saved.records).length+'/'+data.segments.length;
}
function showDialog(title){
 dialog.replaceChildren();
 const header=el('header','review-dialog-head');const heading=el('h2','',title);heading.id='review-dialog-title';
 header.append(heading,button(s.close(),()=>dialog.close()));dialog.append(header);dialog.setAttribute('aria-labelledby',heading.id);
 if(!dialog.open)dialog.showModal();
}
dialog.addEventListener('click',e=>{if(e.target===dialog){const r=dialog.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)dialog.close()}});
function compare(seg){
 showDialog(s.compareTitle({section:seg.name}));
 dialog.append(el('p','review-help',s.intro()));
 const grid=el('div','review-comparisons');
 const original={id:'original',label:s.original(),values:valuesFor(seg,{option:'original'})};
 for(const option of [original,...seg.options]){
  const card=el('article','review-comparison');
  card.append(el('h3','',option.id==='original'?s.original():s.option({id:option.id,label:option.label})));
  const actions=el('div','review-actions');
  actions.append(button(s.preview(),()=>{preview(seg,option.id);dialog.close();setCurrent(current)}),button(s.choose(),async()=>{preview(seg,option.id);await choose(seg);dialog.close()},'review-primary'));card.append(actions);
  const copy=el('div','review-option-copy');
  for(const f of seg.fields){
   const value=option.values[f.id];const tag=['h1','h2'].includes(f.kind)?'h4':f.kind==='h3'?'h5':'p';
   const n=el(tag,'review-copy-'+f.kind,value||s.omit());copy.append(n);
  }
  card.append(copy);grid.append(card);
 }
 dialog.append(grid);
}
function edit(seg){
 showDialog(s.editTitle({section:seg.name}));dialog.append(el('p','review-help',s.editHelp()));
 const initial=recordFor(seg);previews[seg.id]={option:initial.option,values:{...valuesFor(seg,initial)}};
 const fields=el('div','review-edit-fields');
 const kinds={h1:s.heading,h2:s.heading,h3:s.subheading,p:s.paragraph,a:s.link,eyebrow:s.eyebrow};
 for(const [index,f] of seg.fields.entries()){
  const label=el('label');label.append(el('span','',s.field({kind:kinds[f.kind](),number:index+1})));
  const input=el('textarea');input.rows=f.kind==='p'?4:2;input.value=previews[seg.id].values[f.id];
  input.addEventListener('input',()=>{previews[seg.id].values[f.id]=input.value;updatePage(seg);persist()});label.append(input);fields.append(label);
 }
 dialog.append(fields,el('p','review-help',s.editFootnote()),button(s.choose(),async()=>{await choose(seg);dialog.close()},'review-primary'));
}
function summary(){
 showDialog(s.summaryTitle());dialog.append(el('p','review-help',s.summaryHelp()));
 for(const [index,seg] of data.segments.entries()){
  const row=el('div','review-summary-row'),record=saved.records[seg.id];
  row.append(el('strong','',seg.name),el('span','',record?s.savedOption({option:optionName(record.option)+(record.edited?' · '+s.custom():'')}):s.unselected()),button(s.reviewItem(),()=>{dialog.close();setCurrent(index)}));dialog.append(row);
 }
 dialog.append(button(s.export(),exportChoices,'review-primary'));
}
function exportChoices(){
 const result={schema:1,revision:data.revision,source_commit:data.source_commit,source_sha256:data.source_sha256,exported_at:new Date().toISOString(),saved_choices:saved.records,previews};
 const url=URL.createObjectURL(new Blob([JSON.stringify(result,null,2)+'\n'],{type:'application/json'}));
 const a=el('a');a.href=url;a.download=s.exportName();a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);status.textContent=s.exported();
}
async function init(){
 data=await fetch('/proposals.json').then(r=>{if(!r.ok)throw Error('data');return r.json()});
 try{const local=JSON.parse(localStorage.getItem(storageKey)||'null');if(local?.revision===data.revision){previews=local.previews||{};current=Number.isInteger(local.current)&&local.current>=0&&local.current<data.segments.length?local.current:0;}}catch{storageOK=false}
 try{const response=await fetch('/api/state');if(!response.ok)throw Error('state');const remote=await response.json();saved=remote.state;token=remote.token;status.textContent=s.ready()}catch{status.textContent=s.offline()}
 for(const [index,seg] of data.segments.entries()){
  const section=document.querySelector(`[data-review-segment="${seg.id}"]`);
  const b=button(s.reviewSection(),()=>setCurrent(index),'review-section-button');b.dataset.reviewButton=seg.id;section.append(b);updatePage(seg);
 }
 drawBar();if(!storageOK)status.textContent=s.storageUnavailable();
 // Snapshot navigation is decorative except ordinary destination links.
 for(const n of document.querySelectorAll('nav button, nav details, [data-search-open], [data-theme-toggle]'))n.hidden=true;
}
init().catch(()=>{bar.append(status);status.textContent=s.failedLoad()});
