use std::{
    env,
    sync::{Once, OnceLock},
    time::{Duration, Instant},
};

use anyhow::Result;
use indoc::indoc;
use libc::atexit;
use sqlx::{migrate, postgres::PgPoolOptions, PgPool};
use testcontainers::{clients::Cli, Container};
use testcontainers_modules::postgres::Postgres;

static DOCKER: OnceLock<Cli> = OnceLock::new();
static PG_CONTAINER: OnceLock<Container<'static, Postgres>> = OnceLock::new();
static SINGLE_ENTRY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct AcquiredPg {
    #[allow(dead_code)]
    lock: tokio::sync::MutexGuard<'static, ()>,
    pub pool: PgPool,
}

impl std::ops::Deref for AcquiredPg {
    type Target = PgPool;

    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

fn init_logger() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", "debug,sqlx=info");
        }

        pretty_env_logger::init();
    })
}

async fn create_pool(container: &Container<'_, Postgres>) -> PgPool {
    let url = format!(
        "postgresql://postgres:postgres@localhost:{}/postgres",
        container.get_host_port_ipv4(5432)
    );

    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(2))
        .connect(url.as_str())
        .await
        .unwrap()
}

pub async fn setup_db() -> AcquiredPg {
    init_logger();

    let docker = DOCKER.get_or_init(Cli::default);
    let pg_container = PG_CONTAINER.get_or_init(|| {
        unsafe {
            atexit(close_db);
        }
        docker.run(Postgres::default())
    });

    // Migrate the database
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let pool = create_pool(PG_CONTAINER.get().unwrap()).await;
                    migrate!().run(&pool).await.unwrap();
                });
        })
        .join()
        .unwrap();
    });

    let lock = SINGLE_ENTRY.lock().await;
    let pool = create_pool(pg_container).await;
    cleanup_db(&pool).await.unwrap();

    AcquiredPg { pool, lock }
}

async fn cleanup_db(pool: &PgPool) -> Result<()> {
    let start = Instant::now();

    sqlx::query(indoc! {r"
        DO $$
        DECLARE
            _tbl text;
            _seq text;
            _cnt integer;
        BEGIN
            FOR _tbl IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public')
            LOOP
                EXECUTE format('SELECT 1 FROM %I LIMIT 1', _tbl) INTO _cnt;
                IF _cnt = 1 THEN
                    EXECUTE format('TRUNCATE TABLE %I CASCADE', _tbl);
                END IF;
            END LOOP;

            FOR _seq IN (SELECT sequencename FROM pg_sequences WHERE schemaname = 'public' and last_value > 1)
            LOOP
                EXECUTE format('SELECT setval(''%I'', 1, false)', _seq);
            END LOOP;
        END;
        $$"})
    .execute(pool)
    .await?;

    let duration = start.elapsed();
    log::debug!("Database cleanup took {:?}", duration);

    Ok(())
}

extern "C" fn close_db() {
    PG_CONTAINER.get().unwrap().rm();
}
