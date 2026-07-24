// Tic-tac-toe — the whole board is ONE string signal ("---------"); every cell button and
// the status line read it reactively, so a move repaints exactly what changed. The score
// tally persists across launches through day.prefs.

const EMPTY = "---------";
const board = signal(EMPTY);
const scoreX = signal(0);
const scoreO = signal(0);

const LINES: number[][] = [
  [0, 1, 2], [3, 4, 5], [6, 7, 8],
  [0, 3, 6], [1, 4, 7], [2, 5, 8],
  [0, 4, 8], [2, 4, 6],
];

export function winnerOf(b: string): string {
  for (const [a, m, z] of LINES) {
    if (b[a] !== "-" && b[a] === b[m] && b[a] === b[z]) return b[a];
  }
  return "";
}

export function turnOf(b: string): string {
  let x = 0;
  let o = 0;
  for (const c of b) {
    if (c === "X") x += 1;
    if (c === "O") o += 1;
  }
  return x <= o ? "X" : "O";
}

export function isDraw(b: string): boolean {
  return winnerOf(b) === "" && !b.includes("-");
}

function statusText(): string {
  const b = board.get();
  const w = winnerOf(b);
  if (w !== "") return t("status-wins", { p: w });
  if (isDraw(b)) return t("status-draw");
  return t("status-turn", { p: turnOf(b) });
}

function play(i: number): void {
  const b = board.get();
  if (b[i] !== "-" || winnerOf(b) !== "") return;
  const next = b.slice(0, i) + turnOf(b) + b.slice(i + 1);
  board.set(next);
  const w = winnerOf(next);
  if (w === "X") {
    scoreX.update((n: number) => n + 1);
    day.prefs.set("scoreX", String(scoreX.get()));
  } else if (w === "O") {
    scoreO.update((n: number) => n + 1);
    day.prefs.set("scoreO", String(scoreO.get()));
  }
}

function newGame(): void {
  board.set(EMPTY);
}

function cell(i: number) {
  // `.id` BEFORE `.frame`: the id must name the button node itself (where the action
  // lives), not the frame wrapper around it — scripted taps dispatch against the id.
  return button(() => {
    const c = board.get()[i];
    return c === "-" ? " " : c;
  })
    .action(() => play(i))
    .id(`ttt-cell-${i}`)
    .frame(64, 64)
    .background("#2888889a")
    .corner_radius(8);
}

App({
  onLaunch() {
    scoreX.set(Number(day.prefs.get("scoreX") ?? 0));
    scoreO.set(Number(day.prefs.get("scoreO") ?? 0));
  },
});

page("home", () =>
  column(
    label(() => t("title")).font("large_title"),
    label(statusText).font("headline").id("ttt-status"),
    grid(
      grid_row(cell(0), cell(1), cell(2)),
      grid_row(cell(3), cell(4), cell(5)),
      grid_row(cell(6), cell(7), cell(8)),
    ).spacing(6),
    label(() => `X ${scoreX.get()} — ${scoreO.get()} O`).font("callout").id("ttt-score"),
    button(() => t("new-game")).action(newGame).id("ttt-new"),
  )
    .spacing(14)
    .align("center")
    .padding(16)
    .id("ttt-root"),
);
