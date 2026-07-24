import { visibleTodos, openCount } from "../app.ts";

test("starts empty", () => {
  expect(visibleTodos().length).toBe(0);
  expect(openCount()).toBe(0);
});

test("inserted rows appear, search filters by title", () => {
  day.db.exec("insert into todos (title) values (?)", ["buy milk"]);
  day.db.exec("insert into todos (title, done) values (?, 1)", ["ship day-lite"]);
  expect(visibleTodos().length).toBe(2);
  expect(openCount()).toBe(1);
  const hits = day.db.query("select title from todos where title like ?", ["%milk%"]);
  expect(hits.length).toBe(1);
  expect(hits[0].title).toBe("buy milk");
});

test("open items sort before done items", () => {
  const rows = visibleTodos();
  expect(rows[0].done).toBe(0);
  expect(rows[rows.length - 1].done).toBe(1);
});
