// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The predicate vocabulary: set membership, prefixes, and the null tests — plus the two
//! contracts that keep a two-path query layer honest. SQL's three-valued logic is the
//! in-memory rule too (a comparison against NULL is UNKNOWN, not false), and a predicate
//! whose SQL form would select different rows says so through `sql_exact`.

use day_macros::Model;
use day_model::{ModelId, Op, Uuid};
use day_persistence::{
    Fetch, ModelContainer, One, Pred, Recorder, RowView, Sqlite, Value, compare_values, schema,
};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "papers")]
struct Paper {
    #[model(id)]
    id: u32,
    title: String,
    /// Nullable on purpose: every three-valued assertion below runs through it.
    shelf: Option<String>,
    pages: i64,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "authors_p")]
struct Author {
    #[model(id)]
    id: Uuid,
    name: String,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "essays")]
struct Essay {
    #[model(id)]
    id: u32,
    title: String,
    author: Option<One<Author>>,
}

/// A row the predicate layer can read directly, so the unit assertions need no container.
struct Row(Vec<(&'static str, Value)>);

impl RowView for Row {
    fn col(&self, c: &str) -> Option<Value> {
        self.0.iter().find(|(n, _)| *n == c).map(|(_, v)| v.clone())
    }
}

fn text_row(shelf: Option<&str>) -> Row {
    Row(vec![(
        "shelf",
        match shelf {
            Some(t) => Value::Text(t.into()),
            None => Value::Null,
        },
    )])
}

fn papers() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Paper]).expect("open");
    let store = c.cache::<Paper>();
    for (id, title, shelf, pages) in [
        (1u32, "Quicksort", Some("A"), 12i64),
        (2, "quicksort revisited", Some("B"), 30),
        (3, "Mergesort", None, 8),
        (4, "Radix sort", Some("A"), 45),
        (5, "École de tri", None, 20),
    ] {
        store.restructure("add", Op::Insert, id, |v| {
            v.push(Paper {
                id,
                title: title.into(),
                shelf: shelf.map(str::to_string),
                pages,
            })
        });
    }
    c.save().expect("seed");
    c
}

fn matching(c: &ModelContainer, pred: Pred) -> Vec<u64> {
    let q = c
        .query::<Paper>()
        .filter(pred)
        .sort(Paper::id().asc())
        .live();
    q.ids().iter().map(|i| i.handle()).collect()
}

// --- is_in / not_in -------------------------------------------------------------------

#[test]
fn is_in_matches_the_set_and_nothing_else() {
    let c = papers();
    assert_eq!(matching(&c, Paper::pages().is_in([12i64, 45, 999])), [1, 4]);
    assert_eq!(
        matching(&c, Paper::shelf().is_in([Some("A".to_string())])),
        [1, 4]
    );
}

#[test]
fn an_empty_set_matches_nothing_and_its_complement_matches_all() {
    let c = papers();
    let none: [i64; 0] = [];
    assert!(matching(&c, Paper::pages().is_in(none)).is_empty());
    // NOT IN over an empty set is every row whose column is not NULL — SQL's rule, since a
    // NULL column is UNKNOWN rather than "not a member".
    assert_eq!(matching(&c, Paper::pages().not_in(none)), [1, 2, 3, 4, 5]);
    assert_eq!(
        matching(&c, Paper::shelf().not_in(Vec::<Option<String>>::new())),
        [1, 2, 4],
        "rows 3 and 5 have a NULL shelf: UNKNOWN, not selected"
    );
}

#[test]
fn a_single_element_set_agrees_with_eq() {
    let c = papers();
    assert_eq!(
        matching(&c, Paper::pages().is_in([30i64])),
        matching(&c, Paper::pages().eq(30i64)),
    );
}

#[test]
fn duplicates_in_the_set_do_not_duplicate_results() {
    let c = papers();
    assert_eq!(
        matching(&c, Paper::pages().is_in([12i64, 12, 12, 45])),
        [1, 4]
    );
}

#[test]
fn not_in_is_not_the_negation_of_is_in_over_nulls() {
    // The SQL rule, which the in-memory path now follows: `shelf NOT IN ('A')` does not
    // select a row whose shelf is NULL, but `NOT (shelf IN ('A'))` — three-valued NOT over
    // UNKNOWN — does not either. Both exclude it; what differs is that neither is a plain
    // boolean complement, which is exactly what a SQL author expects.
    let c = papers();
    let a = Some("A".to_string());
    assert_eq!(matching(&c, Paper::shelf().not_in([a.clone()])), [2]);
    assert_eq!(matching(&c, !Paper::shelf().is_in([a])), [2]);
}

