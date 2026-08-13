// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-webview — the HarmonyOS half of the piece (docs/extending.md, docs/webview.md).
//
// The ArkUI C node API has no Web node kind (native_node.h stops at the container types), so the
// component is declared here in ArkTS, built into a BuilderNode, and handed to Day as a FrameNode
// the native tree mounts like any other node. `day build` stages this file into the app's hvigor
// project and generates the aggregator that registers `dayPiece` with the shim.
//
// Commands arrive as the piece's own vocabulary ('load' | 'back' | 'forward' | 'stop' | 'reload')
// and are applied through the ArkTS WebviewController. Navigation reports the current URL back
// through `pieceEvent`, which reaches the piece's Rust front-end as an `Event::Custom` — so a
// bound text field follows along exactly as it does on the other backends.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import { webview } from '@kit.ArkWeb';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../DayPiece';

// One live web view: its BuilderNode (which owns the component) and the controller commands go
// through. Held until Day disposes the node — a Web engine instance is expensive and does not go
// away with its FrameNode.
class DayWebViewInstance {
  node: BuilderNode<[DayWebViewParams]>;
  controller: webview.WebviewController;

  constructor(node: BuilderNode<[DayWebViewParams]>, controller: webview.WebviewController) {
    this.node = node;
    this.controller = controller;
  }
}

class DayWebViewParams {
  id: number = 0;
  url: string = '';
  // Inline mode (docs/webview.md): the URL prefix of the bundled site under rawfile ('' =
  // remote mode). A main-document navigation outside it is BLOCKED in onLoadIntercept and
  // reported with num -1; the piece's Rust front-end runs the app's LinkPolicy.
  inlinePrefix: string = '';
  controller: webview.WebviewController = new webview.WebviewController();
}

// The component. `onPageEnd` fires for every committed navigation — including in-page link taps —
// which is what keeps the URL bar honest. `onLoadIntercept` sees each main-document request
// BEFORE it loads, which is where an inline site's link policy runs (subresources go through a
// different callback and are never policed).
@Builder
function dayWebViewBuilder(params: DayWebViewParams) {
  // NO percentage sizing: a BuilderNode is built detached, so '100%' resolves against the whole
  // window and the Web would cover the page. Day owns layout and sets this node's position + size
  // through the ArkUI C API like every other leaf, which is what the piece's `fill_measure` asks
  // for on every other backend too.
  Web({ src: params.url, controller: params.controller })
    .onPageEnd((event) => {
      if (event !== undefined) {
        nativeEntry.pieceEvent(params.id, event.url);
      }
    })
    .onLoadIntercept((event) => {
      if (params.inlinePrefix === '') {
        return false; // remote mode: never intercept
      }
      const url: string = event.data.getRequestUrl();
      if (url.startsWith(params.inlinePrefix) || url === 'about:blank') {
        return false; // in-site: let the engine navigate
      }
      // Leaving the site: block and report (num -1 = the link report); the Rust side runs the
      // app's LinkPolicy — system browser by default.
      nativeEntry.pieceEvent(params.id, url, -1);
      return true;
    })
}

const instances: Map<number, DayWebViewInstance> = new Map();

