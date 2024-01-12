use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use indoc::indoc;
use prost::Message;
use teloxide::dptree::deps;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButtonKind, InlineKeyboardMarkup, MessageEntity, MessageEntityKind};
use teloxide::Bot;
use thiserror::Error;
use time::OffsetDateTime;
use tokio::join;

use crate::bot::ask_next_task_handler::QUESTION_PRELUDE;
use crate::bot::bot_services::Answer;
use crate::utils::rus_numeric;

use super::ask_next_task_handler::ask_next_task;
use super::bot_services::{TaskInfoService, UserInfo, UserStateService};
use super::filter_handlers::handle_filter;
use super::proto;
use super::proto::command::Command;

#[derive(Debug)]
pub(super) struct BotContext<T: TaskInfoService, U: UserStateService> {
    pub(super) tasks: Arc<T>,
    pub(super) user_data: Arc<U>,
    pub(super) feedback_chat_id: Option<ChatId>,
}

#[derive(Debug)]
pub struct BotConfig {
    pub token: String,
    pub feedback_chat_id: Option<ChatId>,
}

#[derive(Error, Debug)]
pub enum BotErrors {
    #[error("No task found")]
    NoTaskFound,
    #[error("No task generated")]
    NoTaskGenerated,
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

pub async fn setup_and_run_bot(
    config: BotConfig,
    tasks: impl TaskInfoService + 'static,
    user_state: impl UserStateService + 'static,
) -> Result<()> {
    let bot = Bot::new(config.token);

    let context = BotContext {
        tasks: Arc::new(tasks),
        user_data: Arc::new(user_state),
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
                bot.send_message(message.chat.id, "Пожалуйста, напишите текст, что хотите отправить. Можно ответить на сообщение бота, чтобы сослаться на него.").send().await?;
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
                Command::QuestionAnswer(answer) => {
                    self.handle_answer(&bot, query.from.id, chat_id, answer, message).await
                }
            }
        })
        .await
    }

    async fn handle_answer(
        &self,
        bot: &Bot,
        user_id: UserId,
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

        let time = OffsetDateTime::from_unix_timestamp(answer.time_asked_ts / 1000)?
            + Duration::from_millis((answer.time_asked_ts % 1000) as u64);

        let record_answer = self.user_data.record_anwer(Answer {
            uid: user_id.0 as i64,
            task_id: answer.task_id,
            correct: answer.is_correct,
            asked_at: time,
            answered_at: OffsetDateTime::now_utc(),
        });

        let (send, record) = join!(call.send(), record_answer);
        send?;
        record?;

        let stat = self
            .user_data
            .get_answer_stat(user_id.0 as i64, Duration::from_secs(60 * 60 * 24))
            .await?;

        if stat.count % 5 == 0 {
            let percent = stat.correct * 100 / stat.count;
            let progress = match percent {
                0..=30 => "Поднажмём! 🥺",
                31..=90 => "Так держать! 🙂",
                _ => "Превосходно! 😎",
            };
            bot.send_message(
                chat_id,
                format!(
                    "{correct} правильно из {count} {tasks} за последние 24 часа ({percent}% правильных). {progress}",
                    correct = stat.correct,
                    count = stat.count,
                    tasks = rus_numeric(stat.count as usize, "задач", "задача", "задачи"),
                ),
            )
            .send()
            .await?;
        }

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
