use luma_cad_jobs::*;
use rusqlite::{Connection, TransactionBehavior};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::Barrier,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn hash(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn scope() -> Scope {
    Scope {
        project: "project".into(),
        recipe: "recipe".into(),
    }
}

fn source(revision: u64) -> SourcePin {
    SourcePin {
        document: "00000000-0000-4000-8000-000000000002".into(),
        revision,
        definition_digest: hash('d'),
    }
}

fn spec(policy: Policy) -> TaskSpec {
    TaskSpec {
        operation: "plate.mesh".into(),
        major: 1,
        provider_fingerprint: hash('a'),
        input_digest: hash('1'),
        source: source(1),
        policy,
        inputs: BTreeMap::new(),
    }
}

fn contracts() -> BTreeMap<String, TaskContract> {
    [
        (
            "plate.mesh".into(),
            TaskContract {
                major: 1,
                provider_fingerprint: hash('a'),
                output_schema: "mesh/1".into(),
                inputs: BTreeMap::new(),
            },
        ),
        (
            "view.frame".into(),
            TaskContract {
                major: 1,
                provider_fingerprint: hash('b'),
                output_schema: "png/1".into(),
                inputs: [("mesh".into(), "mesh/1".into())].into(),
            },
        ),
    ]
    .into()
}

fn make_plan(tasks: BTreeMap<String, TaskSpec>, names: &[(&str, &str)]) -> Result<Plan, Error> {
    Plan::prepare(
        &contracts(),
        &tasks,
        names
            .iter()
            .map(|(key, task)| Intent {
                key: (*key).into(),
                task: (*task).into(),
            })
            .collect(),
    )
}

fn single(task: TaskSpec) -> Plan {
    make_plan([("mesh".into(), task)].into(), &[("preview", "mesh")]).unwrap()
}

fn mesh_artifact() -> Artifact {
    Artifact {
        schema: "mesh/1".into(),
        manifest_digest: hash('2'),
    }
}

fn fixture() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    install(&mut db).unwrap();
    db
}

fn admit_plan(db: &mut Connection, plan: &Plan) -> Vec<Delivery> {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let jobs = admit(&mut tx, &scope(), plan).unwrap();
    tx.commit().unwrap();
    jobs
}

fn pick(db: &mut Connection, provider: char) -> Option<Claimed> {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let result = claim(&mut tx, &hash(provider)).unwrap();
    tx.commit().unwrap();
    result
}