function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  // Decline where the system has no web engine rather than mount a component that cannot render.
  // A `Web` whose engine fails to load still creates its RosenWeb surface, and that surface wedges
  // the window's compositor for the rest of the process — every later frame goes stale, which is
  // far worse than Day's placeholder leaf. Declining reports the placeholder instead, exactly as
  // the GTK arm does where WebKitGTK is absent.
  if (!canIUse('SystemCapability.Web.Webview.Core')) {
    console.warn('day-piece-webview: no SystemCapability.Web.Webview.Core — rendering a placeholder');
    return undefined;
  }
  const controller: webview.WebviewController = new webview.WebviewController();
  const params: DayWebViewParams = new DayWebViewParams();
  params.id = id;
  // Inline mode arrives as `day-inline<US><root>[<US><start>]` (0x1F-separated, the piece's own
  // props encoding): the site lives under rawfile `day/<root>/` (resources/arkui.rs staging),
  // which ArkWeb browses through `resource://rawfile/` URLs — relative references resolve
  // within it natively.
  const SEP: string = '\u001F';
  if (props.startsWith('day-inline' + SEP)) {
    const parts: string[] = props.split(SEP);
    const root: string = parts.length > 1 ? parts[1] : '';
    const start: string = parts.length > 2 && parts[2] !== '' ? parts[2] : 'index.html';
    params.inlinePrefix = 'resource://rawfile/day/' + root + '/';
    params.url = params.inlinePrefix + start;
  } else {
    params.url = props;
  }
  params.controller = controller;
  const node: BuilderNode<[DayWebViewParams]> = new BuilderNode(ui);
  node.build(wrapBuilder<[DayWebViewParams]>(dayWebViewBuilder), params);
  const frame: FrameNode | null = node.getFrameNode();
  if (frame === null) {
    return undefined;
  }
  instances.set(id, new DayWebViewInstance(node, controller));
  return frame;
}

// An engine-level evaluation failure, in the 0x1F envelope the Rust front-end decodes
// (docs/webview-eval.md).
function dayEvalError(message: string): string {
  return '0\u001FArkWeb\u001F' + message;
}

// Normalize a `runJavaScript` reply. The front-end's wrapper makes every result a JS STRING;
// whether the engine hands that back raw or JSON-serialized (one outer quoted layer) was left
// unverified in docs/webview-eval.md, so both spellings are accepted. Empty and literal-null
// replies are engine failures (the wrapper catches script throws before they get that far).
function dayEvalPayload(result: string): string {
  if (result === '') {
    return dayEvalError('empty result');
  }
  if (result === 'null') {
    return dayEvalError('no result');
  }
  if (result.startsWith('"')) {
    try {
      const v: Object | null = JSON.parse(result);
      return typeof v === 'string' ? v : dayEvalError('non-string result');
    } catch (err) {
      return dayEvalError('unparsable result');
    }
  }
  return result;
}

function update(id: number, cmd: string, arg: string): void {
  const live: DayWebViewInstance | undefined = instances.get(id);
  if (live === undefined) {
    return;
  }
  // Evaluation (docs/webview-eval.md): `<req><script>`. The reply is MANDATORY on every
  // path — the Rust future resolves only when `pieceEvent` answers with `req` as the num — so
  // the pre-attach throw (17100001) and the promise rejection both answer with an engine error
  // rather than staying silent.
  if (cmd === 'eval') {
    const sep: number = arg.indexOf('\u001F');
    const req: number = Number(arg.substring(0, sep));
    const script: string = arg.substring(sep + 1);
    try {
      live.controller.runJavaScript(script).then((result: string) => {
        nativeEntry.pieceEvent(id, dayEvalPayload(result), req);
      }).catch((err: BusinessError) => {
        nativeEntry.pieceEvent(id, dayEvalError(String(err.message)), req);
      });
    } catch (err) {
      nativeEntry.pieceEvent(id, dayEvalError(String(err)), req);
    }
    return;
  }
  // Every controller call throws before the Web component finishes initializing (and `forward`
  // /`backward` throw with nowhere to go), which would otherwise take the whole app down.
  try {
    if (cmd === 'load') {
      live.controller.loadUrl(arg);
    } else if (cmd === 'back') {
      if (live.controller.accessBackward()) {
        live.controller.backward();
      }
    } else if (cmd === 'forward') {
      if (live.controller.accessForward()) {
        live.controller.forward();
      }
    } else if (cmd === 'stop') {
      live.controller.stop();
    } else if (cmd === 'reload') {
      live.controller.refresh();
    }
  } catch (err) {
    console.error(`day-piece-webview: ${cmd} failed: ${err}`);
  }
}

function dispose(id: number): void {
  const live: DayWebViewInstance | undefined = instances.get(id);
  if (live !== undefined) {
    instances.delete(id);
    live.node.dispose();
  }
}

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.webview',
  make: make,
  update: update,
  dispose: dispose
};
