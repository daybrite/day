import { LengthMetrics } from '@kit.ArkUI';

// Native Navigation menu content with template-image tinting. NavigationMenuItem.icon
// loads SVGs literally; it does not apply the system foreground in dark mode.
export interface DayToolbarItem extends NavigationMenuItem {
  value: string;
  checked: boolean;
  choices: string[];
  selection: number;
  choose: (index: number) => void;
}

@Component
export struct DayToolbar {
  @Prop items: DayToolbarItem[] = [];

  private visible(): DayToolbarItem[] {
    // Text-only commands belong in the popup, where their full accessible title fits.
    const icons = this.items.filter((item: DayToolbarItem) => item.icon !== undefined);
    return icons.slice(0, this.items.length > 3 || icons.length < this.items.length ? 2 : 3);
  }

  private overflow(): DayToolbarItem[] {
    const shown = this.visible();
    return this.items.filter((item: DayToolbarItem) => !shown.includes(item));
  }

  @Builder
  private popup() {
    Menu() {
      ForEach(this.overflow(), (item: DayToolbarItem) => {
        if (item.choices.length > 0) {
          MenuItem({ content: item.value, builder: () => { this.choicesMenu(item); } })
            .enabled(item.isEnabled !== false)
        } else {
          MenuItem({ content: item.value })
            .selected(item.checked)
            .selectIcon(true)
            .enabled(item.isEnabled !== false)
            .onClick(() => { item.action?.(); })
        }
      })
    }
  }

  @Builder
  private choicesMenu(item: DayToolbarItem) {
    Menu() {
      ForEach(item.choices, (title: string, index: number) => {
        MenuItem({ content: title })
          .selected(index === item.selection)
          .selectIcon(true)
          .enabled(item.isEnabled !== false)
          .onClick(() => { item.choose(index); })
      })
    }
  }

  @Builder
  private glyph(item: DayToolbarItem) {
    Image(item.icon)
      .width(24).height(24)
      .fillColor(item.checked ? Color.White :
        $r('sys.color.ohos_id_color_primary'))
  }

  build() {
    Row({ space: 8 }) {
      ForEach(this.visible(), (item: DayToolbarItem) => {
        if (item.choices.length > 0) {
          Button({ type: ButtonType.Circle }) { this.glyph(item) }
            .width(40).height(40)
            .backgroundColor(item.checked ? $r('sys.color.ohos_id_color_emphasize') :
              $r('sys.color.ohos_id_color_button_normal'))
            .enabled(item.isEnabled !== false)
            .opacity(item.isEnabled === false ? 0.4 : 1)
            .accessibilityText(item.value)
            .bindMenu(() => { this.choicesMenu(item); })
        } else {
          Button({ type: ButtonType.Circle }) { this.glyph(item) }
            .width(40).height(40)
            .backgroundColor(item.checked ? $r('sys.color.ohos_id_color_emphasize') :
              $r('sys.color.ohos_id_color_button_normal'))
            .enabled(item.isEnabled !== false)
            .opacity(item.isEnabled === false ? 0.4 : 1)
            .accessibilityText(item.value)
            .accessibilitySelected(item.checked)
            .onClick(() => { item.action?.(); })
        }
      })
      if (this.overflow().length > 0) {
        Button({ type: ButtonType.Circle }) {
          Text('⋯').fontSize(24).fontWeight(FontWeight.Bold)
            .fontColor($r('sys.color.ohos_id_color_primary'))
        }
        .width(40).height(40)
        .backgroundColor($r('sys.color.ohos_id_color_button_normal'))
        .accessibilityText($r('sys.string.ohos_toolbar_more'))
        .bindMenu(this.popup)
      }
    }
    .height(56)
    .padding({ end: LengthMetrics.vp(16) })
  }
}
