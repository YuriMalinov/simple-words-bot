use std::{future::Future, time::Duration};

use teloxide::types::ChatId;
use time::OffsetDateTime;

use crate::model::{Task, TaskId};

use super::bot_filter::{Filter, FilterInfo};

#[derive(Debug, Default, Clone)]
pub struct UserData {
    pub filter: Option<String>,
}

impl UserData {
    pub fn new(filter: Option<String>) -> Self {
        Self { filter }
    }
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub uid: i64,
    pub username: Option<String>,
    pub full_name: String,
    pub created_at: OffsetDateTime,
    pub last_active_at: OffsetDateTime,
}

impl Default for UserInfo {
    fn default() -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            uid: 0,
            username: None,
            full_name: "".into(),
            created_at: now,
            last_active_at: now,
        }
    }
}

impl UserInfo {
    pub fn from_tg_user(user: &teloxide::types::User) -> Self {
        Self {
            uid: user.id.0 as i64,
            username: user.username.clone(),
            full_name: user.full_name(),
            created_at: OffsetDateTime::now_utc(),
            last_active_at: OffsetDateTime::now_utc(),
        }
    }

    #[cfg(test)]
    pub fn new(uid: i64, username: Option<&str>, full_name: &str) -> Self {
        Self {
            uid,
            username: username.map(|s| s.into()),
            full_name: full_name.into(),
            created_at: OffsetDateTime::now_utc(),
            last_active_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub uid: i64,
    pub task_id: i64,
    pub correct: bool,
    pub asked_at: OffsetDateTime,
    pub answered_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct AnswerStat {
    pub count: i64,
    pub correct: i64,
}

pub trait UserStateService: std::fmt::Debug + Sync + Send {
    fn touch_user(&self, user: &UserInfo) -> impl Future<Output = anyhow::Result<bool>> + Send;
    fn get_state(&self, chat_id: ChatId) -> impl Future<Output = anyhow::Result<UserData>> + Send;
    fn update_state(&self, chat_id: ChatId, update: UserData) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn update_tasks(&self, chat_id: ChatId, tasks: &[TaskId]) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn take_next_task(&self, chat_id: ChatId) -> impl Future<Output = anyhow::Result<Option<TaskId>>> + Send;
    fn record_anwer(&self, answer: Answer) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn get_answer_stat(
        &self,
        user_id: i64,
        period: Duration,
    ) -> impl Future<Output = anyhow::Result<AnswerStat>> + Send;
}

pub trait TaskInfoService: std::fmt::Debug + Sync + Send {
    fn get_task_ids(&self, filter: Option<&Filter>) -> impl Future<Output = anyhow::Result<Vec<TaskId>>> + Send;
    fn collect_filter_info(&self) -> impl Future<Output = anyhow::Result<Vec<FilterInfo>>> + Send;
    fn get_task(&self, id: i64) -> impl Future<Output = anyhow::Result<Option<Task>>> + Send;
}
