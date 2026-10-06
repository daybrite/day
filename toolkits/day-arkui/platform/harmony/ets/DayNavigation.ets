// Each Day navigation surface owns its native Navigation, path and content slots.
// Tabs are ownership boundaries: a sibling's title, search or Back must never escape here.
import { BuilderNode, FrameNode, NodeContent, UIContext } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../daypieces/DayPiece';
import { DayToolbar, DayToolbarItem } from './DayToolbar';

@Observed
class NavigationModel {
  id: number = 0;
  alive: boolean = true;
  width: number = 0;
  height: number = 0;
  rootKey: number = 0;
  rootContent: NodeContent = new NodeContent();
  rootTitle: string = '';
  stack: NavPathStack = new NavPathStack();
  contents: Map<number, NodeContent> = new Map();
  titles: Map<number, string> = new Map();
  guarded: boolean = false;
  menus: DayToolbarItem[] = [];
  rootMenus: DayToolbarItem[] = [];
  searchShown: boolean = false;
  searchPrompt: string = '';
  searchText: string = '';
  // An adaptive host (`NavStyle::Sidebar` left to follow the window) runs NavigationMode.Auto:
  // ArkUI tiles the nav bar beside the content where both fit and stacks them where they do
  // not, and reports which through onNavigationModeChange (docs/size-classes.md). A literal
  // stack host (a nested nav_stack) stays NavigationMode.Stack at every width.
  adaptive: boolean = false;
  // The mode ArkUI last reported: true while the nav bar and the content are side by side.
  split: boolean = false;
}
const models: Map<number, NavigationModel> = new Map();
const nodes: Map<number, BuilderNode<[NavigationModel]>> = new Map();
let pageMenus: DayToolbarItem[] = [];
let rootMenus: DayToolbarItem[] = [];
let windowToolbar: (items: DayToolbarItem[]) => void = (_items: DayToolbarItem[]): void => {};
function refreshWindowToolbar(): void {
  // Apps with no navigation surface still have window-level toolbar commands.
  // A navigation host places them inside its own native title bar instead.
  windowToolbar(models.size === 0 ? rootMenus : []);
}
function animated(): boolean { return AppStorage.get<boolean>('dayDriven') !== true; }

@Component
struct DayNavigation {
  @ObjectLink model: NavigationModel;
  @Builder
  private pageToolbar() { DayToolbar({ items: this.model.menus }) }
  @Builder
  private rootToolbar() { DayToolbar({ items: this.model.rootMenus }) }
  @Builder
  private destination(name: string, param: object) {
    NavDestination() {
      Stack() { ContentSlot(this.model.contents.get(Number(param))) }
        .width('100%').height('100%').clip(true)
        .onAreaChange((_old: Area, area: Area) => {
          if (this.model.alive) nativeEntry.navPageArea(Number(param), Number(area.width), Number(area.height));
        })
    }
    .title(this.model.titles.get(Number(param)) ?? '')
    // Tiled, the first destination sits beside the rows that opened it: a back button there
    // would only deselect the row (the up-arrow Android's tiled host hides for the same reason).
    .hideBackButton(this.model.split && this.model.stack.getAllPathName()[0] === name)
    .menus(this.pageToolbar)
    .onBackPressed(() => {
      if (nativeEntry.coverBackRequested()) return true;
      if (this.model.guarded) {
        nativeEntry.navBackRequested(this.model.id);
        return true;
      }
      // Tiled, the first destination has no back: popping it would leave the detail pane
      // empty beside the rows, the one state a split has no way to draw (docs/navigation.md).
      if (this.model.split && this.model.stack.getAllPathName()[0] === name) return true;
      return false;
    })
    .onDisAppear(() => {
      // A hidden tab or a covered destination is not a pop. Only a removed path
      // acknowledges navigation; disposing the whole host must not pop another one.
      if (this.model.alive && this.model.stack.getAllPathName().indexOf(name) < 0) {
        this.model.contents.delete(Number(param));
        this.model.titles.delete(Number(param));
        nativeEntry.navPopped(this.model.id, Number(param));
      }
    })
  }
  build() {
    Navigation(this.model.stack) {
      Column() {
        if (this.model.searchShown) {
          Search({ value: this.model.searchText, placeholder: this.model.searchPrompt })
            .id('day.navigation.search.' + this.model.id)
            .margin({ left: 16, right: 16, top: 8, bottom: 8 })
            .onChange((value: string) => {
              if (value !== this.model.searchText) {
                this.model.searchText = value;
                nativeEntry.navSearchChanged(this.model.id, value);
              }
            })
        }
        Stack() { ContentSlot(this.model.rootContent) }
          .width('100%').layoutWeight(1).clip(true)
          .onAreaChange((_old: Area, area: Area) => {
            if (this.model.alive && this.model.rootKey !== 0)
              nativeEntry.navPageArea(this.model.rootKey, Number(area.width), Number(area.height));
          })
      }.width('100%').height('100%')
    }
    .title(this.model.rootTitle)
    .hideTitleBar(this.model.rootTitle.length === 0 && this.model.rootMenus.length === 0)
    .titleMode(NavigationTitleMode.Mini).hideBackButton(true)
    .menus(this.rootToolbar).navDestination(this.destination)
    .mode(this.model.adaptive ? NavigationMode.Auto : NavigationMode.Stack)
    // A sidebar's width beside the detail; Auto tiles once navBarWidth + minContentWidth fit.
    .navBarWidth(320).minContentWidth(360)
    .onNavigationModeChange((mode: NavigationMode) => {
      const split = mode === NavigationMode.Split;
      if (this.model.adaptive && split !== this.model.split) {
        this.model.split = split;
        nativeEntry.navModeChanged(this.model.id, split ? 1 : 0);
      }
    })
    .backgroundColor($r('sys.color.ohos_id_color_sub_background'))
    .width(this.model.width).height(this.model.height)
  }
}
@Builder
function navigationBuilder(model: NavigationModel) { DayNavigation({ model: model }) }

