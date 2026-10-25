// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Native modal hosting with ArkUI TextInput/Button content. Request IDs, not a global
// "current dialog", own answers and dismissal. Concurrent requests are presented in order.
import { ComponentContent, UIContext, window } from '@kit.ArkUI';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';

interface DialogButton {
  label: string;
  role: number; // 0 default, 1 cancel, 2 destructive
}

interface DialogSpec {
  title: string;
  message: string;
  sheet: boolean;
  prompt: boolean;
  initial: string;
  placeholder: string;
  buttons: DialogButton[];
}

class Answer {
  kind: number;
  index: number;
  text: string;
  constructor(kind: number = 0, index: number = 0, text: string = '') {
    this.kind = kind;
    this.index = index;
    this.text = text;
  }
}

class Request {
  req: number;
  spec: DialogSpec;
  finish: (answer: Answer) => void;
  text: string;
  content?: ComponentContent<Request>;
  opened: boolean = false;
  closing: boolean = false;
  completed: boolean = false;
  answer?: Answer;
  constructor(req: number, spec: DialogSpec, finish: (answer: Answer) => void) {
    this.req = req;
    this.spec = spec;
    this.finish = finish;
    this.text = spec.initial;
  }

  choose(index: number): void {
    if (this.spec.prompt) {
      this.finish(this.spec.buttons[index].role === 1 ? new Answer() : new Answer(2, 0, this.text));
    } else {
      this.finish(new Answer(1, index));
    }
  }
}

@Builder
function dialogButton(request: Request, button: DialogButton, index: number, horizontal: boolean) {
  Button(button.label).width(horizontal ? 'auto' : '100%')
    .layoutWeight(horizontal ? 1 : 0).constraintSize({ minHeight: 40 })
    .id('day.dialog.button.' + index)
    .fontColor(button.role === 2 ? $r('sys.color.ohos_id_color_warning')
      : $r('sys.color.ohos_id_color_text_primary_activated'))
    .backgroundColor(Color.Transparent)
    .onClick(() => { request.choose(index); })
}

@Builder
function dialogContent(request: Request) {
  Scroll() {
    Column({ space: 16 }) {
      Text(request.spec.title)
        .fontSize(20).fontWeight(FontWeight.Medium)
        .fontColor($r('sys.color.ohos_id_color_text_primary'))
        .width('100%').accessibilityLevel('yes')
      if (request.spec.message.length > 0) {
        Text(request.spec.message).fontSize(16)
          .fontColor($r('sys.color.ohos_id_color_text_secondary')).width('100%')
      }
      if (request.spec.prompt) {
        TextInput({ text: request.text, placeholder: request.spec.placeholder })
          .id('day.dialog.input').accessibilityText(request.spec.placeholder || request.spec.title)
          .enterKeyType(EnterKeyType.Done).defaultFocus(true)
          .onChange((value: string) => { request.text = value; })
          .onSubmit(() => { request.choose(1); })
      }
      if (!request.spec.sheet && request.spec.buttons.length <= 2) {
        Row({ space: 8 }) {
          ForEach(request.spec.buttons, (button: DialogButton, index: number) => {
            dialogButton(request, button, index, true)
          }, (_button: DialogButton, index: number) => index.toString())
        }.width('100%')
      } else {
        ForEach(request.spec.buttons, (button: DialogButton, index: number) => {
          dialogButton(request, button, index, false)
        }, (_button: DialogButton, index: number) => index.toString())
      }
    }.padding(24).width('100%')
  }.width('90%').constraintSize({ maxWidth: 400, maxHeight: '85%' })
    .backgroundColor($r('sys.color.ohos_id_color_dialog_bg')).borderRadius(24)
    .clip(true)
    .scrollBar(BarState.Auto).id('day.dialog.surface')
}

export class DayDialogs {
  private ui: UIContext;
  private requests: Map<number, Request> = new Map();
  private queue: Request[] = [];
  private active?: Request;
  private disposed: boolean = false;

