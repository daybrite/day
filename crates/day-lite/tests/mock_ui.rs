//! Mount a miniapp's UI under the mock toolkit: the page builder crosses JS → dyn registry
//! → real pieces, and the probe asserts the tree actually materialized (the layer
//! `day lite test` deliberately doesn't exercise).

use std::cell::RefCell;
use std::path::Path;

use day_lite::{Host, LiteApp, Store};
use day_mock::MockToolkit;
use day_spec::{Size, WindowOptions};

thread_local! {
    static APP: RefCell<Option<LiteApp>> = const { RefCell::new(None) };
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn miniapp_page_mounts_real_pieces() {
    let dir = std::env::temp_dir().join(format!("day-lite-mockui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir,
        "manifest.json",
        r#"{ "app_id": "org.example.ui", "name": "UI", "version": { "code": 1, "name": "1" },
             "pages": ["home"],
             "req_permissions": [{ "name": "day.permission.STORAGE", "reason": "t" }],
             "day": { "files": ["app.ts"] } }"#,
    );
    write(
        &dir,
        "app.ts",
        r#"
day.db.migrate(["create table items (id integer primary key, title text);"]);
day.db.exec("insert into items (title) values (?)", ["first item"]);
const rev = signal(0);
const draft = signal("");
function items(): object[] {
  rev.get();
  return day.db.query("select id, title from items order by id");
}
App({});
page("home", () =>
  column(
    label("Items").font("title"),
    text_field(draft).placeholder("Add").id("ui-draft"),
    each(items, (it: { id: number; title: string }) =>
      label(it.title).id("ui-item-" + it.id),
    ),
    when(
      () => { rev.get(); return items().length > 0; },
      () => label("has items").id("ui-nonempty"),
    ),
  ).spacing(8).padding(10).id("ui-root"),
);
"#,
    );

    let store = Store::at(dir.join("store"));
    let plan = store.install(dir.to_str().unwrap()).expect("plan");
    let granted: Vec<String> = plan
        .manifest
        .req_permissions
        .iter()
        .map(|p| p.name.clone())
        .collect();
    plan.confirm(&granted).expect("confirm");

    let host = Host::builder().store(store).build();
    let app = host.launch("org.example.ui").expect("launch");
    APP.with(|a| *a.borrow_mut() = Some(app));

    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        || APP.with(|a| a.borrow().as_ref().expect("running").surface()),
    );
    day_reactive::flush_sync();

    let labels: Vec<String> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.text.clone())
        .collect();
    assert!(
        labels.iter().any(|t| t == "Items"),
        "page title mounted: {labels:?}"
    );
    assert!(
        labels.iter().any(|t| t == "first item"),
        "each() row mounted: {labels:?}"
    );
    assert!(
        labels.iter().any(|t| t == "has items"),
        "when() arm mounted: {labels:?}"
    );

    APP.with(|a| a.borrow_mut().take());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The device-observed sequence: app A's page builder THROWS (modifier misorder), the app
/// closes, then app B boots — B's page must still mount cleanly.
#[test]
fn second_app_mounts_after_first_apps_builder_threw() {
    let dir = std::env::temp_dir().join(format!("day-lite-mockui2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let bad = dir.join("bad");
    write(
        &bad,
        "manifest.json",
        r#"{ "app_id": "org.example.bad", "name": "Bad", "version": { "code": 1, "name": "1" },
             "pages": ["home"], "day": { "files": ["app.ts"] } }"#,
    );
    // `.align` after `.padding` — the LateTyped error, thrown during the page build.
    write(
        &bad,
        "app.ts",
        r#"App({}); page("home", () => column(label("x")).padding(4).align("center"));"#,
    );
    let good = dir.join("good");
    write(
        &good,
        "manifest.json",
        r#"{ "app_id": "org.example.good", "name": "Good", "version": { "code": 1, "name": "1" },
             "pages": ["home"], "day": { "files": ["app.ts"] } }"#,
    );
    write(
        &good,
        "app.ts",
        r#"App({}); page("home", () => column(label("good page")).spacing(4).id("good-root"));"#,
    );

    let store = Store::at(dir.join("store"));
    for d in [&bad, &good] {
        store
            .install(d.to_str().unwrap())
            .expect("plan")
            .confirm(&[])
            .expect("confirm");
    }
    let host = Host::builder().store(store).build();

    // App A: boot, mount (builder throws inside), close.
    let app = host.launch("org.example.bad").expect("launch bad");
    APP.with(|a| *a.borrow_mut() = Some(app));
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        || APP.with(|a| a.borrow().as_ref().expect("running").surface()),
    );
    day_reactive::flush_sync();
    let labels: Vec<String> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.text.clone())
        .collect();
    assert!(
        labels.iter().any(|t| t.contains("returned no piece")),
        "bad app degrades visibly: {labels:?}"
    );
    APP.with(|a| a.borrow_mut().take()); // close app A

    // App B: boot and mount — must be unaffected by A's failure.
    let app = host.launch("org.example.good").expect("launch good");
    APP.with(|a| *a.borrow_mut() = Some(app));
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        || APP.with(|a| a.borrow().as_ref().expect("running").surface()),
    );
    day_reactive::flush_sync();
    let labels: Vec<String> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.text.clone())
        .collect();
    assert!(
        labels.iter().any(|t| t == "good page"),
        "app B mounts after A's failure: {labels:?}"
    );

    APP.with(|a| a.borrow_mut().take());
    let _ = std::fs::remove_dir_all(&dir);
}
