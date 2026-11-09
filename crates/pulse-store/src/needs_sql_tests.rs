//! The scheduler's SQL side of `needs` (the store runs it; the hand never
//! does): the condition [`sql_fit`] builds walks the stored list.

use crate::needs::sql_fit;
use std::collections::BTreeSet;

fn caps(words: &[&str]) -> BTreeSet<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

#[test]
fn the_sql_condition_walks_the_stored_list() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE j (id TEXT, needs TEXT NOT NULL DEFAULT '');
         INSERT INTO j VALUES ('any', ''), ('mac', 'macos,xcode'), ('lin', 'linux'), ('dock', 'docker');",
    )
    .unwrap();
    let fits = |caps: &BTreeSet<String>| -> Vec<String> {
        let (cond, vals) = sql_fit("j.needs", caps);
        let mut st = conn
            .prepare(&format!("SELECT id FROM j WHERE {cond} ORDER BY id"))
            .unwrap();
        let refs: Vec<&dyn rusqlite::ToSql> =
            vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        st.query_map(refs.as_slice(), |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(fits(&caps(&["macos", "arm64", "xcode"])), ["any", "mac"]);
    assert_eq!(
        fits(&caps(&["linux", "amd64", "docker"])),
        ["any", "dock", "lin"]
    );
    assert_eq!(fits(&caps(&[])), ["any"]);
}