export function registerDayNavigation(ui: UIContext, toolbar: (items: DayToolbarItem[]) => void): void {
  windowToolbar = toolbar;
  nativeEntry.registerNav(
    (owner: number, key: number, title: string, pushed: boolean): NodeContent => {
      const model = models.get(owner);
      if (model === undefined) throw new Error('Missing Day navigation host ' + owner);
      if (!pushed) {
        model.rootKey = key;
        return model.rootContent;
      }
      const content = new NodeContent();
      model.contents.set(key, content);
      model.titles = new Map(model.titles).set(key, title);
      model.stack.pushPathByName('day-page-' + key, key, animated());
      return content;
    },
    (owner: number): void => { models.get(owner)?.stack.pop(animated()); },
    (owner: number, title: string): void => {
      const model = models.get(owner);
      if (model === undefined) return;
      const count = model.stack.size();
      if (count === 0) model.rootTitle = title;
      else model.titles = new Map(model.titles).set(Number(model.stack.getParamByIndex(count - 1)), title);
    },
    (owner: number, on: boolean): void => {
      const model = models.get(owner);
      if (model !== undefined) model.guarded = on;
    },
    (icons: string, labels: string, actions: string, scopes: string, enabled: string): void => {
      const ic = icons.length === 0 ? [] : icons.split('\n');
      const lb = labels.length === 0 ? [] : labels.split('\n');
      const ac = actions.length === 0 ? [] : actions.split('\n');
      const sc = scopes.length === 0 ? [] : scopes.split('\n');
      const en = enabled.length === 0 ? [] : enabled.split('\n');
      const page: DayToolbarItem[] = [];
      const root: DayToolbarItem[] = [];
      for (let i = 0; i < ac.length; i++) {
        const address = ac[i].split(':');
        const id = Number(address[0]);
        if (id === 0) continue;
        const titles = i < lb.length ? lb[i].split('\u001f') : [''];
        const item: DayToolbarItem = {
          value: titles[0], choices: titles.slice(1),
          selection: address.length > 1 ? Number(address[1]) : -1,
          choose: (index: number) => { nativeEntry.navMenuAction(id, index); },
          icon: i < ic.length && ic[i].length > 0 ? ic[i] : undefined,
          isEnabled: !(i < en.length && (en[i] === '0' || en[i] === '2')),
          checked: i < en.length && (en[i] === '2' || en[i] === '3'),
          action: () => { nativeEntry.navMenuAction(id); }
        };
        const scope = i < sc.length ? sc[i] : '0';
        if (scope !== '0') root.push(item);
        if (scope !== '1') page.push(item);
      }
      pageMenus = page; rootMenus = root;
      refreshWindowToolbar();
      models.forEach((model: NavigationModel) => { model.menus = page; model.rootMenus = root; });
    },
    (owner: number, shown: number, prompt: string, text: string): void => {
      const model = models.get(owner);
      if (model === undefined) return;
      if (shown === -2) {
        if (model.searchShown) ui.getFocusController().requestFocus('day.navigation.search.' + owner);
        return;
      }
      if (shown >= 0) { model.searchShown = shown === 1; model.searchPrompt = prompt; }
      model.searchText = text;
    }
  );
}
export const dayNavigation: DayPieceModule = {
  kind: 'day.navigation.stack',
  make: (ui: UIContext, id: number, title: string): FrameNode | undefined => {
    const model = new NavigationModel();
    model.id = id; model.rootTitle = title; model.menus = pageMenus; model.rootMenus = rootMenus;
    const node = new BuilderNode<[NavigationModel]>(ui);
    models.set(id, model);
    refreshWindowToolbar();
    node.build(wrapBuilder<[NavigationModel]>(navigationBuilder), model);
    const frame = node.getFrameNode();
    if (frame !== null) { nodes.set(id, node); return frame; }
    model.alive = false; models.delete(id); node.dispose(); refreshWindowToolbar();
    return undefined;
  },
  update: (id: number, command: string, arg: string): void => {
    const model = models.get(id);
    if (model !== undefined && command === 'day.frame') {
      const size = arg.split(','); model.width = Number(size[0]); model.height = Number(size[1]);
    }
    if (model !== undefined && command === 'day.adaptive') model.adaptive = arg === '1';
  },
  dispose: (id: number): void => {
    const model = models.get(id);
    if (model !== undefined) model.alive = false;
    models.delete(id); nodes.get(id)?.dispose(); nodes.delete(id); refreshWindowToolbar();
  }
};
