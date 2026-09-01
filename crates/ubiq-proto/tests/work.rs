//! The work family's payloads on the wire, and the answers a state gives about itself.
//!
//! Two things are being guarded here that nothing in the code says out loud. The first is that a
//! `TaskRecord` is the whole of what `tasks.toml` holds, so its encoding has to stay small enough
//! and plain enough for somebody to write one by hand: absent rather than null, a bare variant name
//! rather than a tag, `[[step]]` rather than a nested object. The second is that the states'
//! `all()` and `bucket()` are exhaustive — every match below is deliberately not a wildcard, so a
//! sixth activity or a sixth column is a compile error in this file rather than a badge that
//! silently sorts under the wrong pill.

use chrono::{DateTime, Utc};
use ubiq_proto::ids::{ProjectId, SessionId, StepId, TaskId, WorkspaceId};
use ubiq_proto::messages::Message;
use ubiq_proto::work::{
    Activity, AgentId, Bucket, Priority, Shape, Speaker, Status, Step, StepState, TaskRecord,
};

/// A fixed instant, so the record under test is the same one on every run.
fn moment() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-01T09:41:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn step(title: &str, state: StepState, owner: Option<AgentId>) -> Step {
    Step {
        id: StepId::generate(),
        title: title.to_string(),
        state,
        owner,
    }
}

/// How many of these are distinct, by value. `all()` returning the same variant twice would pass a
/// length check and lose a column.
fn distinct<T: PartialEq>(items: &[T]) -> usize {
    items
        .iter()
        .enumerate()
        .filter(|(i, item)| !items[..*i].contains(item))
        .count()
}

#[test]
fn a_task_with_everything_on_it_survives_the_wire_unchanged() {
    let task = TaskRecord {
        id: TaskId::generate(),
        session: Some(SessionId::generate()),
        status: Status::InProgress,
        priority: Priority::High,
        shape: Shape::Coordinated,
        title: "Split the work family off the session family".to_string(),
        // Markdown the host stores and never parses. The newlines and the markers are exactly what
        // a serialiser that decided to be clever would tidy away.
        description: "## Why\n\nTwo screens read the same record.\n\n- board\n- graph\n"
            .to_string(),
        steps: vec![
            step("write the contract", StepState::Done, None),
            step(
                "seed the columns",
                StepState::Working,
                Some(WorkspaceId::generate()),
            ),
            step("draw the graph", StepState::Idle, None),
        ],
        created_at: moment(),
        updated_at: moment(),
    };

    let json = serde_json::to_string(&task).unwrap();
    let back: TaskRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, task);

    // Equal by value is not enough for a store: the second write of an unchanged record must
    // produce the same file, or every read-modify-write churns the diff.
    assert_eq!(
        serde_json::to_string(&back).unwrap(),
        json,
        "re-encoding what was decoded must give the same bytes"
    );
}

#[test]
fn what_a_task_does_not_have_is_absent_from_the_encoding_rather_than_null() {
    // A task as `new()` makes one, which is the shape a hand-written `tasks.toml` starts from.
    let task = TaskRecord::new("Name it and leave".to_string(), None, moment());
    let json = serde_json::to_string(&task).unwrap();

    assert!(
        !json.contains("session"),
        "a task nobody has started names no session: {json}"
    );
    assert!(
        !json.contains("description"),
        "an empty description is not a key: {json}"
    );
    assert!(
        !json.contains("step"),
        "no steps is no array, not an empty one: {json}"
    );
    // The keys that are always there, so the absences above are absences and not a typo.
    for key in ["id", "status", "priority", "shape", "title", "created_at"] {
        assert!(json.contains(key), "{key} is always written: {json}");
    }

    let owned = Step {
        id: StepId::generate(),
        title: "seed the columns".to_string(),
        state: StepState::Working,
        owner: Some(WorkspaceId::generate()),
    };
    let unowned = Step {
        owner: None,
        ..owned.clone()
    };
    assert!(
        serde_json::to_string(&owned).unwrap().contains("owner"),
        "a step with an owner says so"
    );
    assert!(
        !serde_json::to_string(&unowned).unwrap().contains("owner"),
        "an unowned step has no owner key"
    );

    // The steps are `[[step]]` in the file, which is why the field is renamed. A reader who drops
    // the rename gets `steps = [{...}]` and a document nobody wants to edit.
    let with_steps = TaskRecord {
        steps: vec![owned],
        ..task
    };
    let json = serde_json::to_string(&with_steps).unwrap();
    assert!(json.contains("\"step\""), "the array is `step`: {json}");
    assert!(!json.contains("\"steps\""), "and not `steps`: {json}");
}

