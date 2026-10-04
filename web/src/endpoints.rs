//! The routes, and the shapes they send and receive.
//!
//! No handler carries a catch-all for failures. A domain type that rejects its
//! input returns an error, and the one mapper in `problem` turns that into a
//! status and a body. A per-handler catch is a per-handler chance to answer 200
//! for a failure or to put internals in a body.

use crate::constants::{methods, paths, query};
use crate::problem::Failure;
use crate::requests::{self, Query};
use crate::responses::{self, Answer, Body};
use application::inventory::InventoryService;
use application::ports::{InventoryStore, SignupStore, TaskStore};
use application::signups::SignupService;
use application::tasks::TaskService;
use domain::inventory::{InventoryQuery, InventoryResult};
use domain::signups::{SignupConfirmation, SignupRequest, validate};
use domain::tasks::{TaskFilter, TaskId, TaskItem, TaskTitle, TaskView, errors};
use hyper::body::Incoming;
use hyper::{Request, StatusCode};
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateSignup {
    /// The name box.
    pub full_name: String,
    /// The email box.
    pub email: String,
    /// The plan list.
    pub plan: String,
    /// The seats box, absent when left alone.
    pub seats: Option<i32>,
    /// The notes box.
    #[serde(default)]
    pub notes: String,
    /// The terms box.
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

/// The task routes.
///
/// # Errors
///
/// Returns whatever the domain or the store refused, mapped at the edge.
pub async fn tasks<S: TaskStore>(service: &TaskService<S>, request: Request<Incoming>) -> Answer {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    match (method.as_str(), path.as_str()) {
        (methods::GET, paths::TASKS) => {
            let filter = TaskFilter::parse(Query::of(&request).get(query::FILTER))?;
            let view = service.list(filter)?;
            responses::json(StatusCode::OK, &TaskViewResponse::from(&view))
        }
        (methods::POST, paths::TASKS) => {
            let sent: CreateTask = requests::read_json(request).await?;
            let created = service.create(&TaskTitle::new(&sent.title)?)?;
            responses::json(StatusCode::CREATED, &TaskResponse::from(&created))
        }
        (methods::DELETE, paths::TASKS) => {
            let removed = service.clear_completed()?;
            responses::json(StatusCode::OK, &ClearedTasks { removed })
        }
        (methods::PATCH, _) => {
            let id = task_id_from(&path)?;
            let sent: UpdateTask = requests::read_json(request).await?;
            let completed = sent.completed.ok_or_else(|| {
                Failure::from(domain::errors::DomainError::from(
                    errors::completed_required(),
                ))
            })?;
            let updated = service.set_completed(id, completed)?;
            responses::json(StatusCode::OK, &TaskResponse::from(&updated))
        }
        (methods::DELETE, _) => {
            service.delete(task_id_from(&path)?)?;
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
pub async fn signups<S: SignupStore>(
    service: &SignupService<S>,
    request: Request<Incoming>,
) -> Answer {
    const ALLOWED: &str = methods::POST;

    if request.method() != hyper::Method::POST {
        return Err(Failure::MethodNotAllowed {
            allowed: ALLOWED.to_owned(),
        });
    }

    let sent: CreateSignup = requests::read_json(request).await?;
    let validated = validate(&SignupRequest::from(sent))?;
    let confirmation = service.create(&validated)?;
    responses::json(StatusCode::CREATED, &SignupResponse::from(&confirmation))
}

/// The stock route.
///
/// # Errors
///
/// Returns a validation failure when the query names a column or an order the
/// table does not publish.
pub fn inventory<S: InventoryStore>(
    service: &InventoryService<S>,
    request: &Request<Incoming>,
) -> Answer {
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
    let answered = service.query(&asked)?;
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
/// # Errors
///
/// Returns [`Failure::NotReady`] when the dependency did not answer in time.
pub fn health<P>(
    readiness: &application::health::HealthService<P>,
    request: &Request<Incoming>,
) -> Answer
where
    P: application::ports::HealthProbe + Clone + Send + 'static,
{
    let ready = HealthResponse {
        status: domain::health::constants::STATUS_OK,
    };

    if request.uri().path() != paths::HEALTH_READY {
        return responses::json(StatusCode::OK, &ready);
    }

    let outcome = readiness.check();
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
