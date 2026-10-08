// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-texteditor: the HarmonyOS half of the piece (docs/extending.md, docs/texteditor.md).
//
// The ArkUI C node API has no rich editor: `native_node.h` stops at ARKUI_NODE_TEXT_AREA, which
// is plain text. `RichEditor` exists only in ArkTS, so the component is declared here, built into a
// BuilderNode, and handed to Day as a FrameNode the native tree mounts like any other node. `day
// build` stages this file into the app's hvigor project through `[package.metadata.day.ohos]`.
//
// `RichEditorController` is the best-shaped controller of the eight: `updateSpanStyle` restyles a
// range without touching the characters (so the caret, the IME composition and the undo stack all
// survive a syntax highlighter), `setTypingStyle` is native, and `setSelection`/`getSelection`
// speak the same offsets the styles do.
//
// Everything crosses as strings, because that is the piece channel: `make` takes one props
// string, `update` takes a (cmd, arg) pair, and reports go back through `pieceEvent`, which reaches
// Rust as `Event::Custom`. The editor's two reports are therefore prefixed (`txt <text>` and
// `sel <start> <end>`), which is exactly what the piece's TEXT_PREFIX / SEL_PREFIX name.
//
// Offsets are ArkTS string indices, which are UTF-16 code units, so the Rust side converts with
// the same helpers the Apple arms use.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../DayPiece';

const SEP: string = '\u001F';
const RUN_SEP: string = '\u001E';

// One live editor: its BuilderNode (which owns the component), its controller, and the text it
// currently holds; the last so a change report and an offset conversion never need a round trip.
class DayEditorInstance {
  node: BuilderNode<[DayEditorParams]>;
  params: DayEditorParams;

  constructor(node: BuilderNode<[DayEditorParams]>, params: DayEditorParams) {
    this.node = node;
    this.params = params;
  }
}

class DayEditorParams {
  id: number = 0;
  text: string = '';
  placeholder: string = '';
  basePt: number = 17;
  editable: boolean = true;
  minLines: number = 3;
  // The two modes whose keys the component claims: Enter in a single line, Tab in code.
  singleLine: boolean = false;
  code: boolean = false;
  controller: RichEditorController = new RichEditorController();
  // Set while Day itself writes, so its own edits never report back as user input.
  suppress: boolean = false;
}

// The component. `onDidChange` fires after every committed edit (typing, delete, paste, IME
// commit); the text is read back off the controller rather than tracked by hand, because an IME
// commit can replace more than the range the event describes.
@Builder
function dayEditorBuilder(params: DayEditorParams) {
  RichEditor({ controller: params.controller })
    .onReady(() => {
      if (params.text !== '') {
        params.controller.addTextSpan(params.text);
      }
    })
    .onDidChange(() => {
      if (params.suppress) {
        return;
      }
      params.text = currentText(params.controller);
      nativeEntry.pieceEvent(params.id, 'txt ' + params.text);
    })
    .onSelectionChange((range: RichEditorRange) => {
      if (params.suppress) {
        return;
      }
      const start: number = range.start === undefined ? 0 : range.start;
      const end: number = range.end === undefined ? start : range.end;
      nativeEntry.pieceEvent(params.id, 'sel ' + start + ' ' + end);
    })
    // The keys the two modes claim, reported by their day names the way the other arms report
    // them (the Rust side handles `key <name>` and a dayscript key alike): Enter in a single
    // line (the keyboard's action key, and a hardware Return), Tab in code. Both consumed, so
    // nothing is inserted and focus stays.
    .enterKeyType(params.singleLine ? EnterKeyType.Done : EnterKeyType.NEW_LINE)
    .onSubmit(() => {
      if (params.singleLine) {
        nativeEntry.pieceEvent(params.id, 'key Enter');
      }
    })
    .onKeyEvent((event: KeyEvent) => {
      if (event.type !== KeyType.Down) {
        return;
      }
      if (params.singleLine && (event.keyCode === 2054 || event.keyCode === 2119)) {
        // KEYCODE_ENTER, KEYCODE_NUMPAD_ENTER
        event.stopPropagation();
        nativeEntry.pieceEvent(params.id, 'key Enter');
      } else if (params.code && event.keyCode === 2049) {
        // KEYCODE_TAB
        event.stopPropagation();
        nativeEntry.pieceEvent(params.id, 'key Tab');
      }
    })
    .enabled(params.editable)
    .placeholder(params.placeholder, { font: { size: params.basePt } })
    // Fill the frame Day's layout hands the node (the Rust arm's line band), rather than the
    // content's own height: sized to its content, the editor drew past the frame and over the
    // siblings laid out below it.
    .width('100%')
    .height('100%')
}

