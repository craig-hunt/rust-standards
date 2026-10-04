//! The routes, and the shapes they send and receive.
//!
//! No handler carries a catch-all for failures. A domain type that rejects its
//! input returns an error, and the one mapper in `problem` turns that into a
//! status and a body. A per-handler catch is a per-handler chance to answer 200
//! for a failure or to put internals in a body.

use crate::constants::{methods, paths, query};
use crate::offload::Offload;
use crate::problem::Failure;
use crate::requests::{self, Query};
use crate::responses::{self, Answer, Body};
use application::health::HealthService;
use application::inventory::InventoryService;
use application::ports::{InventoryStore, SignupStore, TaskStore};
use application::signups::SignupService;
use application::tasks::TaskService;
use domain::inventory::{InventoryQuery, InventoryResult};
use domain::signups::{SignupConfirmation, SignupRequest, validate};
use domain::tasks::{TaskFilter, TaskId, TaskItem, TaskTitle, TaskView, errors};
use hyper::body::{Body as HttpBody, Buf};
use hyper::{Request, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What a client sends to create a task.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateTask {
    /// The title, which the domain trims and checks.
    pub title: String,
}

/// What a client sends to change a task's completion.
///
/// The flag is optional so an omitted member reads as missing rather than as
/// false, which would silently reopen a completed task.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateTask {
    /// Whether the task is done.
    pub completed: Option<bool>,
}

/// What a client sends to record a signup.
///
/// Bound here rather than straight onto the domain's own request type, so the
/// wire names stay this layer's decision. They are the same names the domain
/// keys its field problems by, and a test asserts that, because a form cannot
/// place a message beside an input it cannot match.
///
/// Every member defaults, which is what makes those per-field problems
/// reachable. A missing member used to fail inside serde, so an incomplete form
/// answered `invalid_body` with no fields at all, and the route's promise to
/// report every problem at once held only for a form that already carried every
/// member. The domain is the thing that knows a name is required, and it cannot
/// say so about a body that never reached it. Unknown members still fail,
/// because a misspelled member is a different mistake from a missing one and
/// ignoring it would leave a client with no way to find the typo.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateSignup {
    /// The name box.
    #[serde(default)]
    pub full_name: String,
    /// The email box.
    #[serde(default)]
    pub email: String,
    /// The plan list.
    #[serde(default)]
    pub plan: String,
    /// The seats box, absent when left alone.
    pub seats: Option<i32>,
    /// The notes box.
    #[serde(default)]
    pub notes: String,
    /// The terms box.
    #[serde(default)]
    pub accept_terms: bool,
}

impl From<CreateSignup> for SignupRequest {
    fn from(sent: CreateSignup) -> Self {
        Self {
            full_name: sent.full_name,
            email: sent.email,
            plan: sent.plan,
            seats: sent.seats,
            notes: sent.notes,
            accept_terms: sent.accept_terms,
        }
    }
}

/// One task, as a client sees it.
///
/// Primitives rather than the domain's value types. A `TaskId` is a struct
/// wrapping an i64 and would serialize as an object, so the siblings install
/// converters; declaring the primitive here states the wire shape in the type
/// that defines it and costs one mapping.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResponse {
    /// The identifier.
    pub id: i64,
    /// The title.
    pub title: String,
    /// Whether it is done.
    pub completed: bool,
}

impl From<&TaskItem> for TaskResponse {
    fn from(task: &TaskItem) -> Self {
        Self {
            id: task.id.value(),
            title: task.title.value().to_owned(),
            completed: task.completed,
        }
    }
}

/// A filtered task list with counts over the whole set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskViewResponse {
    /// The tasks the filter shows.
    pub tasks: Vec<TaskResponse>,
    /// How many remain across every task.
    pub remaining: usize,
    /// How many exist.
    pub total: usize,
}

impl From<&TaskView> for TaskViewResponse {
    fn from(view: &TaskView) -> Self {
        Self {
            tasks: view.tasks.iter().map(TaskResponse::from).collect(),
            remaining: view.remaining,
            total: view.total,
        }
    }
}

/// How many tasks a clear removed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearedTasks {
    /// The count.
    pub removed: u64,
}

/// What a signup confirmation returns.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignupResponse {
    /// The identifier the database assigned.
    pub id: i64,
    /// The sentence reading the signup back.
    pub summary: String,
}

impl From<&SignupConfirmation> for SignupResponse {
    fn from(confirmation: &SignupConfirmation) -> Self {
        Self {
            id: confirmation.id.value(),
            summary: confirmation.summary.clone(),
        }
    }
}

