use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use indoc::indoc;
use prost::Message;
use rand::seq::SliceRandom;
use teloxide::dptree::deps;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButtonKind, InlineKeyboardMarkup, MessageEntity, MessageEntityKind};
use teloxide::Bot;
use thiserror::Error;
use time::OffsetDateTime;

use crate::bot::ask_next_task_handler::QUESTION_PRELUDE;
use crate::model::{Task, TaskId};

use super::ask_next_task_handler::ask_next_task;
use super::bot_filter::{collect_filter_info, match_task, Filter, FilterInfo};
use super::filter_handlers::handle_filter;
use super::proto;
use super::proto::command::Command;

#[derive(Debug)]
pub(super) struct BotContext<T: TaskInfoService, U: UserStateService> {
    pub(super) tasks: Arc<T>,
    pub(super) user_data: Arc<U>,
    pub(super) feedback_chat_id: Option<ChatId>,
}

#[derive(Debug, Default, Clone)]
pub struct UserData {
    pub(super) current_tasks: Vec<u64>,
    pub(super) filter: Option<String>,
}

#[derive(Debug)]
pub struct BotConfig {
    pub tasks: Vec<Task>,
    pub token: String,
    pub feedback_chat_id: Option<ChatId>,
}

pub(super) trait TaskInfoService: std::fmt::Debug + Sync + Send {
    fn get_task_ids(&self, filter: Option<&Filter>) -> impl Future<Output = anyhow::Result<Vec<TaskId>>> + Send;
    fn collect_filter_info(&self) -> impl Future<Output = anyhow::Result<Vec<FilterInfo>>> + Send;
    fn get_task(&self, id: u64) -> impl Future<Output = anyhow::Result<Option<Task>>> + Send;
}

impl TaskInfoService for Vec<Task> {
    async fn get_task_ids(&self, filter: Option<&Filter>) -> anyhow::Result<Vec<TaskId>> {
        let mut rng = rand::thread_rng();
        let mut task_ids = self
            .iter()
            .filter(|task| match_task(&task.filters, filter.unwrap_or(&Filter::default())))
            .map(|task| task.id)
            .collect::<Vec<_>>();
        task_ids.shuffle(&mut rng);
        Ok(task_ids)
    }

    async fn collect_filter_info(&self) -> anyhow::Result<Vec<FilterInfo>> {
        Ok(collect_filter_info(self))
    }

    async fn get_task(&self, id: u64) -> anyhow::Result<Option<Task>> {
        Ok(self.iter().find(|task| task.id == id).cloned())
    }
}

#[derive(Debug)]
pub struct UserInfo {
    pub uid: i64,
    pub username: Option<String>,
    pub full_name: String,
    pub created_at: OffsetDateTime,
    pub last_active_at: OffsetDateTime,
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
}

