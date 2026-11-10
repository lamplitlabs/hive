/// The event a daemon-side adoption leaves on the record, in the daemon's
/// name: the revision itself is written by `hive_records::adopt`, which only
/// the owner may call, so the daemon's part is attributed here and in the
/// revision note rather than as the revision's author.
pub(crate) const AUTO_ADOPTED_ACTION: &str = "record.auto_adopted";

/// The revision note an auto-adoption carries, so the record's history says
/// who adopted it and under which setting.
pub(crate) const AUTO_ADOPTED_NOTE: &str =
    "auto-adopted by the daemon while the owner was away (records.auto_adopt=1)";

pub(crate) fn auto_adopt_drafts(s: &hive_store::Store, _now: String, actor: String) -> Vec<String> {
    if s.setting("owner.presence", hive_store::OWNER_PRESENT) != hive_store::OWNER_AWAY {
        return Vec::new();
    }
    if s.setting("records.auto_adopt", "0") != "1" {
        return Vec::new();
    }
    let drafts = match hive_records::drafts(s, "") {
        Ok(drafts) => drafts,
        Err(err) => {
            return vec![format!(
                "auto-adopt drafts: {}",
                crate::run::err_message(&err)
            )];
        }
    };
    let mut lines = Vec::new();
    for d in drafts {
        // Adoption is the owner's act in hive-records (an agent actor is
        // refused), so the call goes out as the owner on the strength of
        // records.auto_adopt=1, and the daemon signs the note and the event.
        match hive_records::adopt(s, &d.record_id, hive_store::HUMAN_AGENT, AUTO_ADOPTED_NOTE) {
            Ok(rec) => {
                if let Err(err) = s.event(
                    &actor,
                    AUTO_ADOPTED_ACTION,
                    &rec.id,
                    serde_json::json!({
                        "kind": rec.kind,
                        "rev": rec.current_rev,
                        "author": d.author,
                        "job": d.job_id,
                        "setting": "records.auto_adopt",
                    }),
                ) {
                    lines.push(format!(
                        "auto-adopt {}: event not recorded: {}",
                        rec.id,
                        crate::run::err_message(&err)
                    ));
                }
                lines.push(format!(
                    "auto-adopted {} {} r{} (by {actor}, drafted by {})",
                    rec.kind, rec.id, rec.current_rev, d.author
                ));
            }
            Err(err) => lines.push(format!(
                "auto-adopt {}: {}",
                d.record_id,
                crate::run::err_message(&err)
            )),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_records::{DraftInput, NewOptions};
    use rusqlite::params;

    #[test]
    fn auto_adopt_drafts_only_when_away_and_enabled() {
        let _serial = crate::test_helpers::serial();
        let s = auto_adopt_store();
        let rec = seed_record(&s);
        hive_records::draft(
            &s,
            &rec.id,
            DraftInput {
                body: "auto body".into(),
                author: "aurora".into(),
                note: "test".into(),
                job_id: "job-1".into(),
            },
        )
        .unwrap();

        let lines = auto_adopt_drafts(&s, s.now(), "daemon".into());
        assert!(
            lines.is_empty(),
            "present owner should not auto-adopt: {lines:?}"
        );

        s.set_setting(
            hive_store::HUMAN_AGENT,
            "owner.presence",
            hive_store::OWNER_AWAY,
        )
        .unwrap();
        let lines = auto_adopt_drafts(&s, s.now(), "daemon".into());
        assert!(
            lines.is_empty(),
            "away owner with auto_adopt=0 should not adopt: {lines:?}"
        );

        s.set_setting(hive_store::HUMAN_AGENT, "records.auto_adopt", "1")
            .unwrap();
        let lines = auto_adopt_drafts(&s, s.now(), "daemon".into());
        assert!(
            lines.len() == 1 && lines[0].contains("auto-adopted"),
            "expected one auto-adopted line, got {lines:?}"
        );
        assert!(
            lines[0].contains("by daemon") && lines[0].contains("drafted by aurora"),
            "the line names the adopter and the drafter: {lines:?}"
        );
        let draft = s.get_draft(&rec.id).unwrap();
        assert!(
            draft.is_none(),
            "draft should be cleared after auto-adoption: {draft:?}"
        );
    }

    /// An auto-adoption is the daemon's doing, not the owner's: the new
    /// revision's note says so and names the setting, and a
    /// `record.auto_adopted` event in the daemon's name points at the record,
    /// so the owner can find and revoke it when they are back.
    #[test]
    fn auto_adoption_is_attributed_to_the_daemon() {
        let _serial = crate::test_helpers::serial();
        let s = auto_adopt_store();
        let rec = seed_record(&s);
        hive_records::draft(
            &s,
            &rec.id,
            DraftInput {
                body: "auto body".into(),
                author: "aurora".into(),
                note: "test".into(),
                job_id: "job-1".into(),
            },
        )
        .unwrap();
        s.set_setting(
            hive_store::HUMAN_AGENT,
            "owner.presence",
            hive_store::OWNER_AWAY,
        )
        .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "records.auto_adopt", "1")
            .unwrap();
        let lines = auto_adopt_drafts(&s, s.now(), "daemon".into());
        assert_eq!(lines.len(), 1, "{lines:?}");

        let (head, rev) = hive_records::get(&s, &rec.id, 0).unwrap();
        assert_eq!(head.current_rev, 2);
        assert!(
            rev.note.contains(AUTO_ADOPTED_NOTE),
            "the revision note names the daemon and the setting: {}",
            rev.note
        );
        assert!(
            rev.note.contains("adopted the draft by aurora"),
            "the note keeps the drafting agent: {}",
            rev.note
        );

        let events = s
            .conn()
            .unwrap()
            .prepare("SELECT actor, subject, detail FROM events WHERE action = ?")
            .unwrap()
            .query_map(params![AUTO_ADOPTED_ACTION], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let (actor, subject, detail) = &events[0];
        assert_eq!(
            actor, "daemon",
            "the event is the daemon's, not the owner's"
        );
        assert_eq!(subject, &rec.id);
        let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["kind"], "memory");
        assert_eq!(detail["rev"], 2);
        assert_eq!(detail["author"], "aurora");
        assert_eq!(detail["job"], "job-1");
        assert_eq!(detail["setting"], "records.auto_adopt");
    }

    /// The switch is off until the owner turns it on: a fresh store with the
    /// owner away leaves every draft waiting.
    #[test]
    fn auto_adopt_is_off_by_default() {
        let _serial = crate::test_helpers::serial();
        let s = auto_adopt_store();
        assert_eq!(s.setting("records.auto_adopt", "x"), "0");
        let rec = seed_record(&s);
        hive_records::draft(
            &s,
            &rec.id,
            DraftInput {
                body: "auto body".into(),
                author: "aurora".into(),
                note: "test".into(),
                job_id: "job-1".into(),
            },
        )
        .unwrap();
        s.set_setting(
            hive_store::HUMAN_AGENT,
            "owner.presence",
            hive_store::OWNER_AWAY,
        )
        .unwrap();
        assert!(auto_adopt_drafts(&s, s.now(), "daemon".into()).is_empty());
        assert!(s.get_draft(&rec.id).unwrap().is_some(), "the draft waits");
    }

    fn auto_adopt_store() -> hive_store::Store {
        let _env = hive_testutil::env_lock();
        unsafe { std::env::remove_var("HIVE_JOB") };
        let home = tempfile::tempdir().unwrap();
        hive_store::Store::open(hive_store::Options {
            home: home.keep().display().to_string(),
            create: true,
            ..Default::default()
        })
        .unwrap()
    }

    fn seed_record(s: &hive_store::Store) -> hive_store::Record {
        let path = tempfile::tempdir().unwrap().keep();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO projects (slug, name, path, added_at, added_by) VALUES (?, ?, ?, ?, ?)",
                params!["proj", "Proj", path.display().to_string(), s.now(), "human"],
            )
            .unwrap();
        hive_records::new_record(
            s,
            "memory",
            "proj",
            "A lesson",
            NewOptions {
                body: "# A lesson\n".into(),
                author: "aurora".into(),
                ..Default::default()
            },
        )
        .unwrap()
    }
}
