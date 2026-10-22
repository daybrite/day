// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import accentTargets from '../plugins/accent-target-code.mjs';
import admonitions from '../plugins/admonitions.mjs';
const text = value => ({type:'text',value});
const element = (tagName,...children) => ({type:'element',tagName,properties:{},children});
const nodes = node => [node,...(node.children??[]).flatMap(nodes)];
const labels = node => nodes(node).filter(n => n.properties?.className?.includes('doc-target-label'));

test('toolkit notes and tables reuse target marks, including secondary combinations',()=>{
  const tree=element('div',element('strong',text('AppKit / UIKit: behavior.')),
    element('th',text('GTK')),element('td',text('Qt')),
    element('code',text('windows-gtk')));
  accentTargets()(tree);
  assert.deepEqual(labels(tree).map(n=>n.properties['data-pf']),['macos-appkit','ios-uikit','linux-gtk','linux-qt','windows-gtk']);
  for(const n of labels(tree))assert.equal(n.children[0].properties.ariaHidden,'true');
  assert.match(tree.children[3].children[0].children[0].properties.style,/data:image\/svg\+xml/);
});
test('code blocks, API names, and ordinary prose are preserved',()=>{
  const tree=element('div',element('pre',element('code',text('macos-appkit'))),
    element('p',text('AppKit renders this control.')),
    element('td',element('code',text('Qt::SomeType'))));
  const original=structuredClone(tree);accentTargets()(tree);assert.deepEqual(tree,original);
});
test('a platform warning retains its severity, text, and stable anchor',()=>{
  const tree=element('div',element('blockquote',element('p',text('[!WARNING]\n'),
    element('strong',text('Windows / WinUI: alpha selection.')),text('\nNative alpha selection is unverified.'))));
  accentTargets()(tree);admonitions()(tree);
  const warning=tree.children[0];assert.equal(warning.tagName,'aside');
  assert.equal(warning.properties['data-kind'],'warning');
  assert.equal(warning.properties.id,'admonition-windows-winui-alpha-selection');
  assert.equal(labels(warning).length,2);
  assert.ok(nodes(warning).some(n=>n.value?.includes('Native alpha selection is unverified.')));
});


test('lowercase, mixed markup, and mid-heading names receive marks without changing text', () => {
  const link = element('a', element('code', text('NSSplitView')));
  link.properties.href = 'https://developer.apple.com/documentation/appkit/nssplitview';
  const tree = element('div',
    element('h3', text('The iOS tabs host is driven by '), element('code', text('UITab'))),
    element('td', text('gtk / qt / xaml')),
    element('strong', text('macOS '), link, text(' / Qt '), element('code', text('QSplitter'))),
    element('th', element('a', text('Android'))),
    element('td', element('code', text('arkui'))));
  const content = n => n.type === 'text' ? n.value : (n.children ?? []).map(content).join('');
  const before = content(tree);
  accentTargets()(tree);
  assert.equal(content(tree), before);
  assert.deepEqual(labels(tree).map(n => n.properties['data-pf']), [
    'ios-uikit', 'linux-gtk', 'linux-qt', 'windows-winui', 'macos-appkit', 'linux-qt',
    'android-mdc', 'harmony-arkui',
  ]);
  assert.equal(link.properties.href, 'https://developer.apple.com/documentation/appkit/nssplitview');
  assert.equal(labels(link).length, 0);
});

test('platform-section prose is decorated once per paragraph and ends at the next peer heading', () => {
  const tree = element('div', element('h2', text('Per-toolkit native realization')),
    element('p', text('GTK and Qt use native controls. GTK measures its child.')),
    element('h3', text('Sizing')), element('p', text('AppKit measures the content.')),
    element('h2', text('Authoring')), element('p', text('AppKit renders this control.')),
    element('pre', element('code', text('gtk Qt AppKit'))));
  accentTargets()(tree);
  assert.deepEqual(labels(tree).map(n => n.properties['data-pf']), ['linux-gtk', 'linux-qt', 'macos-appkit']);
  assert.equal(labels(tree.children[5]).length, 0);
  assert.equal(labels(tree.children[6]).length, 0);
});

test('OS-wide labels retain both relevant target marks', () => {
  const tree = element('th', text('Linux / Apple'));
  accentTargets()(tree);
  assert.deepEqual(labels(tree).map(n => n.properties['data-targets']), [
    'linux-gtk linux-qt', 'macos-appkit ios-uikit',
  ]);
  for (const n of labels(tree)) assert.equal(n.children.filter(c => c.properties?.ariaHidden === 'true').length, 2);
});


test('tight list items have independent labels, including sentence-final names', () => {
  const tree = element('div', element('h2', text('Toolkit notes')), element('ul',
    element('li', element('strong', text('GTK / Qt')), text(' measures content.')),
    element('li', element('strong', text('macOS ')), text(' has a different result than Qt.'))));
  accentTargets()(tree);
  assert.deepEqual(labels(tree).map(n => n.properties['data-pf']), [
    'linux-gtk', 'linux-qt', 'macos-appkit', 'linux-qt',
  ]);
});