pub(super) trait UserStateService: std::fmt::Debug + Sync + Send {
    fn touch_user(&self, user: &UserInfo) -> impl Future<Output = anyhow::Result<bool>> + Send;
    fn get_state(&self, chat_id: ChatId) -> impl Future<Output = anyhow::Result<UserData>> + Send;
    fn update_state(&self, chat_id: ChatId, update: UserData) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl UserStateService for Mutex<HashMap<i64, UserData>> {
    async fn touch_user(&self, user: &UserInfo) -> anyhow::Result<bool> {
        let mut state = self.lock().unwrap();
        let entry = state.entry(user.uid);
        let is_new = matches!(entry, Entry::Vacant(_));
        if is_new {
            log::info!("New user: {:?}", user);
        }
        entry.or_default();
        Ok(is_new)
    }

    async fn get_state(&self, chat_id: ChatId) -> anyhow::Result<UserData> {
        let mut state = self.lock().unwrap();
        let user_state = state.entry(chat_id.0).or_default();
        Ok(user_state.clone())
    }

    async fn update_state(&self, chat_id: ChatId, update: UserData) -> anyhow::Result<()> {
        let mut state = self.lock().unwrap();
        state.insert(chat_id.0, update);
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum BotErrors {
    #[error("No task found")]
    NoTaskFound,
    #[error("No message found")]
    NoMessageFound,
    #[error("No data")]
    NoData,
    #[error("Wrong query")]
    WrongQuery,
    #[error("Bad callback data in button")]
    BadCallbackDataInButton,
    #[error("No reply markup")]
    NoReplyMarkup,
    #[error("No feedback chat id")]
    NoFeedbackChatId,
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn setup_and_run_bot(config: BotConfig) -> Result<()> {
    let bot = Bot::new(config.token);

    let user_data = Mutex::new(HashMap::new());
    let context = BotContext {
        tasks: Arc::new(config.tasks),
        user_data: Arc::new(user_data),
        feedback_chat_id: config.feedback_chat_id,
    };

    run_dispatcher(bot, context).await
}

async fn run_dispatcher<T: TaskInfoService + 'static, U: UserStateService + 'static>(
    bot: Bot,
    context: BotContext<T, U>,
) -> Result<()> {
    Dispatcher::builder(
        bot,
        dptree::entry()
            .branch(Update::filter_message().endpoint(
                |bot: Bot, message: teloxide::types::Message, ctx: Arc<BotContext<T, U>>| async move {
                    ctx.handle_message(bot, message).await
                },
            ))
            .branch(Update::filter_callback_query().endpoint(
                |bot: Bot, query: CallbackQuery, ctx: Arc<BotContext<T, U>>| async move {
                    ctx.handle_callback_query(bot, query).await
                },
            )),
    )
    .dependencies(deps![Arc::new(context)])
    .enable_ctrlc_handler()
    .build()
    .dispatch()
    .await;

    Ok(())
}

static HELP_TEXT: &str = indoc! {"
    Hi there! This bot will help you to learn cases in Serbian language (or at least try to).            

    You can start by typing /start command. Return to this message with /help or any other text.
    "};

impl<T: TaskInfoService, U: UserStateService> BotContext<T, U> {
    async fn handle_message(&self, bot: Bot, message: teloxide::types::Message) -> HandlerResult {
        let chat_id = message.chat.id;
        self.handle(&bot.clone(), chat_id, || async {
            if let Some(user) = message.from() {
                self.user_data.touch_user(&UserInfo::from_tg_user(user)).await?;
            } else {
                log::debug!("#{} got message from unknown user", chat_id);
            }

            let text = message.text().ok_or(anyhow::anyhow!("Not a text message"))?;

            let (command, text) = if let Some(command) = text.trim().strip_prefix('/') {
                let mut parts = command.splitn(2, ' ');
                (parts.next().unwrap_or_default(), parts.next())
            } else {
                ("", Some(text))
            };

            match command {
                "start" => {
                    ask_next_task(&bot, self, chat_id).await?;
                }
                "feedback" => {
                    self.send_feedback(&bot, text, &message).await?;
                }
                "filter" => {
                    handle_filter(&bot, text, chat_id, self).await?;
                }
                "filter-reset" => {
                    handle_filter(&bot, Some("-"), chat_id, self).await?;
                }
                _ => {
                    bot.send_message(chat_id, HELP_TEXT).send().await?;
                }
            }
            Ok(())
        })
        .await
    }

    async fn send_feedback(&self, bot: &Bot, text: Option<&str>, message: &teloxide::types::Message) -> Result<()> {
        let feedback_chat_id = self.feedback_chat_id.ok_or(BotErrors::NoFeedbackChatId)?;

        let text = match text {
            Some(text) => text,
            None => {
                bot.send_message(message.chat.id, "Пожалуйста, напишите текст, что хотите отправить. В дополнение можно ответить на сообщение бота, чтобы сослаться на него.").send().await?;
                return Ok(());
            }
        };

        let username = message
            .from()
            .as_ref()
            .map(|user| {
                format!(
                    "@{} ({})",
                    user.username.as_deref().unwrap_or("unknown"),
                    user.full_name()
                )
            })
            .unwrap_or_default();

        let reply = message
            .reply_to_message()
            .map(|r| r.text().unwrap_or_default())
            .map(|r| format!("\n\nReply to:\n\n{}", r))
            .unwrap_or_default();

        let message = format!("Feedback from {username}:\n\n{text}{reply}");
        bot.send_message(feedback_chat_id, message).send().await?;

        Ok(())
    }

    async fn handle_callback_query(&self, bot: Bot, query: CallbackQuery) -> HandlerResult {
        let message = query.message.as_ref().ok_or(BotErrors::NoMessageFound)?;
        self.user_data.touch_user(&UserInfo::from_tg_user(&query.from)).await?;

        let chat_id = message.chat.id;
        self.handle(&bot, chat_id, || async {
            bot.answer_callback_query(query.id).send().await?;

            let data = query.data.ok_or(BotErrors::NoData)?;
            let command = parse_command(data.as_str())?;
            let command = command.command.ok_or(BotErrors::WrongQuery)?;

            // For now the only command is answer
            match &command {
                Command::QuestionAnswer(answer) => self.handle_answer(&bot, chat_id, answer, message).await,
            }
        })
        .await
    }

    async fn handle_answer(
        &self,
        bot: &Bot,
        chat_id: ChatId,
        answer: &proto::QuestionAnswer,
        message: &teloxide::types::Message,
    ) -> HandlerResult {
        log::debug!("#{chat_id} got answer correct={correct}", correct = answer.is_correct);

        let buttons = &message
            .reply_markup()
            .ok_or(BotErrors::NoReplyMarkup)
            .context("reply_markup")?
            .inline_keyboard;

        let mut correct_text = &buttons[0][0].text;
        let mut answer_text = &buttons[0][0].text;
        for button in buttons {
            let button_command = if let InlineKeyboardButtonKind::CallbackData(command) = &button[0].kind {
                parse_command(command)?
            } else {
                return Err(BotErrors::BadCallbackDataInButton.into());
            };
            let button_command = button_command.command.ok_or(BotErrors::WrongQuery)?;

            let Command::QuestionAnswer(button_answer) = button_command;

            if button_answer.index == answer.index {
                answer_text = &button[0].text;
            }
            if button_answer.is_correct {
                correct_text = &button[0].text;
            }
        }

        let text = message.text().ok_or(BotErrors::NoMessageFound)?;
        let (mut text, entities_offset) = if let Some(prefix) = text.strip_prefix(QUESTION_PRELUDE) {
            (prefix.to_owned(), QUESTION_PRELUDE.chars().count())
        } else {
            (text.to_owned(), 0)
        };

        text.push_str("\n\n");
        if !answer.is_correct {
            text.push_str("\n❌ ");
            text.push_str(answer_text);
        }
        text.push_str("\n✅ ");
        text.push_str(correct_text);

        bot.edit_message_reply_markup(chat_id, message.id)
            .reply_markup(InlineKeyboardMarkup::default())
            .send()
            .await?;

        let fixed_entities = message.entities().map(|entities| {
            entities
                .iter()
                .filter(|entity| !matches!(entity.kind, MessageEntityKind::Spoiler))
                .map(|entity| MessageEntity {
                    kind: entity.kind.clone(),
                    offset: entity.offset - entities_offset,
                    length: entity.length,
                })
                .collect::<Vec<_>>()
        });

        let mut call = bot.edit_message_text(chat_id, message.id, text);
        if let Some(entities) = fixed_entities {
            call = call.entities(entities)
        }
        call.send().await?;

        tokio::time::sleep(Duration::from_secs(1)).await;

        ask_next_task(bot, self, chat_id).await?;

        Ok(())
    }

    async fn handle<F, Fut>(&self, bot: &Bot, chat_id: ChatId, callback: F) -> HandlerResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = HandlerResult>,
    {
        let result = callback().await;

        match result {
            Ok(_) => Ok(()),
            Err(err) => {
                log::error!("Error: {}", err);

                bot.send_message(
                    chat_id,
                    format!(
                        "Ууупс! случилась неприятность:\n{}\n\nНапишите (нажмите) /start, чтобы продолжить",
                        err
                    ),
                )
                .send()
                .await?;
                Ok(())
            }
        }
    }
}

fn parse_command(command: &str) -> Result<proto::Command> {
    let command = STANDARD.decode(command)?;
    let command = proto::Command::decode(&command[..])?;
    Ok(command)
}
