// Rehype plugin — give platform-target names their accent color in docs prose.
//
// Wherever markdown writes a target name as inline code (`macos-appkit`, `web-dom`, …), stamp
// the <code> element with data-pf="<target>"; global.css resolves that to the platform's
// accent (the iOS-palette tokens, light/dark aware). Content is never edited — a name that
// stops matching simply renders as ordinary code again. Code BLOCKS are left alone: the
// accent is a prose affordance, not syntax highlighting.

const TARGETS = new Set([
  'macos-appkit',
  'ios-uikit',
  'android-mdc',
  'linux-gtk',
  'linux-qt',
  'windows-xaml',
  'harmony-arkui',
  'web-dom',
  // Secondary desktop combos take their toolkit family's color (see global.css).
  'macos-gtk',
  'macos-qt',
  'windows-gtk',
  'windows-qt',
]);

export default function accentTargetCode() {
  return (tree) => {
    const visit = (node, insidePre) => {
      if (node.type === 'element') {
        if (node.tagName === 'pre') insidePre = true;
        if (node.tagName === 'code' && !insidePre) {
          const only = node.children?.length === 1 ? node.children[0] : null;
          if (only?.type === 'text' && TARGETS.has(only.value)) {
            (node.properties ??= {})['data-pf'] = only.value;
          }
        }
      }
      if (Array.isArray(node.children)) {
        for (const child of node.children) visit(child, insidePre);
      }
    };
    visit(tree, false);
  };
}
