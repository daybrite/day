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
  controller: webview.WebviewController = new webview.WebviewController();
}

// The component. `onPageEnd` fires for every committed navigation — including in-page link taps —
// which is what keeps the URL bar honest.
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
  params.url = props;
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

function update(id: number, cmd: string, arg: string): void {
  const live: DayWebViewInstance | undefined = instances.get(id);
  if (live === undefined) {
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
