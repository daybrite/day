// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Exercise the shipped ABI against sharing availability, activation and cancellation.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const source = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
function setup(navigator) {
  const context = vm.createContext({TextEncoder,TextDecoder,Blob,navigator,document:{addEventListener(){}},ResizeObserver:class{}});
  vm.runInContext(source.replace('export async function start','async function start') + `
    wasm = {memory: {buffer:new ArrayBuffer(65536)}};
    globalThis.available = () => env.day_dom_share_support();
    globalThis.share = () => {
      const url=utf8enc.encode('https://fixture.invalid/story');
      const title=utf8enc.encode('Synthetic share fixture');
      mem().set(url,0);mem().set(title,1024);
      return env.day_dom_share_url(0,url.length,1024,title.length);
    };`,context);
  return context;
}
test('sharing requires both native support and live user activation', () => {
  assert.equal(setup({}).available(),false);
  assert.equal(setup({}).share(),false);
  const calls=[]; const c=setup({userActivation:{isActive:false},share:data=>{calls.push(data);return Promise.resolve();}});
  assert.equal(c.available(),true);assert.equal(c.share(),false);assert.equal(calls.length,0);
});
test('chooser receives the exact URL/title and cancellation does not trigger another action', async () => {
  const calls=[];const c=setup({userActivation:{isActive:true},canShare:()=>true,share:data=>{calls.push(data);return Promise.reject(new Error('synthetic cancellation'));}});
  assert.equal(c.share(),true);
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(calls.length,1);assert.equal(calls[0].url,'https://fixture.invalid/story');assert.equal(calls[0].title,'Synthetic share fixture');
});
test('invalid payloads and synchronous host rejection fail without opening a chooser', () => {
  const c=setup({userActivation:{isActive:true},canShare:()=>false,share:()=>{throw new Error('should not run');}});
  assert.equal(c.share(),false);
  const failed=setup({userActivation:{isActive:true},share:()=>{throw new Error('synthetic host failure');}});
  assert.equal(failed.share(),false);
});
