// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-combobox: the HarmonyOS half of the piece (docs/extending.md, docs/combobox.md).
// HarmonyOS has no combo box, and the ArkUI C node API has no menu to hang suggestions on, so the
// control is composed here in ArkTS: a `TextInput` for free-form entry beside a chevron button
// whose menu lists the suggestions. Typing and picking both report Day's own `TextChanged`
// (pieceEvent kind 1), the event every other backend's combo box sends; a pick fills the entry,
// as a native combo box's does.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../DayPiece';

const SEP: string = '\u001F';
// `pieceEvent`'s kind for Day's `TextChanged`.
const DAY_TEXT_CHANGED: number = 1;

class DayComboParams {
  id: number = 0;
  text: string = '';
  placeholder: string = '';
  items: string[] = [];
  // The frame day-arkui laid the wrapper out at (vp), from `day.frame`; 0 until it says.
  width: number = 0;
  height: number = 0;
}

function report(params: DayComboParams, value: string): void {
  if (value !== params.text) {
    params.text = value;
    nativeEntry.pieceEvent(params.id, value, 0, DAY_TEXT_CHANGED);
  }
}

@Builder
function dayComboBuilder(params: DayComboParams) {
  Row({ space: 4 }) {
    TextInput({ text: params.text, placeholder: params.placeholder })
      .layoutWeight(1)
      .onChange((value: string) => report(params, value))
    Button({ type: ButtonType.Circle, stateEffect: true }) {
      Text('▾').fontSize(18)
    }
    .width(36)
    .height(36)
    .enabled(params.items.length > 0)
    .bindMenu(params.items.map((item: string): MenuElement => ({
      value: item,
      action: () => {
        report(params, item);
        rebuild(params.id, item);
      }
    })))
  }
  .width(params.width > 0 ? params.width : 'auto')
  .height(params.height > 0 ? params.height : 'auto')
}

const nodes: Map<number, BuilderNode<[DayComboParams]>> = new Map();
const current: Map<number, DayComboParams> = new Map();

function copy(was: DayComboParams): DayComboParams {
  const next: DayComboParams = new DayComboParams();
  next.id = was.id;
  next.text = was.text;
  next.placeholder = was.placeholder;
  next.items = was.items;
  next.width = was.width;
  next.height = was.height;
  return next;
}

// Show `text` in the entry (a pick, or the app writing the value).
function rebuild(id: number, text: string): void {
  const node: BuilderNode<[DayComboParams]> | undefined = nodes.get(id);
  const was: DayComboParams | undefined = current.get(id);
  if (node === undefined || was === undefined) {
    return;
  }
  const next: DayComboParams = copy(was);
  next.text = text;
  current.set(id, next);
  node.update(next);
}

// Props: `<text><US><placeholder><US><item>…`.
function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  const parts: string[] = props.split(SEP);
  const params: DayComboParams = new DayComboParams();
  params.id = id;
  params.text = parts.length > 0 ? parts[0] : '';
  params.placeholder = parts.length > 1 ? parts[1] : '';
  params.items = parts.slice(2);
  const node: BuilderNode<[DayComboParams]> = new BuilderNode(ui);
  try {
    node.build(wrapBuilder<[DayComboParams]>(dayComboBuilder), params);
    const frame: FrameNode | null = node.getFrameNode();
    if (frame !== null) {
      nodes.set(id, node);
      current.set(id, params);
      return frame;
    }
  } catch (err) {
    console.error(`day-piece-combobox: component creation failed: ${err}`);
  }
  node.dispose();
  return undefined;
}

// `text` (the app's value), `items` (US-separated suggestions), `day.frame` ("<w>,<h>").
function update(id: number, cmd: string, arg: string): void {
  const node: BuilderNode<[DayComboParams]> | undefined = nodes.get(id);
  const was: DayComboParams | undefined = current.get(id);
  if (node === undefined || was === undefined) {
    return;
  }
  const next: DayComboParams = copy(was);
  if (cmd === 'text') {
    if (arg === was.text) {
      return;
    }
    next.text = arg;
  } else if (cmd === 'items') {
    next.items = arg === '' ? [] : arg.split(SEP);
  } else if (cmd === 'day.frame') {
    const size: string[] = arg.split(',');
    next.width = Number(size[0]);
    next.height = Number(size[1]);
  } else {
    return;
  }
  current.set(id, next);
  node.update(next);
}

function dispose(id: number): void {
  const node: BuilderNode<[DayComboParams]> | undefined = nodes.get(id);
  nodes.delete(id);
  current.delete(id);
  node?.dispose();
}

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.combobox',
  make: make,
  update: update,
  dispose: dispose
};