fn side_effect(db: &mut Connection, apply: impl FnOnce(&mut rusqlite::Transaction<'_>)) {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    apply(&mut tx);
    tx.commit().unwrap();
}

fn frame(dep: &str) -> TaskSpec {
    TaskSpec {
        operation: "view.frame".into(),
        provider_fingerprint: hash('b'),
        inputs: [("mesh".into(), FutureOutput { task: dep.into() })].into(),
        ..spec(Policy::HistoricalPinned)
    }
}

#[test]
fn preparation_checks_typed_dag_and_uses_one_shared_task() {
    let tasks = [
        ("render.left".into(), frame("mesh")),
        ("render.right".into(), frame("mesh")),
        ("mesh".into(), spec(Policy::HistoricalPinned)),
    ]
    .into();
    let plan = make_plan(tasks, &[("left", "render.left"), ("right", "render.right")]).unwrap();
    let mut db = fixture();
    let outputs = admit_plan(&mut db, &plan);
    assert_eq!(admit_plan(&mut db, &plan), outputs);
    assert!(pick(&mut db, 'b').is_none());
    let mesh = pick(&mut db, 'a').unwrap();
    assert!(mesh.inputs.is_empty());
    assert!(pick(&mut db, 'a').is_none());
    side_effect(&mut db, |tx| {
        complete(tx, mesh.attempt, &mesh_artifact()).unwrap()
    });
    assert!(pick(&mut db, 'b').is_none());
    side_effect(&mut db, |tx| {
        assert_eq!(publish(tx, mesh.attempt).unwrap(), Publication::Historical)
    });
    for output in outputs {
        let render = pick(&mut db, 'b').unwrap();
        assert_eq!(render.attempt.job, output.job);
        assert_eq!(render.inputs["mesh"], mesh_artifact());
    }

    let invalid = [
        ("first".into(), frame("second")),
        ("second".into(), frame("first")),
    ]
    .into();
    let mut cyclic_registry = contracts();
    cyclic_registry
        .get_mut("view.frame")
        .unwrap()
        .inputs
        .insert("mesh".into(), "png/1".into());
    assert!(matches!(
        Plan::prepare(
            &cyclic_registry,
            &invalid,
            vec![Intent {
                key: "root".into(),
                task: "first".into()
            }]
        ),
        Err(Error::Invalid("task dependency cycle"))
    ));
    assert!(matches!(
        make_plan(invalid, &[("root", "first")]),
        Err(Error::Invalid("future output schema does not match input"))
    ));
    let invalid = [(
        "mesh".into(),
        TaskSpec {
            provider_fingerprint: hash('c'),
            ..spec(Policy::HistoricalPinned)
        },
    )]
    .into();
    assert!(make_plan(invalid, &[("root", "mesh")]).is_err());
}

#[test]
fn retry_is_an_explicit_attempt_and_failure_blocks_dependents() {
    let mut db = fixture();
    let plan = make_plan(
        [
            ("frame".into(), frame("mesh")),
            ("mesh".into(), spec(Policy::HistoricalPinned)),
        ]
        .into(),
        &[("preview", "frame")],
    )
    .unwrap();
    let mappings = admit_plan(&mut db, &plan);
    let first = pick(&mut db, 'a').unwrap().attempt;
    side_effect(&mut db, |tx| fail(tx, first, "NotConverged").unwrap());
    assert_eq!(admit_plan(&mut db, &plan), mappings);
    assert!(pick(&mut db, 'a').is_none());
    assert!(pick(&mut db, 'b').is_none());
    side_effect(&mut db, |tx| retry(tx, first).unwrap());
    assert_eq!(receipt(&db, first.job).unwrap().state, State::Admitted);
    assert!(receipt(&db, first.job).unwrap().failure.is_none());
    let second = pick(&mut db, 'a').unwrap().attempt;
    assert_eq!(
        second,
        Attempt {
            job: first.job,
            number: first.number + 1
        }
    );
    assert_eq!(
        attempt_receipt(&db, first).unwrap().failure.as_deref(),
        Some("NotConverged")
    );
    side_effect(&mut db, |tx| {
        assert!(matches!(
            complete(tx, first, &mesh_artifact()),
            Err(Error::Conflict)
        ));
        let wrong = Artifact {
            schema: "png/1".into(),
            ..mesh_artifact()
        };
        assert!(complete(tx, second, &wrong).is_err());
        complete(tx, second, &mesh_artifact()).unwrap();
        assert_eq!(cancel(tx, second.job).unwrap(), CancelOutcome::Cancelled);
        assert!(matches!(publish(tx, second), Err(Error::Conflict)));
    });
    assert!(pick(&mut db, 'b').is_none());
    assert_eq!(receipt(&db, second.job).unwrap().state, State::Cancelled);
}

#[test]
fn current_publication_guards_source_and_slot_generations() {
    let mut db = fixture();
    side_effect(&mut db, |tx| set_head(tx, "project", &source(1)).unwrap());
    let policy = Policy::CurrentIfUnchanged {
        slot: "mesh".into(),
        expected_generation: 0,
    };
    let first = admit_plan(&mut db, &single(spec(policy.clone())))[0].clone();
    let new = TaskSpec {
        input_digest: hash('3'),
        ..spec(policy)
    };
    let next = admit_plan(&mut db, &single(new))[0].clone();
    assert_eq!(next.generation, first.generation + 1);
    let one = pick(&mut db, 'a').unwrap().attempt;
    let two = pick(&mut db, 'a').unwrap().attempt;
    assert_eq!(one.job, first.job);
    assert_eq!(two.job, next.job);
    side_effect(&mut db, |tx| {
        complete(tx, one, &mesh_artifact()).unwrap();
        complete(tx, two, &mesh_artifact()).unwrap();
        let expected = Publication::Current(Current {
            job: two.job,
            generation: 1,
        });
        assert_eq!(publish(tx, two).unwrap(), expected);
        assert_eq!(publish(tx, two).unwrap(), expected);
        assert_eq!(
            cancel(tx, two.job).unwrap(),
            CancelOutcome::AlreadyPublished
        );
        assert_eq!(publish(tx, one).unwrap(), Publication::Superseded);
    });
    assert_eq!(
        current(&db, "project", "mesh").unwrap().unwrap().job,
        two.job
    );
    let stale = TaskSpec {
        policy: Policy::CurrentIfUnchanged {
            slot: "mesh".into(),
            expected_generation: 1,
        },
        ..spec(Policy::HistoricalPinned)
    };
    admit_plan(&mut db, &single(stale));
    let old = pick(&mut db, 'a').unwrap().attempt;
    side_effect(&mut db, |tx| {
        complete(tx, old, &mesh_artifact()).unwrap();
        set_head(
            tx,
            "project",
            &SourcePin {
                definition_digest: hash('e'),
                ..source(2)
            },
        )
        .unwrap();
        set_head(tx, "project", &source(3)).unwrap(); // Old bytes, new revision.
        assert!(matches!(
            set_head(tx, "project", &source(1)),
            Err(Error::Conflict)
        ));
        assert_eq!(publish(tx, old).unwrap(), Publication::Superseded);
    });
    assert_eq!(current(&db, "project", "mesh").unwrap(), None);
    assert_eq!(slot_generation(&db, "project", "mesh").unwrap(), 1);
    assert_eq!(
        receipt(&db, old.job).unwrap().artifact,
        Some(mesh_artifact())
    );
}

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("luma-cad-jobs-{}-{time}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn db(&self) -> PathBuf {
        self.0.join("queue.sqlite")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sqlite_commit_and_recovery_share_the_model_transaction() {
    let temp = Temp::new();
    let mut db = Connection::open(temp.db()).unwrap();
    install(&mut db).unwrap();
    db.execute("CREATE TABLE model(revision INTEGER)", [])
        .unwrap();
    db.execute("INSERT INTO model VALUES(1)", []).unwrap();
    drop(db);
    for mode in ["before", "after"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "outbox_crash_child"])
            .env("LCAD_TEST_DB", temp.db())
            .env("LCAD_TEST_CRASH", mode)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        let mut reopened = Connection::open(temp.db()).unwrap();
        let revision: i64 = reopened
            .query_row("SELECT revision FROM model", [], |row| row.get(0))
            .unwrap();
        if mode == "before" {
            assert_eq!(revision, 1);
            assert!(intent(&reopened, &scope(), "preview").unwrap().is_none());
            assert!(pick(&mut reopened, 'a').is_none());
        } else {
            assert_eq!(revision, 2);
            let prior = intent(&reopened, &scope(), "preview").unwrap().unwrap();
            assert_eq!(
                admit_plan(&mut reopened, &single(spec(Policy::HistoricalPinned))),
                [prior]
            );
        }
    }
}