#[test]
fn the_set_is_sorted_so_membership_is_a_binary_search() {
    // Ten thousand ids, built in a deliberately hostile order. A linear scan per candidate
    // row would make the seed quadratic; this is the guard that it is not.
    let c = ModelContainer::open(Sqlite::memory(), schema![Paper]).expect("open");
    let store = c.cache::<Paper>();
    for id in 0..10_000u32 {
        store.restructure("add", Op::Insert, id, |v| {
            v.push(Paper {
                id,
                title: format!("p{id}"),
                shelf: None,
                pages: id as i64,
            })
        });
    }
    c.save().expect("seed");

    let wanted: Vec<i64> = (0..10_000i64).rev().filter(|n| n % 2 == 0).collect();
    let started = std::time::Instant::now();
    let q = c
        .query::<Paper>()
        .filter(Paper::pages().is_in(wanted))
        .live();
    let elapsed = started.elapsed();
    assert_eq!(q.count(), 5_000);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "10k-row seed against a 5k-element set took {elapsed:?} — the set is not being \
         binary-searched"
    );
}

// --- starts_with ----------------------------------------------------------------------

#[test]
fn starts_with_is_case_sensitive() {
    // The LIKE trap: SQLite's LIKE is case-INsensitive for ASCII by default, so a form built
    // on it would also match row 2. This predicate must not.
    let c = papers();
    assert_eq!(matching(&c, Paper::title().starts_with("Quick")), [1]);
    assert_eq!(matching(&c, Paper::title().starts_with_ci("quick")), [1, 2]);
}

#[test]
fn an_empty_prefix_matches_every_row_with_a_value() {
    let c = papers();
    assert_eq!(
        matching(&c, Paper::title().starts_with("")),
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn a_multibyte_prefix_matches_by_character() {
    let c = papers();
    assert_eq!(matching(&c, Paper::title().starts_with("École")), [5]);
    assert_eq!(matching(&c, Paper::title().starts_with("Éc")), [5]);
    assert!(matching(&c, Paper::title().starts_with("Ecole")).is_empty());
}

// --- null tests -----------------------------------------------------------------------

#[test]
fn is_null_and_is_not_null_partition_the_table() {
    let c = papers();
    assert_eq!(matching(&c, Paper::shelf().is_null()), [3, 5]);
    assert_eq!(matching(&c, Paper::shelf().is_not_null()), [1, 2, 4]);
}

#[test]
fn a_reference_column_reports_set_and_unset() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Author, Essay]).expect("open");
    let ada = Uuid::now_v7();
    c.cache::<Author>()
        .restructure("add", Op::Insert, ada, |v| {
            v.push(Author {
                id: ada,
                name: "Ada".into(),
            })
        });
    let essays = c.cache::<Essay>();
    for (id, author) in [(1u32, Some(ada)), (2, None), (3, Some(ada))] {
        essays.restructure("add", Op::Insert, id, |v| {
            v.push(Essay {
                id,
                title: format!("e{id}"),
                author: author.map(One::to),
            })
        });
    }
    c.save().expect("seed");

    let ids = |p: Pred| -> Vec<u64> {
        c.query::<Essay>()
            .filter(p)
            .sort(Essay::id().asc())
            .live()
            .ids()
            .iter()
            .map(|i| i.handle())
            .collect()
    };
    assert_eq!(ids(Essay::author().is_unset()), [2]);
    assert_eq!(ids(Essay::author().is_set()), [1, 3]);
    assert_eq!(ids(Essay::author().is(ada)), [1, 3]);
    assert_eq!(ids(Essay::author().is_one_of([ada])), [1, 3]);
    assert!(ids(Essay::author().is_one_of(Vec::<ModelId<Author>>::new())).is_empty());
}

// --- three-valued logic ---------------------------------------------------------------

#[test]
fn a_comparison_against_null_is_unknown_not_false() {
    // The rule that lets the two paths agree: SQL's `shelf <> 'A'` does not select a row
    // whose shelf is NULL, and neither does this. Before three-valued evaluation, the
    // in-memory path selected it — because `compare_values` sorts NULL below text, which is
    // ORDER BY's rule, not WHERE's.
    let null_row = text_row(None);
    let a_row = text_row(Some("A"));
    let b = Value::Text("B".into());

    for pred in [
        Pred::Ne("shelf", b.clone()),
        Pred::Lt("shelf", b.clone()),
        Pred::Le("shelf", b.clone()),
        Pred::Gt("shelf", b.clone()),
        Pred::Ge("shelf", b.clone()),
        Pred::Between("shelf", Value::Text("A".into()), b.clone()),
        Pred::Contains("shelf", "A".into()),
        Pred::StartsWith("shelf", "A".into()),
        Pred::In("shelf", vec![Value::Text("A".into())]),
    ] {
        assert_eq!(
            pred.eval3(0, &null_row),
            None,
            "{pred:?} against NULL must be UNKNOWN"
        );
        assert!(
            !pred.eval(0, &null_row),
            "{pred:?} must not select a NULL row"
        );
    }
    // …while the same predicates stay definite over a present value.
    assert_eq!(Pred::Ne("shelf", b).eval3(0, &a_row), Some(true));
}

