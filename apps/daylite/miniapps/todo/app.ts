// Todos — sqlite persistence + search (docs/lite.md). The rows are an `each` over a
// reactive query: reading `rev` and `search` inside the items closure registers real
// day-reactive dependencies, so writes re-run the query and re-key the rows.

export interface Todo {
  id: number;
  title: string;
  done: number;
}

day.db.migrate([
  "create table todos (id integer primary key, title text not null, done integer not null default 0);",
]);

const newTitle = signal("");
const search = signal("");
const rev = signal(0); // bumped after every write so queries re-run
const bump = () => rev.update((n: number) => n + 1);

export function visibleTodos(): Todo[] {
  rev.get();
  const q = search.get().trim();
  return q.length > 0
    ? (day.db.query(
        "select id, title, done from todos where title like ? order by done, id desc",
        ["%" + q + "%"],
      ) as Todo[])
    : (day.db.query("select id, title, done from todos order by done, id desc") as Todo[]);
}

export function openCount(): number {
  rev.get();
  return day.db.query("select count(*) as n from todos where done = 0")[0].n as number;
}

function addTodo(): void {
  const t = newTitle.get().trim();
  if (t.length === 0) return;
  day.db.exec("insert into todos (title) values (?)", [t]);
  newTitle.set("");
  bump();
}

function toggleTodo(t: Todo): void {
  day.db.exec("update todos set done = ? where id = ?", [t.done ? 0 : 1, t.id]);
  bump();
}

function clearDone(): void {
  day.db.exec("delete from todos where done = 1");
  bump();
}

App({});

page("home", () =>
  column(
    label(() => t("title")).font("large_title"),
    label(() => t("open-count", { n: openCount() })).font("footnote"),
    row(
      text_field(newTitle).placeholder(t("add-placeholder")).on_submit(addTodo).id("todo-new"),
      button(() => t("add")).action(addTodo).id("todo-add"),
    ).spacing(8),
    text_field(search).placeholder(t("search-placeholder")).id("todo-search"),
    each(visibleTodos, (t: Todo) =>
      row(
        button(t.done ? "☑" : "☐").action(() => toggleTodo(t)),
        label(t.title).id("todo-item-" + t.id).grow_w(),
      )
        .spacing(10)
        .padding(6),
    ),
    when(
      () => {
        rev.get();
        return day.db.query("select count(*) as n from todos where done = 1")[0].n > 0;
      },
      () => button(() => t("clear-completed")).action(clearDone).id("todo-clear"),
    ),
  )
    .spacing(10)
    .padding(16)
    .id("todo-root"),
);