#[test]
fn a_work_message_travels_as_its_variant_name_and_a_payload() {
    let project_id = ProjectId::generate();
    let task_id = TaskId::generate();

    // A request, from the UI.
    let json = serde_json::to_value(Message::UpdateTask {
        project_id,
        task_id,
        title: Some("Split the work family".to_string()),
        description: None,
        priority: Some(Priority::High),
        shape: None,
    })
    .unwrap();

    assert_eq!(json["type"], "UpdateTask");
    assert_eq!(json["payload"]["task_id"], task_id.to_string());
    assert_eq!(
        json["payload"]["task_id"].as_str().unwrap().len(),
        26,
        "an id crosses as its 26 bare characters"
    );
    assert_eq!(json["payload"]["project_id"], project_id.to_string());
    // Display only: an update carries the fields it changes and `null` for the ones it does not.
    assert_eq!(json["payload"]["priority"], "High");
    assert!(json["payload"]["shape"].is_null());

    // And a response, from the host, so both directions are the same envelope.
    let task = TaskRecord::new("Name it and leave".to_string(), None, moment());
    let json = serde_json::to_value(Message::TaskChanged {
        project_id,
        task: task.clone(),
    })
    .unwrap();

    assert_eq!(json["type"], "TaskChanged");
    assert_eq!(json["payload"]["task"]["id"], task.id.to_string());
    assert_eq!(json["payload"]["task"]["id"].as_str().unwrap().len(), 26);
}

#[test]
fn every_state_serialises_as_its_bare_variant_name() {
    // This is what keeps `tasks.toml` hand-editable: `status = "InProgress"` is a line somebody can
    // type, and `status = { tag = "InProgress" }` is not. It is also the thing a
    // `#[serde(tag = ...)]` added to any of these enums later would break silently — every file
    // already written would still parse under the old form and nothing else in the tree would
    // notice, so the assertion lives here.
    assert_eq!(
        serde_json::to_string(&Status::InProgress).unwrap(),
        "\"InProgress\""
    );
    assert_eq!(
        serde_json::to_string(&Status::Backlog).unwrap(),
        "\"Backlog\""
    );
    assert_eq!(
        serde_json::to_string(&Priority::Normal).unwrap(),
        "\"Normal\""
    );
    assert_eq!(
        serde_json::to_string(&Shape::Coordinated).unwrap(),
        "\"Coordinated\""
    );
    assert_eq!(
        serde_json::to_string(&StepState::NeedsYou).unwrap(),
        "\"NeedsYou\""
    );
    assert_eq!(
        serde_json::to_string(&Activity::Thinking).unwrap(),
        "\"Thinking\""
    );
    assert_eq!(
        serde_json::to_string(&Bucket::Waiting).unwrap(),
        "\"Waiting\""
    );
    assert_eq!(serde_json::to_string(&Speaker::Agent).unwrap(), "\"Agent\"");

    // And the name is the whole of it, so what was written reads back.
    assert_eq!(
        serde_json::from_str::<Status>("\"InReview\"").unwrap(),
        Status::InReview
    );
    assert_eq!(
        serde_json::from_str::<StepState>("\"Failed\"").unwrap(),
        StepState::Failed
    );
}

#[test]
fn an_agent_id_is_a_workspace_id() {
    // Not a conversion and not a wrapper: the same type. This assert exists so a future reader who
    // takes `AgentId` for a kind of its own and "fixes" it into a separate `ulid_id!` fails here —
    // the alias is the point, because a workspace *is* its agent until one outlives its pane.
    let agent: AgentId = WorkspaceId::generate();
    assert_eq!(agent.to_string().len(), 26);
}

#[test]
fn a_newly_named_task_claims_nothing_it_cannot_know() {
    let now = moment();
    let task = TaskRecord::new("Name it and leave".to_string(), None, now);

    assert_eq!(task.title, "Name it and leave");
    assert_eq!(task.status, Status::Backlog);
    assert_eq!(
        task.priority,
        Priority::Normal,
        "unprioritised, not middling"
    );
    assert_eq!(task.shape, Shape::Direct);
    assert!(task.session.is_none());
    assert!(task.steps.is_empty());
    assert!(task.description.is_empty());
    assert_eq!(
        task.created_at, task.updated_at,
        "a task that has not changed was last changed when it was named"
    );

    // The session is the one thing `new()` is told, so it must arrive.
    let session = SessionId::generate();
    let started = TaskRecord::new("Started".to_string(), Some(session), now);
    assert_eq!(started.session, Some(session));
}

#[test]
fn unticking_a_step_lands_on_idle_rather_than_on_what_it_was_before() {
    let mut done = step("write the contract", StepState::Done, None);
    assert!(done.done());
    done.toggle();
    assert_eq!(
        done.state,
        StepState::Idle,
        "nothing here can know what its owner would go back to doing"
    );
    assert!(!done.done());

    // Every other state is a reason the box is not ticked, so ticking it is the same move from all
    // of them — a failed step included.
    for state in [
        StepState::Idle,
        StepState::Working,
        StepState::NeedsYou,
        StepState::Failed,
    ] {
        let mut s = step("seed the columns", state, None);
        assert!(!s.done(), "{state:?} is not done");
        s.toggle();
        assert_eq!(s.state, StepState::Done, "{state:?} ticks to done");
        assert!(s.done());
    }
}

