-- Add migration script here
CREATE TABLE user_info (
    uid BIGINT NOT NULL,
    username text NULL,
    full_name text NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    last_active_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (uid)
);

CREATE TABLE user_state (
    chat_id BIGINT NOT NULL,
    filter text,
    PRIMARY KEY (chat_id)
);

CREATE TABLE task_info (
    id bigserial NOT NULL,
    hash bigint not null,
    active boolean NOT NULL,
    filters jsonb NOT NULL,
    task_data jsonb NOT NULL,
    PRIMARY KEY (id)
);

create unique index task_hash on task_info (hash);

create table user_task (
    id bigserial not null,
    chat_id bigint not null, -- no ref as might reference chat with no user
    task_id bigint not null, -- no ref as might reference deleted task
    PRIMARY KEY (id)
);

create table user_answer (
    id bigserial not null,
    uid bigint not null references user_info(uid) on delete cascade,
    task_id bigint not null, -- no ref as might reference deleted task
    correct boolean,
    asked_at TIMESTAMP WITH TIME ZONE NOT NULL,
    answered_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (id)
);