#[test]
fn unknown_propagates_through_and_or_and_not() {
    let null_row = text_row(None);
    let unknown = Pred::Eq("shelf", Value::Text("A".into()));
    let yes = Pred::Always;
    let no = Pred::Not(Box::new(Pred::Always));

    // AND: false dominates, otherwise UNKNOWN survives.
    assert_eq!(
        Pred::And(Box::new(unknown.clone()), Box::new(no.clone())).eval3(0, &null_row),
        Some(false)
    );
    assert_eq!(
        Pred::And(Box::new(unknown.clone()), Box::new(yes.clone())).eval3(0, &null_row),
        None
    );
    // OR: true dominates.
    assert_eq!(
        Pred::Or(Box::new(unknown.clone()), Box::new(yes)).eval3(0, &null_row),
        Some(true)
    );
    assert_eq!(
        Pred::Or(Box::new(unknown.clone()), Box::new(no)).eval3(0, &null_row),
        None
    );
    // NOT UNKNOWN is UNKNOWN — so a negated predicate still does not select a NULL row.
    assert_eq!(
        Pred::Not(Box::new(unknown.clone())).eval3(0, &null_row),
        None
    );
    assert!(!Pred::Not(Box::new(unknown)).eval(0, &null_row));
}

#[test]
fn is_null_stays_definite_about_null() {
    let null_row = text_row(None);
    assert_eq!(
        Pred::Eq("shelf", Value::Null).eval3(0, &null_row),
        Some(true)
    );
    assert_eq!(
        Pred::Ne("shelf", Value::Null).eval3(0, &null_row),
        Some(false)
    );
    // And a NOT over it is an ordinary negation, because there is no UNKNOWN to propagate.
    assert_eq!(
        Pred::Not(Box::new(Pred::Eq("shelf", Value::Null))).eval3(0, &null_row),
        Some(false)
    );
}

// --- the day_fold contract --------------------------------------------------------------

#[test]
fn case_insensitive_predicates_fold_full_unicode_in_sql() {
    // The divergence day_fold exists to close: Rust's `to_lowercase` is full Unicode,
    // SQLite's own `lower()` is ASCII only, so `École` folds one way and not the other.
    // The driver registers Rust's fold as a SQL function, and these run THROUGH the engine.
    let c = papers();
    assert_eq!(matching(&c, Paper::title().contains_ci("ÉCOLE")), [5]);
    assert_eq!(matching(&c, Paper::title().starts_with_ci("école")), [5]);
    assert_eq!(
        matching(&c, Paper::title().contains_ci("QUICKSORT")),
        [1, 2]
    );
    // Compounds fold each side independently.
    assert_eq!(
        matching(
            &c,
            Paper::title().contains_ci("ÉCOLE") | Paper::title().starts_with_ci("RADIX")
        ),
        [4, 5]
    );
}

// --- SQL forms (asserted through the Recorder\'s statement log) --------------------------

/// The last compiled query SELECT and its parameters, from a Recorder container.
fn compiled(pred: Pred) -> (String, Vec<Value>) {
    let (driver, log) = Recorder::new();
    let c = ModelContainer::open(driver, schema![Paper]).expect("open");
    log.clear();
    let _q = c.query::<Paper>().filter(pred).live();
    log.entries()
        .into_iter()
        .rev()
        .find(|(sql, _)| sql.starts_with("SELECT papers.id FROM papers"))
        .expect("a compiled SELECT was recorded")
}

#[test]
fn the_empty_set_compiles_to_a_constant_not_to_in_nothing() {
    // `IN ()` is a syntax error in SQLite rather than an empty set.
    let (sql, params) = compiled(Pred::In("pages", vec![]));
    assert!(sql.contains("WHERE 0"), "{sql}");
    assert!(params.is_empty());
    let (sql, _) = compiled(Pred::NotIn("pages", vec![]));
    assert!(
        sql.contains("WHERE papers.pages IS NOT NULL"),
        "NOT IN () is every row with a present value: {sql}"
    );
    let (sql, _) = compiled(Pred::IdIn(vec![]));
    assert!(sql.contains("WHERE 0"), "{sql}");
}