#[test]
fn outbox_crash_child() {
    let Some(path) = std::env::var_os("LCAD_TEST_DB") else {
        return;
    };
    let mut db = Connection::open(path).unwrap();
    let mut tx = db.transaction().unwrap();
    tx.execute("UPDATE model SET revision=2", []).unwrap();
    admit(&mut tx, &scope(), &single(spec(Policy::HistoricalPinned))).unwrap();
    if std::env::var("LCAD_TEST_CRASH").as_deref() == Ok("after") {
        tx.commit().unwrap();
    }
    std::process::exit(73);
}

#[test]
fn competing_workers_claim_one_durable_attempt() {
    let temp = Temp::new();
    let mut db = Connection::open(temp.db()).unwrap();
    install(&mut db).unwrap();
    let expected = admit_plan(&mut db, &single(spec(Policy::HistoricalPinned)))[0].job;
    let barrier = Barrier::new(2);
    let found = std::thread::scope(|scope| {
        let handles = std::array::from_fn::<_, 2, _>(|_| {
            scope.spawn(|| {
                let mut worker = Connection::open(temp.db()).unwrap();
                worker.busy_timeout(Duration::from_secs(5)).unwrap();
                barrier.wait();
                pick(&mut worker, 'a').map(|claimed| claimed.attempt)
            })
        });
        handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        found,
        [Attempt {
            job: expected,
            number: 1
        }]
    );
}

#[test]
fn cancelling_while_publishing_has_one_durable_outcome() {
    let temp = Temp::new();
    let mut db = Connection::open(temp.db()).unwrap();
    install(&mut db).unwrap();
    side_effect(&mut db, |tx| set_head(tx, "project", &source(1)).unwrap());
    let policy = Policy::CurrentIfUnchanged {
        slot: "mesh".into(),
        expected_generation: 0,
    };
    admit_plan(&mut db, &single(spec(policy)));
    let attempt = pick(&mut db, 'a').unwrap().attempt;
    side_effect(&mut db, |tx| {
        complete(tx, attempt, &mesh_artifact()).unwrap()
    });

    let barrier = Barrier::new(2);
    let (cancelled, published) = std::thread::scope(|threads| {
        let open = || {
            let connection = Connection::open(temp.db()).unwrap();
            connection.busy_timeout(Duration::from_secs(5)).unwrap();
            barrier.wait();
            connection
        };
        let requester = threads.spawn(move || {
            let mut connection = open();
            let mut tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let outcome = cancel(&mut tx, attempt.job).unwrap();
            tx.commit().unwrap();
            outcome
        });
        let worker = threads.spawn(move || {
            let mut connection = open();
            let mut tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let outcome = publish(&mut tx, attempt);
            tx.commit().unwrap();
            outcome
        });
        (requester.join().unwrap(), worker.join().unwrap())
    });
    let state = receipt(&db, attempt.job).unwrap().state;
    match (cancelled, published) {
        (CancelOutcome::Cancelled, Err(Error::Conflict)) => {
            assert_eq!(state, State::Cancelled);
            assert!(current(&db, "project", "mesh").unwrap().is_none());
        }
        (CancelOutcome::AlreadyPublished, Ok(Publication::Current(binding))) => {
            assert_eq!(state, State::Published);
            assert_eq!(current(&db, "project", "mesh").unwrap(), Some(binding));
        }
        other => panic!("invalid linearized outcome: {other:?}"),
    }
}

#[test]
fn failed_admission_does_not_leak_a_partial_graph() {
    let mut db = fixture();
    db.execute_batch(
        "CREATE TRIGGER reject_frame BEFORE INSERT ON lcad_jobs
        WHEN NEW.task_key='frame' BEGIN SELECT RAISE(ABORT, 'fixture fault'); END;",
    )
    .unwrap();
    let plan = make_plan(
        [
            ("frame".into(), frame("mesh")),
            ("mesh".into(), spec(Policy::HistoricalPinned)),
        ]
        .into(),
        &[("preview", "frame")],
    )
    .unwrap();
    side_effect(&mut db, |tx| assert!(admit(tx, &scope(), &plan).is_err()));
    assert!(intent(&db, &scope(), "preview").unwrap().is_none());
    assert!(pick(&mut db, 'a').is_none());
}

#[test]
fn luma_jobs_prepare_against_captured_receipts_and_share_the_authority_commit() {
    use luma::{
        host::{run_with_host, SourceBundle},
        host_api::{
            CallSite, Descriptor, Dispatch, Effect, Mode, Registry, ResultType, Type, Value,
        },
    };

    struct Provider(Registry);
    impl Dispatch for Provider {
        fn registry(&self) -> &Registry {
            &self.0
        }

        fn call(
            &self,
            descriptor: &Descriptor,
            _mode: Mode,
            _site: &CallSite,
            _arguments: &[Value],
        ) -> Result<Value, luma::host_api::Error> {
            let (kind, admitted) = match descriptor.operation.as_str() {
                "provider.task" => ("TaskSpecRef", "mesh"),
                "provider.job" => ("JobRef", "old"),
                _ => unreachable!(),
            };
            Ok(Value::Opaque {
                type_name: kind.into(),
                payload: serde_json::json!(admitted),
            })
        }
    }

    let provider = Provider(
        Registry::new(
            ["jobs".into(), "provider".into()],
            [("task", "TaskSpecRef"), ("job", "JobRef")].map(|(name, result)| Descriptor {
                module: "provider".into(),
                name: name.into(),
                operation: format!("provider.{name}"),
                major: 1,
                parameters: vec![],
                result: ResultType::Exact(Type::Opaque(result.into())),
                effect: Effect::ReadPinned,
                modes: [Mode::Command].into(),
            }),
        )
        .unwrap(),
    );
    let entry = "main.luma";
    let program = |extra: &str| {
        SourceBundle {
        entry: entry.into(),
        files: [(
            entry.into(),
            format!(
                "use jobs\nuse provider\npin = provider.job()\n                 status = jobs.status(pin)\nresult = jobs.result(pin)\n                 submitted = jobs.submit(\"new\", provider.task())\n{extra}"
            ),
        )]
        .into(),
    }
    };
    let task = spec(Policy::HistoricalPinned);
    let mut db = fixture();
    db.execute("CREATE TABLE accepted(revision INTEGER NOT NULL)", [])
        .unwrap();
    let old = admit_plan(&mut db, &single(task.clone()))[0].job;
    let captured = [("old".into(), receipt(&db, old).unwrap())].into();

    let attempt = pick(&mut db, 'a').unwrap().attempt;
    side_effect(&mut db, |tx| {
        complete(tx, attempt, &mesh_artifact()).unwrap();
        publish(tx, attempt).unwrap();
    });
    let mut changed = task;
    changed.input_digest = hash('3');
    let tasks = [("mesh".into(), changed)].into();
    let host = JobsHost::new(&provider, &tasks, &captured).unwrap();
    let run = run_with_host(&program(""), Mode::Command, &host, 1000).unwrap();
    assert!(
        matches!(&run.outputs["status"], Value::Opaque { payload, .. } if payload["state"] == "admitted")
    );
    assert!(
        matches!(&run.outputs["result"], Value::Opaque { payload, .. } if payload["kind"] == "pending")
    );
    assert!(
        matches!(&run.outputs["submitted"], Value::Opaque { type_name, .. } if type_name == "JobIntentRef")
    );
    let plan = host.finish(&contracts()).unwrap();
    assert!(intent(&db, &scope(), "new").unwrap().is_none());

    let current = [("old".into(), receipt(&db, old).unwrap())].into();
    let host = JobsHost::new(&provider, &tasks, &current).unwrap();
    let result = run_with_host(&program(""), Mode::Command, &host, 1000).unwrap();
    assert!(
        matches!(&result.outputs["result"], Value::Opaque { payload, .. } if payload["artifact"]["manifest_digest"] == hash('2'))
    );
    let rejected = JobsHost::new(&provider, &tasks, &current).unwrap();
    let duplicate = run_with_host(
        &program("again = jobs.submit(\"new\", provider.task())\n"),
        Mode::Command,
        &rejected,
        1000,
    )
    .unwrap_err();
    assert_eq!(duplicate.diagnostics[0].1.code, "InvalidJobIntentKey");
    assert!(intent(&db, &scope(), "new").unwrap().is_none());

    let admit_with_model = |tx: &mut rusqlite::Transaction<'_>| {
        tx.execute("INSERT INTO accepted VALUES(2)", []).unwrap();
        admit(tx, &scope(), &plan).unwrap()
    };
    {
        let mut tx = db.transaction().unwrap();
        admit_with_model(&mut tx);
        tx.rollback().unwrap();
    }
    let revisions = |db: &Connection| {
        db.query_row("SELECT count(*) FROM accepted", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(revisions(&db), 0);
    assert!(intent(&db, &scope(), "new").unwrap().is_none());
    let mut tx = db.transaction().unwrap();
    let delivery = admit_with_model(&mut tx);
    tx.commit().unwrap();
    assert_eq!(revisions(&db), 1);
    assert_eq!(
        intent(&db, &scope(), "new").unwrap(),
        Some(delivery[0].clone())
    );
    assert_eq!(admit_plan(&mut db, &plan), delivery);
}
