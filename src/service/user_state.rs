use crate::{
    bot::bot_services::{Answer, AnswerStat, UserData, UserInfo, UserStateService},
    model::TaskId,
};
use sqlx::{postgres::types::PgInterval, PgPool};
use teloxide::types::ChatId;

#[derive(Debug)]
pub struct PgUserService {
    pool: PgPool,
}

impl PgUserService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl UserStateService for PgUserService {
    async fn touch_user(&self, user: &UserInfo) -> anyhow::Result<bool> {
        let row: Option<(bool,)> = sqlx::query_as(indoc::indoc! {"
                INSERT INTO user_info (uid, username, full_name, created_at, last_active_at)
                VALUES ($1, $2, $3, now(), now())
                ON CONFLICT (uid) DO UPDATE SET last_active_at = now()
                RETURNING last_active_at = created_at
            "})
        .bind(user.uid)
        .bind(user.username.as_deref())
        .bind(user.full_name.as_str())
        .fetch_optional(&self.pool)
        .await?;

        let (new_user,) = row.ok_or(anyhow::format_err!("Failed to touch user"))?;

        Ok(new_user)
    }

    async fn get_state(&self, chat_id: ChatId) -> anyhow::Result<UserData> {
        let row: Option<(Option<String>,)> = sqlx::query_as(indoc::indoc! {"
                SELECT filter
                FROM user_state
                WHERE chat_id = $1
            "})
        .bind(chat_id.0)
        .fetch_optional(&self.pool)
        .await?;

        let (filter,) = row.unwrap_or_default();

        Ok(UserData::new(filter))
    }

    async fn update_state(&self, chat_id: ChatId, update: UserData) -> anyhow::Result<()> {
        sqlx::query(indoc::indoc! {"
                INSERT INTO user_state (chat_id, filter)
                VALUES ($1, $2)
                ON CONFLICT (chat_id) DO UPDATE SET filter = $2
            "})
        .bind(chat_id.0)
        .bind(update.filter)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn update_tasks(&self, chat_id: ChatId, tasks: &[TaskId]) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM user_task WHERE chat_id = $1")
            .bind(chat_id.0)
            .execute(&mut *tx)
            .await?;

        for task_id in tasks {
            sqlx::query("INSERT INTO user_task (chat_id, task_id) VALUES ($1, $2)")
                .bind(chat_id.0)
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    async fn take_next_task(&self, chat_id: ChatId) -> anyhow::Result<Option<TaskId>> {
        let row: Option<(i64,)> = sqlx::query_as(indoc::indoc! {"
                DELETE FROM user_task
                WHERE chat_id = $1 and task_id = (
                    SELECT task_id
                    FROM user_task
                    WHERE chat_id = $1
                    ORDER BY task_id
                    LIMIT 1
                )
                RETURNING task_id
            "})
        .bind(chat_id.0)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(task_id,)| task_id))
    }

    async fn record_anwer(&self, answer: Answer) -> anyhow::Result<()> {
        sqlx::query(indoc::indoc! {"
                INSERT INTO user_answer (uid, task_id, correct, asked_at, answered_at)
                VALUES ($1, $2, $3, $4, $5)
            "})
        .bind(answer.uid)
        .bind(answer.task_id)
        .bind(answer.correct)
        .bind(answer.asked_at)
        .bind(answer.answered_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_answer_stat(&self, user_id: i64, period: std::time::Duration) -> anyhow::Result<AnswerStat> {
        let interval = PgInterval::try_from(period)
            .map_err(|e| anyhow::format_err!("Failed to convert duration to interval: {}", e))?;

        let row: Option<(i64, i64)> = sqlx::query_as(indoc::indoc! {"
                SELECT count(*) as count, coalesce(sum(correct::int), 0) as correct
                FROM user_answer
                WHERE uid = $1 AND answered_at > now() - $2
            "})
        .bind(user_id)
        .bind(interval)
        .fetch_optional(&self.pool)
        .await?;

        let (count, correct) = row.unwrap_or_default();

        Ok(AnswerStat { count, correct })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_db::setup_db;
    use anyhow::Result;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn test_touch_user() -> Result<()> {
        let pg = setup_db().await;
        let service = PgUserService { pool: pg.pool };
        let user = UserInfo::new(1, Some("username"), "full_name");

        let new_user = service.touch_user(&user).await?;
        assert!(new_user);

        let new_user = service.touch_user(&user).await?;
        assert!(!new_user);

        Ok(())
    }

    #[tokio::test]
    async fn test_state() -> Result<()> {
        let pg = setup_db().await;
        let service = PgUserService { pool: pg.pool };
        let chat_id = ChatId(1);

        let state = service.get_state(chat_id).await?;
        assert_eq!(state.filter, None);

        let state = UserData::new(Some("filter".into()));
        service.update_state(chat_id, state.clone()).await?;

        let state = service.get_state(chat_id).await?;
        assert_eq!(state.filter, Some("filter".into()));

        let state = UserData::new(None);
        service.update_state(chat_id, state.clone()).await?;

        let state = service.get_state(chat_id).await?;
        assert_eq!(state.filter, None);

        Ok(())
    }

    #[tokio::test]
    async fn test_tasks() -> Result<()> {
        let pg = setup_db().await;
        let service = PgUserService { pool: pg.pool };
        let chat_id = ChatId(1);

        let tasks = vec![1, 2, 3];
        service.update_tasks(chat_id, &tasks).await?;

        let task = service.take_next_task(chat_id).await?;
        assert_eq!(task, Some(1));

        let task = service.take_next_task(chat_id).await?;
        assert_eq!(task, Some(2));

        let task = service.take_next_task(chat_id).await?;
        assert_eq!(task, Some(3));

        let task = service.take_next_task(chat_id).await?;
        assert_eq!(task, None);

        Ok(())
    }

    #[tokio::test]
    async fn test_answer_and_answer_stat() -> Result<()> {
        let pg = setup_db().await;
        let service = PgUserService { pool: pg.pool };
        let user_id = 1;

        service.touch_user(&UserInfo::new(user_id, Some("test"), "test")).await?;

        let stat = service.get_answer_stat(user_id, std::time::Duration::from_secs(1)).await?;
        assert_eq!(stat.count, 0);
        assert_eq!(stat.correct, 0);

        let answer = Answer {
            uid: user_id,
            task_id: 1,
            correct: true,
            asked_at: OffsetDateTime::now_utc() - std::time::Duration::from_secs(15),
            answered_at: OffsetDateTime::now_utc() - std::time::Duration::from_secs(15),
        };
        service.record_anwer(answer).await?;
        let answer = Answer {
            uid: user_id,
            task_id: 1,
            correct: false,
            asked_at: OffsetDateTime::now_utc() - std::time::Duration::from_secs(15),
            answered_at: OffsetDateTime::now_utc() - std::time::Duration::from_secs(15),
        };
        service.record_anwer(answer).await?;

        let stat = service.get_answer_stat(user_id, std::time::Duration::from_secs(60)).await?;
        assert_eq!(stat.count, 2);
        assert_eq!(stat.correct, 1);

        let stat = service.get_answer_stat(user_id, std::time::Duration::from_secs(0)).await?;
        assert_eq!(stat.count, 0);
        assert_eq!(stat.correct, 0);

        Ok(())
    }
}
