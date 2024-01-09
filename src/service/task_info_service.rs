use std::collections::HashMap;

use indoc::indoc;
use sqlx::{types::Json, PgPool};

use crate::{
    bot::{
        bot_filter::{match_task, Filter, FilterInfo},
        bot_services::TaskInfoService,
    },
    model::{Task, TaskId},
};

#[derive(Clone, Debug)]
pub struct PgTaskInfoService {
    pool: PgPool,
}

#[derive(Debug, sqlx::FromRow)]
struct TaskInfo {
    id: i64,
    // hash: i64, // no need to have it in struct
    // #[sqlx(json)]
    // filters: HashMap<String, String>, // no need to have it in struct
    #[sqlx(json)]
    task_data: Task,
}

impl PgTaskInfoService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn update_tasks(&self, tasks: &[Task]) -> anyhow::Result<(u64, u64)> {
        let mut tx = self.pool.begin().await?;

        let mut ids = Vec::new();
        for task in tasks {
            let filters: HashMap<String, String> = task
                .filters
                .iter()
                .map(|filter| (filter.name.clone(), filter.value.clone()))
                .collect();
            let filters = Json(filters);
            let task_data = Json(task.clone());
            let (id,): (i64,) = sqlx::query_as(indoc! {"
                    INSERT INTO task_info (hash, filters, active, task_data)
                    VALUES ($1, $2, true, $3)
                    ON CONFLICT (hash) DO UPDATE
                    SET filters = $2, active = true, task_data = $3
                    RETURNING id
                "})
            .bind(task.hash)
            .bind(filters)
            .bind(task_data)
            .fetch_one(&mut *tx)
            .await?;

            ids.push(id);
        }
        log::info!("Inserted {} tasks", ids.len());
        let inserted_count = ids.len();

        let result = sqlx::query(indoc! {"
                UPDATE task_info
                SET active = false
                WHERE id != all($1)
            "})
        .bind(ids)
        .execute(&mut *tx)
        .await?;

        log::info!("Deactivated {} tasks", result.rows_affected());

        tx.commit().await?;
        Ok((inserted_count as u64, result.rows_affected()))
    }
}

#[derive(Debug, sqlx::FromRow)]
struct FilterValueRow {
    key: String,
    values: Vec<String>,
}

impl TaskInfoService for PgTaskInfoService {
    async fn get_task_ids(&self, filter: Option<&Filter>) -> anyhow::Result<Vec<TaskId>> {
        let task_ids = sqlx::query_as::<_, TaskInfo>(indoc! {"
                SELECT id, hash, filters, task_data
                FROM task_info
                WHERE active = true
            "})
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        // very inefficient, but will replace this later with other mechanism
        .filter(|task| filter.map(|f| match_task(&task.task_data.filters, f)).unwrap_or(true))
        .map(|task| task.id)
        .collect::<Vec<_>>();
        Ok(task_ids)
    }

    async fn collect_filter_info(&self) -> anyhow::Result<Vec<FilterInfo>> {
        let values: Vec<FilterValueRow> = sqlx::query_as(indoc! {"
                select (r.r).key as key, array_agg(distinct (r.r).value::text order by (r.r).value) as values
                from (
                    SELECT jsonb_each_text(filters) as r
                    FROM task_info 
                    WHERE active = true
                ) as r
                group by 1
                order by 1
            "})
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::new();
        for FilterValueRow { key, values } in values {
            result.push(FilterInfo {
                name: key,
                possible_values: values,
            });
        }
        Ok(result)
    }

    async fn get_task(&self, id: i64) -> anyhow::Result<Option<Task>> {
        let task: Option<(Json<Task>,)> = sqlx::query_as(indoc! {"
                SELECT task_data
                FROM task_info
                WHERE id = $1
            "})
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(task.map(|(json,)| json.0))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Result;

    use crate::{
        bot::bot_filter::FilterGroup,
        model::{FilterValue, Task},
        test_db::setup_db,
    };

    #[tokio::test]
    async fn test_update_tasks() -> Result<()> {
        let pg = setup_db().await;
        let service = super::PgTaskInfoService::new(pg.pool.clone());
        let (inserted, deactivated) = service
            .update_tasks(&[
                Task {
                    id: 0,
                    hash: 1,
                    task: "task1".into(),
                    masked_task: "task1".into(),
                    correct: "correct1".into(),
                    base: "base1".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
                Task {
                    id: 0,
                    hash: 2,
                    task: "task2".into(),
                    masked_task: "task2".into(),
                    correct: "correct2".into(),
                    base: "base2".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
            ])
            .await?;
        assert_eq!(inserted, 2);
        assert_eq!(deactivated, 0);

        let tasks: Vec<TaskInfo> = sqlx::query_as("SELECT * FROM task_info").fetch_all(&pg.pool).await?;
        assert_eq!(tasks.len(), 2);

        let (updated, deactivated) = service
            .update_tasks(&[
                Task {
                    id: 0,
                    hash: 1,
                    task: "task1".into(),
                    masked_task: "task1".into(),
                    correct: "correct1".into(),
                    base: "base1".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
                Task {
                    id: 0,
                    hash: 3,
                    task: "task3".into(),
                    masked_task: "task3".into(),
                    correct: "correct3".into(),
                    base: "base3".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
            ])
            .await?;
        assert_eq!(updated, 2);
        assert_eq!(deactivated, 1);

        let tasks: Vec<TaskInfo> = sqlx::query_as("SELECT * FROM task_info").fetch_all(&pg.pool).await?;
        assert_eq!(tasks.len(), 3);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_task() -> Result<()> {
        let pg = setup_db().await;
        let service = super::PgTaskInfoService::new(pg.pool.clone());
        let (inserted, _) = service
            .update_tasks(&[
                Task {
                    id: 0,
                    hash: 1,
                    task: "task1".into(),
                    masked_task: "task1".into(),
                    correct: "correct1".into(),
                    base: "base1".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
                Task {
                    id: 0,
                    hash: 2,
                    task: "task2".into(),
                    masked_task: "task2".into(),
                    correct: "correct2".into(),
                    base: "base2".into(),
                    info: Vec::new(),
                    hints: Vec::new(),
                    filters: Vec::new(),
                    wrong_answers: Vec::new(),
                },
            ])
            .await?;
        assert_eq!(inserted, 2);

        let task = service.get_task(1).await?;
        assert!(task.is_some());
        let task = task.unwrap();
        assert_eq!(task.task, "task1");

        let task = service.get_task(3).await?;
        assert!(task.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_collect_filter_info() -> Result<()> {
        let pg = setup_db().await;
        let service = super::PgTaskInfoService::new(pg.pool.clone());
        let (inserted, _) = service
            .update_tasks(&[
                Task {
                    id: 0,
                    hash: 1,
                    task: "task1".into(),
                    masked_task: "task1".into(),
                    correct: "correct1".into(),
                    base: "base1".into(),
                    info: vec!["info1".into(), "info2".into()],
                    hints: Vec::new(),
                    filters: vec![
                        FilterValue {
                            name: "filter1".into(),
                            value: "value1".into(),
                        },
                        FilterValue {
                            name: "filter2".into(),
                            value: "value2".into(),
                        },
                    ],
                    wrong_answers: Vec::new(),
                },
                Task {
                    id: 0,
                    hash: 2,
                    task: "task2".into(),
                    masked_task: "task2".into(),
                    correct: "correct2".into(),
                    base: "base2".into(),
                    info: vec!["info1".into(), "info3".into()],
                    hints: Vec::new(),
                    filters: vec![
                        FilterValue {
                            name: "filter1".into(),
                            value: "value1".into(),
                        },
                        FilterValue {
                            name: "filter3".into(),
                            value: "value3".into(),
                        },
                    ],
                    wrong_answers: Vec::new(),
                },
            ])
            .await?;
        assert_eq!(inserted, 2);

        let filter_info = service.collect_filter_info().await?;
        assert_eq!(
            filter_info,
            vec![
                FilterInfo {
                    name: "filter1".into(),
                    possible_values: vec!["value1".into()]
                },
                FilterInfo {
                    name: "filter2".into(),
                    possible_values: vec!["value2".into()]
                },
                FilterInfo {
                    name: "filter3".into(),
                    possible_values: vec!["value3".into()]
                },
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_get_task_ids() -> Result<()> {
        let pg = setup_db().await;
        let service = super::PgTaskInfoService::new(pg.pool.clone());
        let (inserted, _) = service
            .update_tasks(&[
                Task {
                    id: 0,
                    hash: 1,
                    task: "task1".into(),
                    masked_task: "task1".into(),
                    correct: "correct1".into(),
                    base: "base1".into(),
                    info: vec!["info1".into(), "info2".into()],
                    hints: Vec::new(),
                    filters: vec![
                        FilterValue {
                            name: "filter1".into(),
                            value: "value1".into(),
                        },
                        FilterValue {
                            name: "filter2".into(),
                            value: "value2".into(),
                        },
                    ],
                    wrong_answers: Vec::new(),
                },
                Task {
                    id: 0,
                    hash: 2,
                    task: "task2".into(),
                    masked_task: "task2".into(),
                    correct: "correct2".into(),
                    base: "base2".into(),
                    info: vec!["info1".into(), "info3".into()],
                    hints: Vec::new(),
                    filters: vec![
                        FilterValue {
                            name: "filter1".into(),
                            value: "value311".into(),
                        },
                        FilterValue {
                            name: "filter3".into(),
                            value: "value3".into(),
                        },
                    ],
                    wrong_answers: Vec::new(),
                },
            ])
            .await?;
        assert_eq!(inserted, 2);

        let task_ids = service.get_task_ids(None).await?;
        assert_eq!(task_ids, vec![1, 2]);

        let task_ids = service
            .get_task_ids(Some(&Filter {
                groups: vec![FilterGroup {
                    values: vec!["value2".into(), "value3".into()],
                }],
            }))
            .await?;
        assert_eq!(task_ids, vec![1, 2]);

        let task_ids = service
            .get_task_ids(Some(&Filter {
                groups: vec![
                    FilterGroup {
                        values: vec!["value1".into()],
                    },
                    FilterGroup {
                        values: vec!["value2".into(), "value3".into()],
                    },
                ],
            }))
            .await?;
        assert_eq!(task_ids, vec![1]);

        Ok(())
    }
}
