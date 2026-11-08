#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Audit all rendered pages, independently of the scoped Markdown check."""
from pathlib import Path
from html.parser import HTMLParser
from email.utils import parsedate_to_datetime
import collections
import json
import sys
import xml.etree.ElementTree as ET
from urllib.parse import urlsplit
from urllib.parse import unquote

ROOT=Path(__file__).resolve().parents[2]
WORK=ROOT/'.cache/docs'
WORK.mkdir(parents=True, exist_ok=True)
DIST=Path(sys.argv[1]).resolve() if len(sys.argv)>1 else ROOT/'website/dist'
class Page(HTMLParser):
    def __init__(self):
        super().__init__();self.ids=[];self.h1=0;self.main=0;self.lang=None;self.canonical=[];self.description=[];self.images=[];self.headings=[];self.in_main=False;self.title=False;self.title_text='';self.links=[]
        self.in_head=False;self.feeds=[]
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if tag=='head':self.in_head=True
        if self.in_head and tag=='link' and a.get('type')=='application/rss+xml':self.feeds.append(a)
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
        if tag=='head':self.in_head=False
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
    # Include copied HTML hosts as well as Astro pages in the discovery audit.
    for f in sorted(DIST.rglob('*.html')):
        page=Page();page.feed(f.read_text())
        feeds=[link for link in page.feeds if link.get('href')=='/rss.xml' and link.get('rel')=='alternate']
        if len(feeds)!=1:
            publication_failures.append(f'Missing or duplicate RSS discovery: {f.relative_to(DIST)}')
    try:
        feed=ET.parse(DIST/'rss.xml').getroot()
        if feed.tag!='rss' or feed.get('version')!='2.0':
            publication_failures.append('Feed must be RSS 2.0')
        channel=feed.find('channel')
        if channel is None:
            raise ValueError('RSS channel is missing')
        for field in ['title', 'description', 'link']:
            if not channel.findtext(field):
                publication_failures.append(f'RSS channel lacks {field}')
        seen=set()
        for item in channel.findall('item'):
            link=item.findtext('link') or ''
            parsed=urlsplit(link)
            path=unquote(parsed.path).rstrip('/')
            if parsed.scheme!='https' or parsed.netloc!='daybrite.dev' or not path.startswith('/blog/'):
                publication_failures.append(f'RSS item has an invalid permalink: {link}')
            elif not (DIST/path.lstrip('/')/'index.html').is_file():
                publication_failures.append(f'RSS item has no published page: {link}')
            if '/drafts/' in path:
                publication_failures.append(f'Draft in RSS: {link}')
            if not item.findtext('title') or not item.findtext('description'):
                publication_failures.append(f'RSS item lacks title/summary: {link}')
            if not (item.findtext('{http://purl.org/rss/1.0/modules/content/}encoded') or '').strip():
                publication_failures.append(f'RSS item lacks full article content: {link}')
            guid=item.findtext('guid')
            if guid!=link or guid in seen:
                publication_failures.append(f'RSS item has invalid/duplicate GUID: {link}')
            seen.add(guid)
            parsedate_to_datetime(item.findtext('pubDate') or '')
    except (OSError, ET.ParseError, ValueError, TypeError) as error:
        publication_failures.append(f'Invalid RSS feed: {error}')
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
        if f.is_relative_to(DIST / 'demos/shared-signal'):
            continue  # Embedded Wasm host, not a documentation route; live-demo.test.mjs validates it.
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
