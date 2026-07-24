//! End-to-end: a TypeScript miniapp booted through the real pipeline (store install →
//! QuickJS → TS strip → bootstrap API → page registration) and exercised via the
//! `day lite test` runner core — pieces, signals, sqlite, sandboxed fs, permission gating.

use std::path::Path;

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn stage_miniapp(dir: &Path) {
    write(
        dir,
        "manifest.json",
        r#"{ "app_id": "org.example.e2e", "name": "E2E",
             "version": { "code": 1, "name": "1.0" },
             "pages": ["home"],
             "req_permissions": [
               { "name": "day.permission.NETWORK", "reason": "test" },
               { "name": "day.permission.STORAGE", "reason": "test" }
             ],
             "day": { "files": ["app.ts", "logic.ts"] } }"#,
    );
    write(
        dir,
        "logic.ts",
        "export const double = (n: number): number => n * 2;\n",
    );
    write(
        dir,
        "app.ts",
        r#"
import { double } from "./logic.ts";

interface CounterState { label: string }
const count = signal(0);
const name = signal("world");

App({ onLaunch(_opts: object) { console.log("launched"); } });

page("home", () =>
  column(
    label(() => `Count: ${count.get()} (${double(count.get())})`).font("title"),
    button("Increment").action(() => count.update((n: number) => n + 1)),
    text_field(name).placeholder("Your name"),
    label(() => `Hello, ${name.get()}!`),
  ).spacing(12).padding(16).id("home-root"),
);
export const doubled = (n: number): number => double(n);
"#,
    );
    write(
        dir,
        "tests/app.test.ts",
        r#"
import { doubled } from "../app.ts";

test("typescript module logic runs", () => {
  expect(doubled(21)).toBe(42);
});

test("signals hold typed values", () => {
  const s = signal(5);
  s.set(6);
  expect(s.get()).toBe(6);
  const t = signal("x");
  t.update((v: string) => v + "y");
  expect(t.get()).toBe("xy");
});

test("pieces construct and chain", () => {
  const p = column(label("hi").font("body"), button("go")).spacing(4).padding(8);
  expect(typeof p.__p).toBe("number");
});

test("typed modifier after generic is a clear error", () => {
  expect(() => label("x").padding(4).font("body")).toThrow();
});

test("sqlite migrates, execs, and queries", () => {
  day.db.migrate(["create table todos (id integer primary key, title text, done integer not null default 0);"]);
  day.db.exec("insert into todos (title) values (?)", ["write tests"]);
  day.db.exec("insert into todos (title, done) values (?, 1)", ["ship"]);
  const open = day.db.query("select count(*) as n from todos where done = 0");
  expect(open[0].n).toBe(1);
  const found = day.db.query("select title from todos where title like ?", ["%test%"]);
  expect(found[0].title).toBe("write tests");
});

test("fs sandbox roundtrips and refuses escapes", () => {
  day.fs.write("notes/a.txt", "hello");
  expect(day.fs.read("notes/a.txt")).toBe("hello");
  expect(day.fs.entries("notes")[0].name).toBe("a.txt");
  expect(() => day.fs.read("../outside")).toThrow();
});

test("network is never granted in tests", () => {
  expect(day.can("NETWORK")).toBe(false);
});

test("sys info names the app", () => {
  expect(day.sys.info().appId).toBe("org.example.e2e");
});
"#,
    );
}

#[test]
fn miniapp_pipeline_end_to_end() {
    let dir = std::env::temp_dir().join(format!("day-lite-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    stage_miniapp(&dir);

    let outcomes = day_lite::run_tests(&dir).expect("runner");
    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| format!("{}: {}: {}", o.module, o.name, o.detail))
        .collect();
    assert!(failed.is_empty(), "failed tests:\n{}", failed.join("\n"));
    assert_eq!(outcomes.len(), 8, "all tests discovered");

    let _ = std::fs::remove_dir_all(&dir);
}
