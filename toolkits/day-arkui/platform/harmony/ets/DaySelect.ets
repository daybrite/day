// Built-in menu and segmented pickers. The C node API has only a TEXT_PICKER wheel;
// the host registers this ArkTS component for a Select or a compact row of native buttons.
// Props are <index><US><option>...; separate kinds preserve older menu-only host compatibility.
// Options updates keep the style and clamp selection. SelectionChanged is Day event kind 4.
// `enabled` ('0'/'1') disables the Select or every segment button.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../daypieces/DayPiece';

const SEP: string = '\u001F';
// `pieceEvent`'s kind for Day's `SelectionChanged`.
const DAY_SELECTION_CHANGED: number = 4;

class DaySelectParams {
  id: number = 0;
  segmented: boolean = false;
  options: SelectOption[] = [];
  selected: number = 0;
  // The width day-arkui laid out (its widest option), in vp; 0 until it says.
  width: number = 0;
  enabled: boolean = true;
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
  if (params.segmented) {
    Row({ space: 2 }) {
      ForEach(params.options, (option: SelectOption, index: number) => {
        Button(option.value, { type: ButtonType.Normal })
          .layoutWeight(1)
          .height(32)
          .padding({ left: 8, right: 8 })
          .fontSize(14)
          .fontColor(index === params.selected ? Color.White : $r('sys.color.ohos_id_color_text_primary'))
          .backgroundColor(index === params.selected ? '#337DFF' : Color.Transparent)
          .borderRadius(6)
          .accessibilitySelected(index === params.selected)
          .enabled(params.enabled)
          .onClick(() => {
            const live: DaySelectParams | undefined = current.get(params.id);
            if (live !== undefined) {
              live.selected = index;
            }
            nativeEntry.pieceEvent(params.id, '', index, DAY_SELECTION_CHANGED);
          })
      }, (option: SelectOption, index: number) => `${index}:${option.value}`)
    }
    .padding(4)
    .height(40)
    .width(params.width > 0 ? params.width : 'auto')
    .borderRadius(10)
    .backgroundColor($r('sys.color.ohos_id_color_sub_background'))
  } else {
    Select(params.options)
      .width(params.width > 0 ? params.width : 'auto')
      .selected(params.selected)
      .value(params.selected >= 0 ? params.options[params.selected].value : '')
      .enabled(params.enabled)
      .onSelect((index: number) => {
        // Remembered, so a later option list or re-render keeps the user's choice.
        const live: DaySelectParams | undefined = current.get(params.id);
        if (live !== undefined) {
          live.selected = index;
        }
        nativeEntry.pieceEvent(params.id, '', index, DAY_SELECTION_CHANGED);
      })
  }
}

const nodes: Map<number, BuilderNode<[DaySelectParams]>> = new Map();

function make(ui: UIContext, id: number, props: string, segmented: boolean = false): FrameNode | undefined {
  const params: DaySelectParams = parse(id, props);
  params.segmented = segmented;
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
  } else if (cmd === 'width' || cmd === 'day.frame') {
    next = new DaySelectParams();
    next.id = id;
    next.options = was.options;
    next.selected = was.selected;
    next.width = Number(arg.split(',')[0]);
  } else if (cmd === 'selected') {
    next = new DaySelectParams();
    next.id = id;
    next.options = was.options;
    const i: number = Number(arg);
    next.selected = i >= 0 && i < was.options.length ? i : was.selected;
    next.width = was.width;
  } else if (cmd === 'enabled') {
    next = new DaySelectParams();
    next.id = id;
    next.options = was.options;
    next.selected = was.selected;
    next.width = was.width;
  } else {
    return;
  }
  next.segmented = was.segmented;
  next.enabled = cmd === 'enabled' ? arg !== '0' : was.enabled;
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

function makeSegmented(ui: UIContext, id: number, props: string): FrameNode | undefined {
  return make(ui, id, props, true);
}

export const daySegmented: DayPieceModule = {
  kind: 'day.picker.segmented',
  make: makeSegmented,
  update: update,
  dispose: dispose
};