#[test]
fn a_task_counts_its_done_steps_and_says_when_one_has_failed() {
    let steps = vec![
        step("write the contract", StepState::Done, None),
        step("seed the columns", StepState::Done, None),
        step("draw the graph", StepState::Working, None),
        step("wire the toolbar", StepState::NeedsYou, None),
    ];
    let ids: Vec<StepId> = steps.iter().map(|s| s.id).collect();
    let mut task = TaskRecord {
        steps,
        ..TaskRecord::new("Split the work family".to_string(), None, moment())
    };

    assert_eq!(task.done(), 2, "only the ticked ones count");
    assert!(
        !task.blocked(),
        "a step waiting on the user is not a step that failed"
    );

    assert_eq!(task.step(ids[2]).unwrap().title, "draw the graph");
    assert!(
        task.step(StepId::generate()).is_none(),
        "an id the task does not hold finds nothing"
    );

    // The step that failed is the one the user has to look at, which is what `blocked()` is for.
    task.step_mut(ids[3]).unwrap().state = StepState::Failed;
    assert!(task.blocked());
    assert_eq!(task.done(), 2, "a failure does not change the count");

    let empty = TaskRecord::new("Name it and leave".to_string(), None, moment());
    assert_eq!(empty.done(), 0);
    assert!(!empty.blocked(), "a task with no steps blocks on nothing");
}

#[test]
fn every_activity_reads_in_the_bucket_its_doc_comment_names() {
    // Three ways of working are one bucket, because the question the filter answers is "is it
    // moving", not "what is it doing".
    let cases = [
        (Activity::Thinking, Bucket::Running),
        (Activity::Writing, Bucket::Running),
        (Activity::Tools, Bucket::Running),
        (Activity::NeedsYou, Bucket::Waiting),
        (Activity::Ended, Bucket::Ended),
        (Activity::Failed, Bucket::Error),
    ];

    for (activity, bucket) in cases {
        assert_eq!(
            activity.bucket(),
            bucket,
            "{activity:?} sorts under {bucket:?}"
        );
        // Deliberately without a wildcard: a seventh activity is a compile error here, and the
        // table above is the only place to put it.
        match activity {
            Activity::Thinking
            | Activity::Writing
            | Activity::Tools
            | Activity::NeedsYou
            | Activity::Ended
            | Activity::Failed => {}
        }
    }
}

#[test]
fn every_step_state_reads_in_the_bucket_its_doc_comment_names() {
    // A step takes the same four colours as everything else on the screen rather than a palette of
    // its own, which is why a step that is finished and a step nobody has started share a bucket:
    // neither is moving.
    let cases = [
        (StepState::Idle, Bucket::Ended),
        (StepState::Done, Bucket::Ended),
        (StepState::Working, Bucket::Running),
        (StepState::NeedsYou, Bucket::Waiting),
        (StepState::Failed, Bucket::Error),
    ];

    for (state, bucket) in cases {
        assert_eq!(state.bucket(), bucket, "{state:?} sorts under {bucket:?}");
        match state {
            StepState::Idle
            | StepState::Working
            | StepState::NeedsYou
            | StepState::Failed
            | StepState::Done => {}
        }
    }
}

#[test]
fn each_state_lists_every_one_of_its_variants_exactly_once() {
    // `all()` seeds the board's columns and the pickers' menus, so a variant missing from one of
    // these lists is a state the user can never choose and a column that never draws.
    let buckets = Bucket::all();
    assert_eq!(
        buckets,
        [
            Bucket::Running,
            Bucket::Waiting,
            Bucket::Ended,
            Bucket::Error
        ]
    );

    let statuses = Status::all();
    assert_eq!(
        statuses,
        [
            Status::Backlog,
            Status::Ready,
            Status::InProgress,
            Status::InReview,
            Status::Done,
        ],
        "the order is the order the board draws them in"
    );

    let shapes = Shape::all();
    assert_eq!(shapes, [Shape::Direct, Shape::Chain, Shape::Coordinated]);

    let priorities = Priority::all();
    assert_eq!(
        priorities,
        [Priority::Low, Priority::Normal, Priority::High]
    );

    assert_eq!(distinct(&buckets), buckets.len(), "no bucket listed twice");
    assert_eq!(
        distinct(&statuses),
        statuses.len(),
        "no column listed twice"
    );
    assert_eq!(distinct(&shapes), shapes.len(), "no shape listed twice");
    assert_eq!(
        distinct(&priorities),
        priorities.len(),
        "no priority listed twice"
    );

    // Again without wildcards, so a new variant anywhere above stops compiling here first.
    for bucket in buckets {
        match bucket {
            Bucket::Running | Bucket::Waiting | Bucket::Ended | Bucket::Error => {}
        }
    }
    for status in statuses {
        match status {
            Status::Backlog
            | Status::Ready
            | Status::InProgress
            | Status::InReview
            | Status::Done => {}
        }
    }
    for shape in shapes {
        match shape {
            Shape::Direct | Shape::Chain | Shape::Coordinated => {}
        }
    }
    for priority in priorities {
        match priority {
            Priority::Low | Priority::Normal | Priority::High => {}
        }
    }
}
