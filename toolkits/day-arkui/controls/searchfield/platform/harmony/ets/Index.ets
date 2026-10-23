// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-searchfield: the HarmonyOS half of the piece (docs/extending.md). HarmonyOS's search
// field is the ArkTS `Search` component (magnifier, clear button, the system's search keyboard);
// the ArkUI C node API has no search kind, so it is built here in a BuilderNode and mounted by
// day-arkui like any node. Edits go back as Day's own `TextChanged` (pieceEvent kind 1), the
// event every other backend's field sends.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../DayPiece';

const SEP: string = '\u001F';
// `pieceEvent`'s kinds for Day's `TextChanged` and `Submitted`.
const DAY_TEXT_CHANGED: number = 1;
const DAY_SUBMITTED: number = 17;

class DaySearchParams {
  id: number = 0;
  text: string = '';
  placeholder: string = '';
}

@Builder
function daySearchBuilder(params: DaySearchParams) {
  // No width: the field takes its parent's, which is the wrapper day-arkui lays out ('100%'
  // would resolve against the window, since a BuilderNode is built detached).
  Search({ value: params.text, placeholder: params.placeholder })
    .onChange((value: string) => {
      if (value !== params.text) {
        params.text = value;
        nativeEntry.pieceEvent(params.id, value, 0, DAY_TEXT_CHANGED);
      }
    })
    .onSubmit(() => {
      nativeEntry.pieceEvent(params.id, '', 0, DAY_SUBMITTED);
    })
}

const nodes: Map<number, BuilderNode<[DaySearchParams]>> = new Map();
const current: Map<number, DaySearchParams> = new Map();

// Props: `<text><US><placeholder>`.
function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  const separator: number = props.indexOf(SEP);
  const params: DaySearchParams = new DaySearchParams();
  params.id = id;
  params.text = separator < 0 ? props : props.substring(0, separator);
  params.placeholder = separator < 0 ? '' : props.substring(separator + 1);
  const node: BuilderNode<[DaySearchParams]> = new BuilderNode(ui);
  try {
    node.build(wrapBuilder<[DaySearchParams]>(daySearchBuilder), params);
    const frame: FrameNode | null = node.getFrameNode();
    if (frame !== null) {
      nodes.set(id, node);
      current.set(id, params);
      return frame;
    }
  } catch (err) {
    console.error(`day-piece-searchfield: component creation failed: ${err}`);
  }
  node.dispose();
  return undefined;
}

// The one command: `text`, the app writing the query (a programmatic set is not an edit, so
// it is not reported back).
function update(id: number, cmd: string, arg: string): void {
  const node: BuilderNode<[DaySearchParams]> | undefined = nodes.get(id);
  const was: DaySearchParams | undefined = current.get(id);
  if (cmd !== 'text' || node === undefined || was === undefined || was.text === arg) {
    return;
  }
  const next: DaySearchParams = new DaySearchParams();
  next.id = id;
  next.text = arg;
  next.placeholder = was.placeholder;
  current.set(id, next);
  node.update(next);
}

function dispose(id: number): void {
  const node: BuilderNode<[DaySearchParams]> | undefined = nodes.get(id);
  nodes.delete(id);
  current.delete(id);
  node?.dispose();
}

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.searchfield',
  make: make,
  update: update,
  dispose: dispose
};
