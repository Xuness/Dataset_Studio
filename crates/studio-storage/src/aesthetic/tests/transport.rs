use super::*;

#[test]
fn raw_receipts_are_immutable_bounded_and_owned_by_the_attempt() {
    let (_dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let raw = LlmRawReceipt {
        http_status: 200,
        headers: Default::default(),
        provider_request_id: Some("request-1".into()),
        protocol: LlmProtocol::OpenaiChat,
        adapter_version: "native_json_v1".into(),
        complete: true,
        failure: None,
        body: b"not JSON; still paid evidence".to_vec(),
    };
    db.save_raw(&id, &attempt, raw.clone()).unwrap();
    db.save_raw(&id, &attempt, raw.clone()).unwrap();
    assert_eq!(
        db.raw_receipt(&id, &attempt).unwrap().unwrap().body,
        raw.body
    );
    assert!(db.raw_receipt("other", &attempt).unwrap().is_none());
    assert!(db.save_raw("other", &attempt, raw.clone()).is_err());
    let mut changed = raw;
    changed.body.push(0);
    assert!(db.save_raw(&id, &attempt, changed).is_err());
    assert_eq!(
        db.attempts(&id, batch.sequence).unwrap()[0]
            .raw_receipt
            .as_ref()
            .unwrap()
            .http_status,
        200
    );
    let aid = attempt.clone();
    db.writer
        .submit(1024, move |tx| {
            tx.execute(
                "UPDATE raw_receipts SET body=x'00' WHERE attempt_id=?1",
                [aid],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        db.raw_receipt(&id, &attempt).unwrap_err().code,
        "EVALUATION_CORRUPT"
    );
}

#[test]
fn regroup_preserves_original_identity_and_never_changes_a_sent_batch() {
    let (_dir, db, id) = fixture(16);
    let original = db.claim(&id).unwrap().unwrap();
    db.regroup(
        &id,
        original.sequence,
        vec![
            original.members[..8].to_vec(),
            original.members[8..].to_vec(),
        ],
        vec![],
        "byte_limit",
    )
    .unwrap();
    let rows = db.batches(&id, 0, 10).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].state, "superseded");
    assert_eq!(
        rows[0]
            .members
            .iter()
            .map(|m| &m.candidate.key)
            .collect::<Vec<_>>(),
        original
            .members
            .iter()
            .map(|m| &m.candidate.key)
            .collect::<Vec<_>>()
    );
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
    let (batch, _) = sent(&db, &id);
    assert_eq!(
        db.regroup(&id, batch.sequence, vec![batch.members], vec![], "illegal")
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn local_reparse_keeps_previous_normalization_and_does_not_double_count_usage() {
    let (_dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let good = receipt(&batch);
    let mut bad = good.clone();
    bad.outputs[0].content = vec![LlmContent::Text {
        text: "broken normalization".into(),
    }];
    db.receive(&id, &attempt, bad).unwrap();
    db.parse_received(&id).unwrap();
    db.settle(&id, Some("invalid".into())).unwrap();
    assert_eq!(db.stage(&id).unwrap().invalid, 1);
    db.apply_reparsed(&id, &attempt, good.clone()).unwrap();
    db.parse_received(&id).unwrap();
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.accepted, 1);
    assert_eq!(stage.invalid, 0);
    assert_eq!(stage.input_tokens, 17);
    db.apply_reparsed(&id, &attempt, good).unwrap();
    assert_eq!(db.stage(&id).unwrap().input_tokens, 17);
    let count: u64 = db
        .read()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM receipt_parses WHERE attempt_id=?1 AND normalized IS NOT NULL",
            [attempt],
            |r| crate::unsigned(r, 0),
        )
        .unwrap();
    assert_eq!(count, 2);
}