/// The plain text of every span, in order; RichEditor has no `getText`.
function currentText(controller: RichEditorController): string {
  const spans: Array<RichEditorImageSpanResult | RichEditorTextSpanResult> =
    controller.getSpans({ start: 0, end: -1 });
  let out: string = '';
  for (let i = 0; i < spans.length; i++) {
    // The results are interfaces, so `instanceof` is not available: the discriminator is the
    // property itself. An image span carries `valuePixelMap`/`valueResourceStr` and no `value`;
    // this editor never inserts one, but a paste could.
    const text: RichEditorTextSpanResult = spans[i] as RichEditorTextSpanResult;
    if (text.value !== undefined) {
      out += text.value;
    }
  }
  return out;
}

const instances: Map<number, DayEditorInstance> = new Map();

// `props` is `basePt<US>editable<US>spellcheck<US>minLines<US>maxLines<US>singleLine<US>code
// <US>placeholder<US>text`, with the text last so a document containing the separator still
// arrives intact (everything after the eighth separator is text).
function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  const parts: string[] = props.split(SEP);
  const params: DayEditorParams = new DayEditorParams();
  params.id = id;
  params.basePt = parts.length > 0 ? Number(parts[0]) : 17;
  params.editable = parts.length > 1 && parts[1] === '1';
  params.minLines = parts.length > 3 ? Number(parts[3]) : 3;
  params.singleLine = parts.length > 5 && parts[5] === '1';
  params.code = parts.length > 6 && parts[6] === '1';
  params.placeholder = parts.length > 7 ? parts[7] : '';
  params.text = parts.length > 8 ? parts.slice(8).join(SEP) : '';
  const node: BuilderNode<[DayEditorParams]> = new BuilderNode(ui);
  node.build(wrapBuilder<[DayEditorParams]>(dayEditorBuilder), params);
  const frame: FrameNode | null = node.getFrameNode();
  if (frame === null) {
    return undefined;
  }
  instances.set(id, new DayEditorInstance(node, params));
  return frame;
}

/// Day's ARGB number as the string ArkUI's color properties take.
function colorOf(argb: number): string {
  const hex: string = (argb >>> 0).toString(16).padStart(8, '0');
  return '#' + hex;
}

// One run: `start,end,flags,color,background,scale`. All numbers, so nothing here needs escaping.
// The flag bits are the ones the whole piece shares: 1 bold, 2 italic, 4 monospace,
// 8 strikethrough, 16 color, 32 background, 64 underline.
function applyRuns(live: DayEditorInstance, arg: string): void {
  const c: RichEditorController = live.params.controller;
  const len: number = live.params.text.length;
  live.params.suppress = true;
  // Reset first: a run that stopped being bold has to lose the attribute, and updateSpanStyle
  // only ever adds. `undefined` restores the component's own default for a property.
  c.updateSpanStyle({
    start: 0,
    end: len,
    textStyle: {
      fontSize: live.params.basePt,
      fontWeight: FontWeight.Normal,
      fontStyle: FontStyle.Normal,
      decoration: { type: TextDecorationType.None }
    }
  });
  const rows: string[] = arg === '' ? [] : arg.split(RUN_SEP);
  for (let i = 0; i < rows.length; i++) {
    const f: string[] = rows[i].split(',');
    if (f.length < 6) {
      continue;
    }
    const start: number = Number(f[0]);
    const end: number = Number(f[1]);
    const flags: number = Number(f[2]);
    const scale: number = Number(f[5]) / 1000;
    const style: RichEditorTextStyle = {
      fontSize: live.params.basePt * (scale > 0 ? scale : 1),
      fontWeight: (flags & 1) !== 0 ? FontWeight.Bold : FontWeight.Normal,
      fontStyle: (flags & 2) !== 0 ? FontStyle.Italic : FontStyle.Normal
    };
    if ((flags & 4) !== 0) {
      style.fontFamily = 'HarmonyOS Sans Mono';
    }
    if ((flags & 16) !== 0) {
      style.fontColor = colorOf(Number(f[3]));
    }
    if ((flags & 32) !== 0) {
      style.textBackgroundStyle = { color: colorOf(Number(f[4])) };
    }
    // ArkUI carries one decoration per span, so an underlined strikethrough keeps the
    // strikethrough (the rarer, more meaningful mark); recorded in docs/texteditor.md.
    if ((flags & 8) !== 0) {
      style.decoration = { type: TextDecorationType.LineThrough };
    } else if ((flags & 64) !== 0) {
      style.decoration = { type: TextDecorationType.Underline };
    }
    c.updateSpanStyle({ start: start, end: end, textStyle: style });
  }
  live.params.suppress = false;
}