/// One stock row, as a client sees it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItemResponse {
    /// What it is.
    pub name: String,
    /// How many are left.
    pub quantity: i32,
    /// What the table says.
    pub status: String,
}

/// Matching stock rows with counts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryResponse {
    /// The rows that matched.
    pub items: Vec<InventoryItemResponse>,
    /// How many matched.
    pub shown: usize,
    /// How many exist.
    pub total: usize,
}

impl From<&InventoryResult> for InventoryResponse {
    fn from(result: &InventoryResult) -> Self {
        Self {
            items: result
                .items
                .iter()
                .map(|row| InventoryItemResponse {
                    name: row.name.clone(),
                    quantity: row.quantity,
                    status: row.status.value().to_owned(),
                })
                .collect(),
            shown: result.shown,
            total: result.total,
        }
    }
}

/// What a health probe receives.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    /// Either `ok` or `unavailable`.
    pub status: &'static str,
}

/// Reads the identifier out of a path, treating an unreadable one as naming no
/// task.
///
/// Not-found rather than a validation failure: a path segment that is not a
/// number addresses nothing, and telling a client its value failed validation
/// would imply the path was otherwise right. The siblings answer the same way, by
/// refusing the route rather than the value.
fn task_id_from(path: &str) -> Result<TaskId, Failure> {
    path.strip_prefix(paths::TASKS_PREFIX)
        .and_then(TaskId::parse)
        .ok_or_else(|| Failure::from(errors::not_found()))
}

/// Whether a path addresses one task rather than the collection.
///
/// The wildcard arms of the task routes ask this instead of matching anything.
/// `(PATCH, _)` sat below the collection arms and so matched `/api/tasks` too,
/// which the handler then read as an identifier, failed to parse, and answered
/// 404 for. The collection does not answer PATCH, and 405 with an `Allow` header
/// is what tells a client that rather than implying the path was wrong.
fn addresses_one_task(path: &str) -> bool {
    path.starts_with(paths::TASKS_PREFIX)
}

/// The task routes.
///
/// # Errors
///
/// Returns whatever the domain or the store refused, mapped at the edge.
pub async fn tasks<S, B>(
    service: &Arc<TaskService<S>>,
    offload: &Offload,
    request: Request<B>,
) -> Answer
where
    S: TaskStore + Send + Sync + 'static,
    B: HttpBody,
    B::Data: Buf,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    match (method.as_str(), path.as_str()) {
        (methods::GET, paths::TASKS) => {
            let filter = TaskFilter::parse(Query::of(&request).get(query::FILTER))?;
            let reading = Arc::clone(service);
            let view = offload.run(move || reading.list(filter)).await?;
            responses::json(StatusCode::OK, &TaskViewResponse::from(&view))
        }
        (methods::POST, paths::TASKS) => {
            let sent: CreateTask = requests::read_json(request).await?;
            let title = TaskTitle::new(&sent.title)?;
            let writing = Arc::clone(service);
            let created = offload.run(move || writing.create(&title)).await?;
            responses::json(StatusCode::CREATED, &TaskResponse::from(&created))
        }
        (methods::DELETE, paths::TASKS) => {
            let clearing = Arc::clone(service);
            let removed = offload.run(move || clearing.clear_completed()).await?;
            responses::json(StatusCode::OK, &ClearedTasks { removed })
        }
        (methods::PATCH, item) if addresses_one_task(item) => {
            let id = task_id_from(&path)?;
            let sent: UpdateTask = requests::read_json(request).await?;
            let completed = sent.completed.ok_or_else(|| {
                Failure::from(domain::errors::DomainError::from(
                    errors::completed_required(),
                ))
            })?;
            let updating = Arc::clone(service);
            let updated = offload
                .run(move || updating.set_completed(id, completed))
                .await?;
            responses::json(StatusCode::OK, &TaskResponse::from(&updated))
        }
        (methods::DELETE, item) if addresses_one_task(item) => {
            let id = task_id_from(&path)?;
            let deleting = Arc::clone(service);
            offload.run(move || deleting.delete(id)).await?;
            responses::empty(StatusCode::NO_CONTENT)
        }
        _ => Err(Failure::MethodNotAllowed {
            allowed: allowed_on(&path),
        }),
    }
}

/// Which methods a task path answers, for the `Allow` header.
fn allowed_on(path: &str) -> String {
    const ON_COLLECTION: &str = "GET, POST, DELETE";
    const ON_ONE_TASK: &str = "PATCH, DELETE";

    if path == paths::TASKS {
        ON_COLLECTION.to_owned()
    } else {
        ON_ONE_TASK.to_owned()
    }
}