#[test]
fn set_and_prefix_forms_bind_what_they_say() {
    let (sql, params) = compiled(Pred::In("pages", vec![Value::Int(1), Value::Int(2)]));
    assert!(sql.contains("papers.pages IN (?, ?)"), "{sql}");
    assert_eq!(params, [Value::Int(1), Value::Int(2)]);

    let (sql, _) = compiled(Pred::NotIn("pages", vec![Value::Int(3)]));
    assert!(sql.contains("papers.pages NOT IN (?)"), "{sql}");

    // Prefix binds a CHARACTER count, not a byte count — `substr` counts characters.
    let (sql, params) = compiled(Pred::StartsWith("title", "École".into()));
    assert!(sql.contains("substr(papers.title, 1, ?) = ?"), "{sql}");
    assert_eq!(params[0], Value::Int(5), "5 characters, not 6 bytes");
}

// --- IdIn -----------------------------------------------------------------------------

#[test]
fn id_membership_reads_no_column_at_all() {
    // The row carries nothing; only the key decides. That is the property that makes this
    // the compilation target for relation traversal.
    let empty = Row(Vec::new());
    let pred = Pred::IdIn(vec![7, 9, 11]);
    assert!(pred.eval(9, &empty));
    assert!(!pred.eval(8, &empty));
    // And it depends on no column, so a column write can never move a row through it.
    let mut cols = Vec::new();
    pred.columns(&mut cols);
    assert!(cols.is_empty());
}

#[test]
fn a_fetch_over_ids_selects_exactly_those_rows() {
    let c = papers();
    assert_eq!(matching(&c, Pred::IdIn(vec![2, 4])), [2, 4]);
}

// --- the maintained path ---------------------------------------------------------------

#[test]
fn the_predicates_stay_live() {
    let c = papers();
    let q = c
        .query::<Paper>()
        .filter(Paper::title().starts_with("Quick") | Paper::pages().is_in([8i64]))
        .sort(Paper::id().asc())
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 3]
    );

    // A column the fetch never mentions costs nothing (proven with a trace in query.rs);
    // here: it also moves nothing.
    c.cache::<Paper>().elem(2).shelf().write(Some("Z".into()));
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 3]
    );

    // A dependency column moves exactly one row.
    c.cache::<Paper>()
        .elem(5)
        .title()
        .write("Quicksort in French".into());
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 3, 5]
    );
}

#[test]
fn a_swapped_fetch_reseeds_over_the_new_set() {
    let c = papers();
    let q = c.query_fn::<Paper>(|| Fetch::new().filter(Pred::IdIn(vec![1])));
    assert_eq!(q.count(), 1);
    q.set_fetch(Fetch::new().filter(Pred::IdIn(vec![1, 2, 3])));
    assert_eq!(q.count(), 3);
}

// --- the ordering rule the set relies on ------------------------------------------------

#[test]
fn membership_uses_the_same_equality_eq_does() {
    // `compare_values` puts Int and Real in one numeric class, so a binary search alone
    // would call Int(1) and Real(1.0) equal. Membership confirms exactly, so `is_in` and
    // `eq` can never disagree with each other.
    assert_eq!(
        compare_values(&Value::Int(1), &Value::Real(1.0)),
        std::cmp::Ordering::Equal
    );
    let row = Row(vec![("n", Value::Real(1.0))]);
    assert!(!Pred::In("n", vec![Value::Int(1)]).eval(0, &row));
    assert!(!Pred::Eq("n", Value::Int(1)).eval(0, &row));
    assert!(Pred::In("n", vec![Value::Real(1.0)]).eval(0, &row));
}

// --- the dependency structure -----------------------------------------------------------

#[test]
fn a_fetch_reports_its_predicate_and_sort_columns() {
    let deps = Fetch::new()
        .filter(Paper::pages().is_in([1i64]) & Paper::title().starts_with("Q"))
        .sort(Paper::shelf().asc())
        .dependencies();

    assert!(deps.touches_local("pages"));
    assert!(deps.touches_local("title"), "predicate columns count");
    assert!(deps.touches_local("shelf"), "so does the sort key");
    assert!(!deps.touches_local("id"), "a column nothing reads does not");
}

#[test]
fn an_id_set_depends_on_no_column() {
    // A row's key never changes, so no column write can move a row through `IdIn` — the
    // property that keeps relation traversal's compilation target free.
    let deps = Fetch::new().filter(Pred::IdIn(vec![1, 2])).dependencies();
    assert!(deps.local.is_empty());
    assert!(!deps.touches_local("pages"));
}

#[test]
fn a_local_fetch_crosses_no_relation() {
    let deps = Fetch::new()
        .filter(Paper::title().contains("sort"))
        .dependencies();
    assert!(deps.related.is_empty());
    assert!(deps.related_tables().is_empty());
    assert!(!deps.touches_related("lodging", "name"));
}
