use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Mutex,
};

use rand::seq::SliceRandom;
use teloxide::types::ChatId;
use time::OffsetDateTime;

use crate::model::{Task, TaskId};

use super::{
    bot_filter::{collect_filter_info, match_task, Filter, FilterInfo},
    bot_services::{Answer, AnswerStat, TaskInfoService, UserData, UserInfo, UserStateService},
};

#[derive(Debug)]
pub struct LocalTasks {
    tasks: Vec<Task>,
}

impl LocalTasks {
    pub fn new(mut tasks: Vec<Task>) -> Self {
        for task in &mut tasks {
            task.id = task.hash;
        }
        Self { tasks }
    }
}

impl TaskInfoService for LocalTasks {
    async fn get_task_ids(&self, filter: Option<&Filter>) -> anyhow::Result<Vec<TaskId>> {
        let mut rng = rand::thread_rng();
        let mut task_ids = self
            .tasks
            .iter()
            .filter(|task| match_task(&task.filters, filter.unwrap_or(&Filter::default())))
            .map(|task| task.id)
            .collect::<Vec<_>>();
        task_ids.shuffle(&mut rng);
        Ok(task_ids)
    }

    async fn collect_filter_info(&self) -> anyhow::Result<Vec<FilterInfo>> {
        Ok(collect_filter_info(&self.tasks))
    }

    async fn get_task(&self, id: i64) -> anyhow::Result<Option<Task>> {
        Ok(self.tasks.iter().find(|task| task.id == id).cloned())
    }
}

#[derive(Debug, Default, Clone)]
struct ChatState {
    user_data: UserData,
    tasks: Vec<TaskId>,
}

#[derive(Debug, Default)]
struct UserState {
    #[allow(dead_code)]
    user_info: UserInfo,
    answers: Vec<Answer>,
}

#[derive(Debug, Default)]
pub struct LocalUserStateService {
    state: Mutex<HashMap<i64, ChatState>>,
    user_state: Mutex<HashMap<i64, UserState>>,
}

impl UserStateService for LocalUserStateService {
    async fn touch_user(&self, user: &UserInfo) -> anyhow::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let entry = state.entry(user.uid);
        let is_new = matches!(entry, Entry::Vacant(_));
        if is_new {
            log::info!("New user: {:?}", user);
        }
        entry.or_default();
        Ok(is_new)
    }

    async fn get_state(&self, chat_id: ChatId) -> anyhow::Result<UserData> {
        let mut state = self.state.lock().unwrap();
        let user_state = state.entry(chat_id.0).or_default();
        Ok(user_state.user_data.clone())
    }

    async fn update_state(&self, chat_id: ChatId, update: UserData) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        let user_state = state.entry(chat_id.0).or_default();
        user_state.user_data = update;
        Ok(())
    }

    async fn update_tasks(&self, chat_id: ChatId, tasks: &[TaskId]) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        let user_state = state.entry(chat_id.0).or_default();
        user_state.tasks = tasks.to_vec();
        Ok(())
    }

    async fn take_next_task(&self, chat_id: ChatId) -> anyhow::Result<Option<TaskId>> {
        let mut state = self.state.lock().unwrap();
        let user_state = state.entry(chat_id.0).or_default();
        Ok(user_state.tasks.pop())
    }

    async fn record_anwer(&self, answer: Answer) -> anyhow::Result<()> {
        let mut state = self.user_state.lock().unwrap();
        let user_state = state.entry(answer.uid).or_default();
        user_state.answers.push(answer);
        Ok(())
    }

    async fn get_answer_stat(&self, user_id: i64, period: std::time::Duration) -> anyhow::Result<AnswerStat> {
        let mut state = self.user_state.lock().unwrap();
        let user_state = state.entry(user_id).or_default();
        let from = OffsetDateTime::now_utc() - period;
        let mut count = 0;
        let mut correct = 0;
        for answer in &user_state.answers {
            if answer.answered_at > from {
                count += 1;
                if answer.correct {
                    correct += 1;
                }
            }
        }

        Ok(AnswerStat { count, correct })
    }
}
