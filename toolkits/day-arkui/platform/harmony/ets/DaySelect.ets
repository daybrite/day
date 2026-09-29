// The menu-style option picker (docs/picker.md): HarmonyOS's own `Select`, the dropdown button
// whose options open in a menu. The ArkUI C node API has no select node kind (native_node.h stops
// at TEXT_PICKER, the wheel), so this is a built-in ArkTS piece: the host passes it to
// `registerDayPieces` beside the app's own, and day-arkui realizes `PickerStyle::Menu` through it
// (falling back to the wheel when a host is too old to register it).
//
// Props and commands carry the selected index first, then the option labels, 0x1F-separated
// (`<selected><US><opt>…`): the same encoding for `make` and for the `options` command. A choice
// goes back through `pieceEvent` as the picker's own `SelectionChanged` (kind 4, the index).

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../daypieces/DayPiece';

const SEP: string = '\u001F';
// `pieceEvent`'s kind for Day's `SelectionChanged`.
const DAY_SELECTION_CHANGED: number = 4;

class DaySelectParams {
  id: number = 0;
  options: SelectOption[] = [];
  selected: number = 0;
  // The width day-arkui laid out (its widest option), in vp; 0 until it says.
  width: number = 0;
}

// The live parameters each Select was last built with, by day node id.
const current: Map<number, DaySelectParams> = new Map();

// `keep` is the selection an empty index field stands for (a new option list keeps the choice).
function parse(id: number, encoded: string, keep: number = 0): DaySelectParams {
  const parts: string[] = encoded.split(SEP);
  const params: DaySelectParams = new DaySelectParams();
  params.id = id;
  params.selected = parts[0] === '' ? keep : Number(parts[0]);
  params.options = parts.slice(1).map((value: string): SelectOption => ({ value: value }));
  if (params.selected >= params.options.length) {
    params.selected = params.options.length - 1;
  }
  if (!(params.selected >= 0)) {
    params.selected = params.options.length > 0 ? 0 : -1;
  }
  return params;
}

@Builder
function daySelectBuilder(params: DaySelectParams) {
  // No percentage sizing (a BuilderNode is built detached, so '100%' would resolve against the
  // window): the button starts at its intrinsic size, which day-arkui measures, and then takes
  // the width of its widest option, so a choice never changes it.
  Select(params.options)
    .width(params.width > 0 ? params.width : 'auto')
    .selected(params.selected)
    .value(params.selected >= 0 ? params.options[params.selected].value : '')
    .onSelect((index: number) => {
      // Remembered, so a later option list or re-render keeps the user's choice.
      const live: DaySelectParams | undefined = current.get(params.id);
      if (live !== undefined) {
        live.selected = index;
      }
      nativeEntry.pieceEvent(params.id, '', index, DAY_SELECTION_CHANGED);
    })
}

const nodes: Map<number, BuilderNode<[DaySelectParams]>> = new Map();

function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  const params: DaySelectParams = parse(id, props);
  const node: BuilderNode<[DaySelectParams]> = new BuilderNode(ui);
  try {
    node.build(wrapBuilder<[DaySelectParams]>(daySelectBuilder), params);
    const frame: FrameNode | null = node.getFrameNode();
    if (frame !== null) {
      nodes.set(id, node);
      current.set(id, params);
      return frame;
    }
  } catch (err) {
    console.error(`day select: component creation failed: ${err}`);
  }
  node.dispose();
  return undefined;
}

function update(id: number, cmd: string, arg: string): void {
  const node: BuilderNode<[DaySelectParams]> | undefined = nodes.get(id);
  const was: DaySelectParams | undefined = current.get(id);
  if (node === undefined || was === undefined) {
    return;
  }
  let next: DaySelectParams;
  if (cmd === 'options') {
    next = parse(id, arg, was.selected);
    next.width = was.width;
  } else if (cmd === 'width') {
    next = new DaySelectParams();
    next.id = id;
    next.options = was.options;
    next.selected = was.selected;
    next.width = Number(arg);
  } else if (cmd === 'selected') {
    next = new DaySelectParams();
    next.id = id;
    next.options = was.options;
    const i: number = Number(arg);
    next.selected = i >= 0 && i < was.options.length ? i : was.selected;
    next.width = was.width;
  } else {
    return;
  }
  current.set(id, next);
  node.update(next);
}

function dispose(id: number): void {
  const node: BuilderNode<[DaySelectParams]> | undefined = nodes.get(id);
  nodes.delete(id);
  current.delete(id);
  node?.dispose();
}

export const daySelect: DayPieceModule = {
  kind: 'day.picker.menu',
  make: make,
  update: update,
  dispose: dispose
};