/// The signup route.
///
/// # Errors
///
/// Returns every field problem at once when the form is incomplete, so a client
/// corrects them in one pass.
pub async fn signups<S, B>(
    service: &Arc<SignupService<S>>,
    offload: &Offload,
    request: Request<B>,
) -> Answer
where
    S: SignupStore + Send + Sync + 'static,
    B: HttpBody,
    B::Data: Buf,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    const ALLOWED: &str = methods::POST;

    if request.method() != hyper::Method::POST {
        return Err(Failure::MethodNotAllowed {
            allowed: ALLOWED.to_owned(),
        });
    }

    let sent: CreateSignup = requests::read_json(request).await?;
    let validated = validate(&SignupRequest::from(sent))?;
    let recording = Arc::clone(service);
    let confirmation = offload.run(move || recording.create(&validated)).await?;
    responses::json(StatusCode::CREATED, &SignupResponse::from(&confirmation))
}

/// The stock route.
///
/// # Errors
///
/// Returns a validation failure when the query names a column or an order the
/// table does not publish.
pub async fn inventory<S, B>(
    service: &Arc<InventoryService<S>>,
    offload: &Offload,
    request: &Request<B>,
) -> Answer
where
    S: InventoryStore + Send + Sync + 'static,
{
    const ALLOWED: &str = methods::GET;

    if request.method() != hyper::Method::GET {
        return Err(Failure::MethodNotAllowed {
            allowed: ALLOWED.to_owned(),
        });
    }

    let parameters = Query::of(request);
    let asked = InventoryQuery::parse(
        parameters.get(query::SEARCH),
        parameters.get(query::SORT),
        parameters.get(query::DIRECTION),
    )?;
    let reading = Arc::clone(service);
    let answered = offload.run(move || reading.query(&asked)).await?;
    responses::json(StatusCode::OK, &InventoryResponse::from(&answered))
}

/// Liveness and readiness.
///
/// They say different things. Liveness answers as long as the process can serve
/// a request, so a platform restarts an instance only when it has actually
/// stopped working. Readiness answers for the dependencies, so a platform stops
/// sending traffic to an instance whose database has gone away without killing a
/// process that would recover when it comes back. Collapsing the two turns a
/// database outage into a restart loop.
///
/// Readiness logs the reason and answers with a generic body. The caller is a
/// platform probe, which acts on the status and has no use for the reason; the
/// operator needs the reason, and the log is where the operator looks.
///
/// Both probes answer GET and nothing else. A probe path that answered a POST
/// with the liveness body would tell a caller the service accepts writes there,
/// and the guard is the same one every other route carries.
///
/// # Errors
///
/// Returns [`Failure::NotReady`] when the dependency did not answer in time.
pub async fn health<B>(readiness: &Arc<HealthService>, request: &Request<B>) -> Answer {
    const ALLOWED: &str = methods::GET;

    if request.method() != hyper::Method::GET {
        return Err(Failure::MethodNotAllowed {
            allowed: ALLOWED.to_owned(),
        });
    }

    let ready = HealthResponse {
        status: domain::health::constants::STATUS_OK,
    };

    if request.uri().path() != paths::HEALTH_READY {
        return responses::json(StatusCode::OK, &ready);
    }

    // The check waits on a channel, which is a blocking wait however short, so it
    // happens on the blocking pool rather than on the worker polling this
    // connection. The thread it occupies is the one `PROBE_WORKERS` reserves.
    let asking = Arc::clone(readiness);
    let outcome = tokio::task::spawn_blocking(move || asking.check())
        .await
        .map_err(|_| Failure::NotReady)?;

    if outcome.ready {
        return responses::json(StatusCode::OK, &ready);
    }
    if let Some(reason) = outcome.failure {
        tracing::error!(%reason, "{}", domain::health::constants::MSG_NOT_READY);
    }
    Err(Failure::NotReady)
}