  constructor(ui: UIContext) {
    this.ui = ui;
    nativeEntry.registerDialogs(
      (req: number, encoded: string) => { this.present(req, encoded); },
      (req: number) => {
        const request = this.requests.get(req);
        if (request !== undefined) this.finish(request, new Answer());
      });
  }

  private present(req: number, encoded: string): void {
    if (this.disposed) {
      nativeEntry.onDialogResult(req, 0, 0, '');
      return;
    }
    try {
      const spec = JSON.parse(encoded) as DialogSpec;
      const request = new Request(req, spec, (answer: Answer) => {
        this.finish(request, answer);
      });
      this.requests.set(req, request);
      this.queue.push(request);
      this.advance();
    } catch (err) {
      console.error(`day: dialog request failed: ${(err as BusinessError).message}`);
      nativeEntry.onDialogResult(req, 0, 0, '');
    }
  }

  private advance(): void {
    if (this.disposed || this.active !== undefined) return;
    const request = this.queue.shift();
    if (request === undefined) return;
    this.active = request;
    try {
      const content = new ComponentContent(this.ui, wrapBuilder(dialogContent), request);
      request.content = content;
      this.ui.getPromptAction().openCustomDialog(content, {
        alignment: request.spec.sheet ? DialogAlignment.Bottom : DialogAlignment.Center,
        offset: { dx: 0, dy: request.spec.sheet ? -this.bottomInset() - 16 : 0 },
        autoCancel: true,
        onWillDismiss: () => { this.finish(request, new Answer()); },
        onDidDisappear: () => { this.complete(request); }
      }).then(() => {
        request.opened = true;
        // Dismissal can arrive before ArkUI has finished opening the content.
        if (request.answer !== undefined) this.close(request);
      }).catch((err: BusinessError) => {
        console.error(`day: opening dialog failed: ${err.code} ${err.message}`);
        this.complete(request);
      });
    } catch (err) {
      console.error(`day: building dialog failed: ${(err as BusinessError).message}`);
      this.complete(request);
    }
  }

  private bottomInset(): number {
    const name = this.ui.getWindowName();
    if (name === undefined) return 0;
    const owner = window.findWindow(name);
    // PromptAction's Bottom alignment uses the full window, including navigation bars.
    // ArkUI's avoidance rectangles are px; dialog offsets are vp.
    return this.ui.px2vp(Math.max(
      owner.getWindowAvoidArea(window.AvoidAreaType.TYPE_SYSTEM).bottomRect.height,
      owner.getWindowAvoidArea(window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR).bottomRect.height));
  }

  private finish(request: Request, answer: Answer): void {
    if (request.completed || request.answer !== undefined) return;
    request.answer = answer;
    if (this.active !== request) {
      this.queue = this.queue.filter((item: Request) => item !== request);
      this.complete(request);
    } else if (request.opened) {
      this.close(request);
    }
  }

  private close(request: Request): void {
    if (request.closing || request.completed || request.content === undefined) return;
    request.closing = true;
    try {
      this.ui.getPromptAction().closeCustomDialog(request.content).catch((err: BusinessError) => {
        console.error(`day: closing dialog failed: ${err.code} ${err.message}`);
        this.complete(request);
      });
    } catch (err) {
      console.error(`day: closing dialog failed: ${(err as BusinessError).message}`);
      this.complete(request);
    }
    // onDidDisappear owns successful completion, after the native close animation.
  }

  private complete(request: Request): void {
    if (request.completed) return;
    request.completed = true;
    this.requests.delete(request.req);
    const answer = request.answer ?? new Answer();
    // Let ArkUI finish its disappearance callback before disposing the BuilderNode or
    // presenting a continuation's next dialog. The request stays alive in this closure.
    setTimeout(() => {
      request.content?.dispose();
      request.content = undefined;
      if (this.active === request) this.active = undefined;
      nativeEntry.onDialogResult(request.req, answer.kind, answer.index, answer.text);
      this.advance();
    }, 0);
  }

  dispose(): void {
    this.disposed = true;
    this.requests.forEach((request: Request) => { this.finish(request, new Answer()); });
  }
}
