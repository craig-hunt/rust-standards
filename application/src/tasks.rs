//! The task operations an endpoint calls.

use crate::ports::{StoreResult, TaskStore};
use domain::tasks::{TaskFilter, TaskId, TaskItem, TaskTitle, TaskView};

/// Counting and filtering run here rather than in the store, so every store
/// implementation reports the same totals and a test exercises the rule without
/// a database.
pub struct TaskService<S> {
    store: S,
}

impl<S: TaskStore> TaskService<S> {
    /// Wires the service to a store.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// The tasks the filter shows, with counts across the whole set.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported.
    pub fn list(&self, filter: TaskFilter) -> StoreResult<TaskView> {
        Ok(TaskView::summarize(&self.store.list()?, filter))
    }

    /// Records a new task.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported.
    pub fn create(&self, title: &TaskTitle) -> StoreResult<TaskItem> {
        self.store.create(title)
    }

    /// Sets a task's completion.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported, including not-found.
    pub fn set_completed(&self, id: TaskId, completed: bool) -> StoreResult<TaskItem> {
        self.store.set_completed(id, completed)
    }

    /// Removes one task.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported, including not-found.
    pub fn delete(&self, id: TaskId) -> StoreResult<()> {
        self.store.delete(id)
    }

    /// Removes the completed tasks, answering how many went.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported.
    pub fn clear_completed(&self) -> StoreResult<u64> {
        self.store.delete_completed()
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::TaskService;
    use crate::ports::{StoreResult, TaskStore};
    use domain::tasks::{TaskFilter, TaskId, TaskItem, TaskTitle, errors};
    use std::cell::RefCell;

    const FIRST: &str = "Read the ADR";
    const SECOND: &str = "Answer the questionnaire";
    const NEW_TITLE: &str = "Draft the runbook";
    const FIRST_ID: i64 = 1;
    const SECOND_ID: i64 = 2;
    const ABSENT_ID: i64 = 99;
    const ONE: usize = 1;
    const TWO: usize = 2;
    const ONE_CLEARED: u64 = 1;
    const NOTHING_CLEARED: u64 = 0;

    /// A store that holds its rows in memory.
    ///
    /// Hand-written rather than generated. The service calls five methods on one
    /// small trait, and a fake holding the rows lets a test assert on the result
    /// of a write rather than on the fact that a call happened.
    struct Remembering {
        rows: RefCell<Vec<TaskItem>>,
        next_id: RefCell<i64>,
    }

    impl Remembering {
        fn holding(rows: Vec<TaskItem>) -> Self {
            // The next identifier comes from the largest already present rather
            // than from the row count. Deriving it from a length would need a
            // cast from usize, and this repository forbids a cast that can wrap.
            let next = rows
                .iter()
                .map(|row| row.id.value())
                .max()
                .unwrap_or_default()
                + FIRST_ID;
            Self {
                rows: RefCell::new(rows),
                next_id: RefCell::new(next),
            }
        }
    }

    impl TaskStore for Remembering {
        fn list(&self) -> StoreResult<Vec<TaskItem>> {
            Ok(self.rows.borrow().clone())
        }

        fn create(&self, title: &TaskTitle) -> StoreResult<TaskItem> {
            let mut next = self.next_id.borrow_mut();
            let created = TaskItem {
                id: TaskId::new(*next)?,
                title: title.clone(),
                completed: false,
            };
            *next += FIRST_ID;
            self.rows.borrow_mut().push(created.clone());
            Ok(created)
        }

        fn set_completed(&self, id: TaskId, completed: bool) -> StoreResult<TaskItem> {
            let mut rows = self.rows.borrow_mut();
            let found = rows
                .iter_mut()
                .find(|row| row.id == id)
                .ok_or(errors::not_found())?;
            found.completed = completed;
            Ok(found.clone())
        }

        fn delete(&self, id: TaskId) -> StoreResult<()> {
            let mut rows = self.rows.borrow_mut();
            let before = rows.len();
            rows.retain(|row| row.id != id);
            if rows.len() == before {
                return Err(errors::not_found().into());
            }
            Ok(())
        }

        fn delete_completed(&self) -> StoreResult<u64> {
            let mut rows = self.rows.borrow_mut();
            let before = rows.len();
            rows.retain(|row| !row.completed);
            Ok(u64::try_from(before - rows.len()).unwrap_or_default())
        }
    }

    fn task(id: i64, title: &str, completed: bool) -> TaskItem {
        TaskItem {
            id: TaskId::new(id).unwrap(),
            title: TaskTitle::new(title).unwrap(),
            completed,
        }
    }

    fn service() -> TaskService<Remembering> {
        TaskService::new(Remembering::holding(vec![
            task(FIRST_ID, FIRST, false),
            task(SECOND_ID, SECOND, true),
        ]))
    }

    #[test]
    fn listing_counts_across_every_task_even_when_the_filter_narrows_it() {
        let view = service().list(TaskFilter::Completed).unwrap();

        assert_eq!(view.tasks.len(), ONE);
        assert_eq!(view.remaining, ONE, "one remains across the whole set");
        assert_eq!(view.total, TWO);
    }

    #[test]
    fn listing_shows_the_tasks_the_filter_names() {
        let view = service().list(TaskFilter::Active).unwrap();

        let titles: Vec<&str> = view.tasks.iter().map(|task| task.title.value()).collect();
        assert_eq!(titles, vec![FIRST]);
    }

    #[test]
    fn a_created_task_starts_out_incomplete() {
        let service = service();

        let created = service.create(&TaskTitle::new(NEW_TITLE).unwrap()).unwrap();

        assert_eq!(created.title.value(), NEW_TITLE);
        assert!(!created.completed);
        assert_eq!(service.list(TaskFilter::All).unwrap().total, TWO + ONE);
    }

    #[test]
    fn completion_can_be_set_and_unset() {
        let service = service();
        let id = TaskId::new(FIRST_ID).unwrap();

        assert!(service.set_completed(id, true).unwrap().completed);
        assert!(!service.set_completed(id, false).unwrap().completed);
    }

    #[test]
    fn setting_completion_on_a_task_that_does_not_exist_is_not_found() {
        let outcome = service().set_completed(TaskId::new(ABSENT_ID).unwrap(), true);

        assert!(outcome.is_err());
    }

    #[test]
    fn deleting_removes_the_task_it_names() {
        let service = service();

        service.delete(TaskId::new(SECOND_ID).unwrap()).unwrap();

        assert_eq!(service.list(TaskFilter::All).unwrap().total, ONE);
    }

    #[test]
    fn deleting_a_task_that_does_not_exist_is_not_found() {
        assert!(service().delete(TaskId::new(ABSENT_ID).unwrap()).is_err());
    }

    #[test]
    fn clearing_reports_how_many_went_and_leaves_the_rest() {
        let service = service();

        assert_eq!(service.clear_completed().unwrap(), ONE_CLEARED);
        assert_eq!(service.clear_completed().unwrap(), NOTHING_CLEARED);
        assert_eq!(service.list(TaskFilter::All).unwrap().total, ONE);
    }
}