/// The body type the routes answer with, re-exported so the service can name it.
pub type ResponseBody = Body;

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{health, signups, tasks};
    use crate::constants::{headers, methods, paths};
    use crate::offload::Offload;
    use crate::problem::Failure;
    use application::health::HealthService;
    use application::ports::{SignupStore, StoreResult, TaskStore};
    use application::signups::SignupService;
    use application::tasks::TaskService;
    use domain::signups::{Signup, SignupId};
    use domain::tasks::{TaskId, TaskItem, TaskTitle};
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::{Method, Request, StatusCode};
    use std::sync::Arc;

    const ON_THE_COLLECTION: &str = "GET, POST, DELETE";
    const ONE_TASK: &str = "/api/tasks/4";
    const ASSIGNED_ID: i64 = 1;
    const NO_BODY: &str = "";
    const AN_EMPTY_FORM: &str = "{}";
    const FORM_MEMBERS: usize = 4;
    const EXPECTED_FIELDS: [&str; FORM_MEMBERS] = ["fullName", "email", "plan", "acceptTerms"];

    struct NoTasks;

    impl TaskStore for NoTasks {
        fn list(&self) -> StoreResult<Vec<TaskItem>> {
            Ok(Vec::new())
        }

        fn create(&self, _title: &TaskTitle) -> StoreResult<TaskItem> {
            unreachable!("no test here reaches a write")
        }

        fn set_completed(&self, _id: TaskId, _completed: bool) -> StoreResult<TaskItem> {
            unreachable!("no test here reaches a write")
        }

        fn delete(&self, _id: TaskId) -> StoreResult<()> {
            unreachable!("no test here reaches a write")
        }

        fn delete_completed(&self) -> StoreResult<u64> {
            unreachable!("no test here reaches a write")
        }
    }

    struct AcceptingSignups;

    impl SignupStore for AcceptingSignups {
        fn save(&self, _signup: &Signup) -> StoreResult<SignupId> {
            Ok(SignupId::new(ASSIGNED_ID)?)
        }
    }

    struct Answering;

    impl application::ports::HealthProbe for Answering {
        fn ping(&self) -> StoreResult<()> {
            Ok(())
        }
    }

    fn addressed(method: &Method, path: &str, body: &str) -> Request<Full<Bytes>> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Full::new(Bytes::from(body.to_owned())))
            .unwrap()
    }

    #[tokio::test]
    async fn patching_the_collection_is_a_method_it_does_not_answer_rather_than_a_missing_task() {
        let service = Arc::new(TaskService::new(NoTasks));

        let refused = tasks(
            &service,
            &Offload::new(),
            addressed(&Method::PATCH, paths::TASKS, NO_BODY),
        )
        .await
        .unwrap_err();

        match refused {
            Failure::MethodNotAllowed { allowed } => assert_eq!(allowed, ON_THE_COLLECTION),
            other => panic!("the collection does not answer PATCH, so 405: {other:?}"),
        }
    }

    #[tokio::test]
    async fn patching_one_task_still_reaches_the_handler_that_reads_the_identifier() {
        let service = Arc::new(TaskService::new(NoTasks));

        let refused = tasks(
            &service,
            &Offload::new(),
            addressed(&Method::PATCH, ONE_TASK, NO_BODY),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(refused, Failure::InvalidBody),
            "an item path reads its body, and this one sent none"
        );
    }

    #[tokio::test]
    async fn an_empty_form_is_answered_with_a_problem_for_every_box_it_left_out() {
        let service = Arc::new(SignupService::new(AcceptingSignups));

        let refused = signups(
            &service,
            &Offload::new(),
            addressed(&Method::POST, paths::SIGNUPS, AN_EMPTY_FORM),
        )
        .await
        .unwrap_err();

        let problem = refused.to_problem();
        let Some(fields) = problem.fields else {
            panic!("an incomplete form answers with a problem for every box it left out");
        };

        assert_eq!(problem.status, StatusCode::UNPROCESSABLE_ENTITY.as_u16());
        for member in EXPECTED_FIELDS {
            assert!(
                fields.contains_key(member),
                "{member} was left out and the client needs to be told where to look"
            );
        }
    }

    #[tokio::test]
    async fn a_probe_path_answers_a_read_and_refuses_anything_else() {
        let readiness = Arc::new(HealthService::new(Answering));

        let answered = health(&readiness, &addressed(&Method::GET, paths::HEALTH, NO_BODY))
            .await
            .unwrap();
        assert_eq!(answered.status(), StatusCode::OK);

        for path in [paths::HEALTH, paths::HEALTH_READY] {
            let refused = health(&readiness, &addressed(&Method::POST, path, NO_BODY))
                .await
                .unwrap_err();

            match refused {
                Failure::MethodNotAllowed { allowed } => assert_eq!(allowed, methods::GET),
                other => panic!("{path} is a probe, not a write surface: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn the_readiness_path_reports_the_dependency_answering() {
        let readiness = Arc::new(HealthService::new(Answering));

        let answered = health(
            &readiness,
            &addressed(&Method::GET, paths::HEALTH_READY, NO_BODY),
        )
        .await
        .unwrap();

        assert_eq!(answered.status(), StatusCode::OK);
        assert!(answered.headers().get(headers::ALLOW).is_none());
    }
}
