#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Audit all rendered pages, independently of the scoped Markdown check."""
from pathlib import Path
from html.parser import HTMLParser
import collections
import json
import sys
import xml.etree.ElementTree as ET
from urllib.parse import urlsplit

ROOT=Path(__file__).resolve().parents[2]
WORK=ROOT/'.cache/docs'
WORK.mkdir(parents=True, exist_ok=True)
DIST=Path(sys.argv[1]).resolve() if len(sys.argv)>1 else ROOT/'website/dist'
class Page(HTMLParser):
    def __init__(self):
        super().__init__();self.ids=[];self.h1=0;self.main=0;self.lang=None;self.canonical=[];self.description=[];self.images=[];self.headings=[];self.in_main=False;self.title=False;self.title_text='';self.links=[]
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if 'id' in a:self.ids.append(a['id'])
        if tag=='html':self.lang=a.get('lang')
        if tag=='main':self.main+=1;self.in_main=True
        if tag=='h1':self.h1+=1
        if self.in_main and tag in ['h1','h2','h3','h4','h5','h6']:self.headings.append(int(tag[1]))
        if tag=='img':self.images.append(a)
        if tag=='link' and a.get('rel')=='canonical':self.canonical.append(a.get('href'))
        if tag=='meta' and a.get('name')=='description':self.description.append(a.get('content'))
        if tag=='a':self.links.append(a.get('href'))
        if tag=='title':self.title=True
    def handle_endtag(self,tag):
        if tag=='main':self.in_main=False
        if tag=='title':self.title=False
    def handle_data(self,d):
        if self.title:self.title_text+=d

def main():
    results=[];sitemap={n.text for n in ET.parse(DIST/'sitemap.xml').findall('.//{http://www.sitemaps.org/schemas/sitemap/0.9}loc')}
    catalog=json.loads((ROOT/'website/src/data/content-routes.json').read_text())
    unpublished={p['route'] for p in catalog.values() if p.get('published') is False}
    unpublished.update(['/contributing/design', '/contributing/backend-contracts',
        '/contributing/backend-contracts/element-ids', '/contributing/backend-contracts/testing',
        '/contributing/backend-contracts/native-project-migration'])
    publication_failures=[]
    for route in sorted(unpublished):
        if (DIST/route.lstrip('/')/'index.html').exists():
            publication_failures.append(f'Unpublished source rendered: {route}')
        if 'https://daybrite.dev'+route in sitemap:
            publication_failures.append(f'Unpublished source in sitemap: {route}')
    for f in sorted(DIST.rglob('index.html')):
        if f.is_relative_to(DIST / 'contributing/backend-contracts'):
            publication_failures.append(f'Retired backend-contract route rendered: {f}')
        if f.is_relative_to(DIST / 'api'):
            continue  # Rustdoc has its own templates and a separate bundle check.
        route='/' if f.parent==DIST else '/'+str(f.parent.relative_to(DIST));p=Page();p.feed(f.read_text());issues=[]
        for href in p.links:
            link=urlsplit(href or '')
            if link.netloc in ('', 'daybrite.dev') and link.path.rstrip('/') in unpublished:
                issues.append(f'Link to unpublished source: {href}')
        if p.h1!=1:issues.append(f'H1 count: {p.h1}')
        if p.main!=1:issues.append(f'main count: {p.main}')
        if p.lang!='en':issues.append('Missing/wrong document language')
        if not p.title_text:issues.append('Missing title')
        if len(p.description)!=1 or not p.description[0]:issues.append('Missing description')
        if p.canonical!=['https://daybrite.dev'+route]:issues.append('Wrong canonical URL')
        if 'https://daybrite.dev'+route not in sitemap:issues.append('Missing sitemap entry')
        duplicates=[k for k,n in collections.Counter(p.ids).items() if n>1]
        if duplicates:issues.append('Duplicate IDs: '+', '.join(duplicates))
        if any('alt' not in i for i in p.images):issues.append('Image missing alt attribute')
        previous=0
        for level in p.headings:
            if level>previous+1:issues.append(f'Heading jump: {previous} to {level}')
            previous=level
        results.append({'route':route,'title':p.title_text,'issues':issues})
    out={'pages':len(results),'failed':[r for r in results if r['issues']],'publication_failures':publication_failures,'results':results}
    (WORK/'site-audit.json').write_text(json.dumps(out,indent=2)+'\n')
    print(json.dumps({'pages':out['pages'],'failed':out['failed'],'publication_failures':publication_failures},indent=2))
    return int(bool(out['failed'] or publication_failures))
if __name__=='__main__':sys.exit(main())