// One paragraph: `start,end,align,indent,marker`.
function applyParagraphs(live: DayEditorInstance, arg: string): void {
  const c: RichEditorController = live.params.controller;
  const rows: string[] = arg === '' ? [] : arg.split(RUN_SEP);
  live.params.suppress = true;
  for (let i = 0; i < rows.length; i++) {
    const f: string[] = rows[i].split(',');
    if (f.length < 5) {
      continue;
    }
    const align: number = Number(f[2]);
    const indent: number = Number(f[3]) + Number(f[4]);
    c.updateParagraphStyle({
      start: Number(f[0]),
      end: Number(f[1]),
      style: {
        textAlign: align === 1 ? TextAlign.Center : align === 2 ? TextAlign.End
          : align === 3 ? TextAlign.JUSTIFY : TextAlign.Start,
        leadingMargin: indent
      }
    });
  }
  live.params.suppress = false;
}

// Replace the characters, keeping the caret as close to where it was as the new text allows.
function setDocumentText(live: DayEditorInstance, text: string): void {
  if (live.params.text === text) {
    return;
  }
  const c: RichEditorController = live.params.controller;
  const caret: RichEditorSelection = c.getSelection();
  const at: number = caret.selection.length > 0 ? caret.selection[0] : 0;
  live.params.suppress = true;
  c.deleteSpans({ start: 0, end: -1 });
  if (text !== '') {
    c.addTextSpan(text);
  }
  live.params.text = text;
  const to: number = Math.min(at, text.length);
  c.setSelection(to, to);
  live.params.suppress = false;
}

function update(id: number, cmd: string, arg: string): void {
  const live: DayEditorInstance | undefined = instances.get(id);
  if (live === undefined) {
    return;
  }
  if (cmd === 'text') {
    setDocumentText(live, arg);
  } else if (cmd === 'runs') {
    applyRuns(live, arg);
  } else if (cmd === 'paragraphs') {
    applyParagraphs(live, arg);
  } else if (cmd === 'selection') {
    // Not suppressed: the onSelectionChange report that follows tells the piece where the
    // caret now is, after a report the component sent before this write (an insertion's own
    // caret move) that would otherwise stand; the piece's echo guard keeps it from being
    // patched back.
    const f: string[] = arg.split(',');
    live.params.controller.setSelection(Number(f[0]), Number(f[1]));
  } else if (cmd === 'typing') {
    // Native typing style: one of only three backends that has the concept.
    const f: string[] = arg.split(',');
    const flags: number = Number(f[0]);
    const scale: number = Number(f[3]) / 1000;
    const style: RichEditorTextStyle = {
      fontSize: live.params.basePt * (scale > 0 ? scale : 1),
      fontWeight: (flags & 1) !== 0 ? FontWeight.Bold : FontWeight.Normal,
      fontStyle: (flags & 2) !== 0 ? FontStyle.Italic : FontStyle.Normal
    };
    if ((flags & 16) !== 0) {
      style.fontColor = colorOf(Number(f[1]));
    }
    if ((flags & 32) !== 0) {
      style.textBackgroundStyle = { color: colorOf(Number(f[2])) };
    }
    if ((flags & 8) !== 0) {
      style.decoration = { type: TextDecorationType.LineThrough };
    } else if ((flags & 64) !== 0) {
      style.decoration = { type: TextDecorationType.Underline };
    }
    live.params.controller.setTypingStyle(style);
  } else if (cmd === 'editable') {
    live.params.editable = arg === '1';
  } else if (cmd === 'insert') {
    // `at<US>text`: insert through the controller with the caret after it. Not suppressed, since
    // the `onDidChange` report is how the piece learns of the text.
    const sep: number = arg.indexOf(SEP);
    if (sep < 0) {
      return;
    }
    // A BYTE offset of the UTF-8 text, converted against the text held now (the Rust arm's copy
    // is as old as the last patch, and an insertion follows a keystroke reported since).
    const at: number = indexOfByte(live.params.text, Number(arg.substring(0, sep)));
    const text: string = arg.substring(sep + 1);
    live.params.controller.addTextSpan(text, { offset: at });
    live.params.controller.setSelection(at + text.length, at + text.length);
  }
}

/// The string index at a UTF-8 byte offset, clamped to the string.
function indexOfByte(s: string, atByte: number): number {
  let bytes: number = 0;
  let i: number = 0;
  while (i < s.length && bytes < atByte) {
    const cp: number = s.codePointAt(i) ?? 0;
    bytes += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    i += cp >= 0x10000 ? 2 : 1;
  }
  return i;
}

function dispose(id: number): void {
  const live: DayEditorInstance | undefined = instances.get(id);
  if (live !== undefined) {
    instances.delete(id);
    live.node.dispose();
  }
}

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.texteditor',
  make: make,
  update: update,
  dispose: dispose
};
