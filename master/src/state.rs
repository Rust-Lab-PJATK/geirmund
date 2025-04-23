use std::{net::SocketAddr, str::FromStr};

use error::{
    AssignNameToWorkerError, CreateWorkerError, DeleteWorkerError, GetError, NewStateError,
    SetLoadingModelStatusError, SetModelError,
};
use proto::ModelType;
use sqlx::{migrate, Decode, Encode, Pool, Sqlite, SqlitePool, SqliteTransaction, Type};

use crate::Channel;

pub struct StateTransaction<'a> {
    tx: SqliteTransaction<'a>,
    change_signal: Channel<()>,
}

impl<'a> StateTransaction<'a> {
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        self.tx.commit().await?;

        self.change_signal.sender().send(()).unwrap();

        Ok(())
    }

    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        self.tx.rollback().await
    }
}

#[derive(Clone)]
pub struct State {
    pool: Pool<Sqlite>,

    change_signal: Channel<()>,
}

impl State {
    pub async fn new() -> Result<Self, NewStateError> {
        let random_id: u32 = rand::random();

        let connection_url = format!("file:memory_database_{random_id}?mode=memory");

        let pool = SqlitePool::connect(&connection_url).await?;

        migrate!("./migrations");

        Ok(Self {
            pool,
            change_signal: Channel::new(),
        })
    }

    pub async fn start_transaction<'a, 'b>(&'b self) -> Result<StateTransaction<'a>, sqlx::Error>
    where
        'a: 'b, // transaction lives longer than reference to state handle
    {
        Ok(StateTransaction {
            tx: self.pool.begin().await?,
            change_signal: self.change_signal.clone(),
        })
    }

    pub async fn create_worker<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
    ) -> Result<(), CreateWorkerError> {
        if let Err(error) = sqlx::query("INSERT INTO workers (socket_addr) VALUES (?);")
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await
        {
            if is_error_a_primary_key_violation(&error, "socket_addr") {
                return Err(CreateWorkerError::GivenSocketAddrAlreadyExists(socket_addr));
            }

            return Err(CreateWorkerError::DatabaseError(error));
        }

        Ok(())
    }

    pub async fn delete_worker<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
    ) -> Result<(), DeleteWorkerError> {
        let result = sqlx::query("DELETE FROM workers WHERE socket_addr = ?")
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await?;

        assert!(result.rows_affected() < 2, "Rows affected should be less than 2! Something nasty is going on with primary keys in memory db!");

        if result.rows_affected() == 0 {
            return Err(DeleteWorkerError::WorkerWithGivenSocketAddrDoesNotExist(
                socket_addr,
            ));
        }

        Ok(())
    }

    pub async fn assign_name_to_worker<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
        name: Option<String>,
    ) -> Result<(), AssignNameToWorkerError> {
        match sqlx::query("UPDATE workers SET name = ? WHERE socket_addr = ?")
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await
        {
            Ok(result) => {
                assert!(result.rows_affected() < 2, "Rows affected should be less than 2! Something nasty is going on with primary keys in memory db!");

                if result.rows_affected() == 0 {
                    return Err(
                        AssignNameToWorkerError::WorkerWithGivenSocketAddrDoesNotExist(socket_addr),
                    );
                } else {
                    return Ok(());
                }
            }
            Err(error) => {
                if is_error_a_unique_constraint_violation(&error, "name") {
                    return Err(AssignNameToWorkerError::NameAlreadyExists(
                        name.expect("expected name to not be a None"),
                    ));
                }

                return Err(AssignNameToWorkerError::DatabaseError(error));
            }
        };
    }

    pub async fn set_is_loading_model_status<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        is_loading: bool,
        socket_addr: SocketAddr,
    ) -> Result<(), SetLoadingModelStatusError> {
        let result = sqlx::query("UPDATE workers SET is_loading = ? WHERE socket_addr = ?")
            .bind(is_loading)
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await?;

        assert!(result.rows_affected() < 2, "Rows affected should be less than 2! Something nasty is going on with primary keys in memory db!");

        if result.rows_affected() == 0 {
            return Err(
                SetLoadingModelStatusError::WorkerWithGivenSocketAddrDoesNotExist(socket_addr),
            );
        }

        Ok(())
    }

    pub async fn set_model<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        model: Option<proto::ModelType>,
        socket_addr: SocketAddr,
    ) -> Result<(), SetModelError> {
        let model: Option<i32> = model.map(|model| model.into());

        let result = sqlx::query("UPDATE workers SET model = ? WHERE socket_addr = ?")
            .bind(model)
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await?;

        assert!(result.rows_affected() < 2, "Rows affected should be less than 2! Something nasty is going on with primary keys in memory db!");

        if result.rows_affected() == 0 {
            return Err(SetModelError::WorkerWithGivenSocketAddrDoesNotExist(
                socket_addr,
            ));
        }

        Ok(())
    }

    pub async fn get_all_workers<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
    ) -> Result<Vec<Worker>, GetError> {
        let result: Vec<Worker> = sqlx::query_as("SELECT * FROM workers")
            .fetch_all(&mut *tx.tx)
            .await?;

        Ok(result)
    }

    pub async fn get_worker_by_name<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        name: &str,
    ) -> Result<Option<Worker>, GetError> {
        Ok(sqlx::query_as("SELECT * FROM workers WHERE name = ?")
            .bind(name)
            .fetch_optional(&mut *tx.tx)
            .await?)
    }

    pub async fn get_worker_by_socket_addr<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
    ) -> Result<Option<Worker>, GetError> {
        Ok(sqlx::query_as("SELECT * FROM workers socket_addr = ?")
            .bind(socket_addr.to_string())
            .fetch_optional(&mut *tx.tx)
            .await?)
    }

    pub async fn create_generation_query<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
        query: String,
    ) -> Result<i64, error::CreateGenerationQueryError> {
        let result = sqlx::query(
            "INSERT INTO generation_queries (socket_addr, query, is_generating) VALUES (?, ?, false);",
        )
        .bind(socket_addr.to_string())
        .bind(query)
        .execute(&mut *tx.tx)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn set_status_of_generation_query<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
        is_generating: bool,
    ) -> Result<(), error::SetStatusOfGenerationQueryError> {
        sqlx::query("UPDATE generation_queries SET is_generating = ? WHERE socket_addr = ?")
            .bind(is_generating)
            .bind(socket_addr.to_string())
            .execute(&mut *tx.tx)
            .await?;

        Ok(())
    }

    pub async fn get_last_generation_query_for_socket_addr<'a>(
        &self,
        tx: &mut StateTransaction<'a>,
        socket_addr: SocketAddr,
    ) -> Result<Option<GenerationQuery>, error::GetLastGenerationQuery> {
        let generation_query: Option<GenerationQuery> = sqlx::query_as(
            "SELECT * FROM generation_queries WHERE socket_addr = ? ORDER id DESC LIMIT 1",
        )
        .bind(socket_addr.to_string())
        .fetch_optional(&mut *tx.tx)
        .await?;

        Ok(generation_query)
    }
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct GenerationQuery {
    id: i64,
    socket_addr: WorkerSocketAddr,
    query: String,
    is_generating: bool,
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Worker {
    socket_addr: WorkerSocketAddr,
    name: Option<String>,
    is_loading_model: bool,
    model: WorkerModelType,
}

#[derive(Clone, Debug)]
struct WorkerSocketAddr(SocketAddr);

#[derive(Clone, Debug)]
struct WorkerModelType(Option<proto::ModelType>);

impl<'a> sqlx::Decode<'a, Sqlite> for WorkerSocketAddr {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let value = <&str as Decode<Sqlite>>::decode(value)?;

        Ok(WorkerSocketAddr(
            SocketAddr::from_str(value).expect("valid socket addr"),
        ))
    }
}

impl<'a> sqlx::Encode<'a, Sqlite> for WorkerSocketAddr {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let stringified = self.0.to_string();

        <String as Encode<Sqlite>>::encode(stringified, buf)
    }
}

impl Type<Sqlite> for WorkerSocketAddr {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> sqlx::Decode<'a, Sqlite> for WorkerModelType {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let value = <Option<&str> as Decode<Sqlite>>::decode(value)?;

        Ok(WorkerModelType(value.map(|model| {
            ModelType::from_str_name(model).expect("valid value")
        })))
    }
}

impl<'a> sqlx::Encode<'a, Sqlite> for WorkerModelType {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let value: Option<String> = self.0.map(|val| val.as_str_name().to_string());

        match value {
            Some(value) => <String as Encode<Sqlite>>::encode(value, buf),
            None => Ok(sqlx::encode::IsNull::Yes),
        }
    }
}

impl Type<Sqlite> for WorkerModelType {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl Worker {
    pub fn socket_addr(&self) -> SocketAddr {
        self.socket_addr.0
    }

    pub fn name(&self) -> Option<String> {
        self.name.clone() // yeah, unfortunately
    }

    pub fn is_loading_model(&self) -> bool {
        self.is_loading_model
    }

    pub fn model(&self) -> Option<proto::ModelType> {
        self.model.0
    }
}

pub mod error {
    use std::net::SocketAddr;

    #[derive(thiserror::Error, Debug)]
    pub enum NewStateError {
        #[error("sqlite connection error: {0}")]
        SqliteConnectionError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum CreateWorkerError {
        #[error("given socket addr already exists: {0}")]
        GivenSocketAddrAlreadyExists(SocketAddr),

        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum WorkerCrudError {
        #[error("worker with given socket addr does not exist: {0}")]
        WorkerWithGivenSocketAddrDoesNotExist(SocketAddr),

        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    pub type SetLoadingModelStatusError = WorkerCrudError;
    pub type SetModelError = WorkerCrudError;
    pub type DeleteWorkerError = WorkerCrudError;

    #[derive(thiserror::Error, Debug)]
    pub enum GetError {
        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum AssignNameToWorkerError {
        #[error("worker with given socket addr does not exist: {0}")]
        WorkerWithGivenSocketAddrDoesNotExist(SocketAddr),

        #[error("worker with given name already exists: {0}")]
        NameAlreadyExists(String),

        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum CreateGenerationQueryError {
        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum SetStatusOfGenerationQueryError {
        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum GetLastGenerationQuery {
        #[error("database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
    }
}

fn is_error_a_primary_key_violation(error: &sqlx::Error, column: &str) -> bool {
    match error {
        sqlx::Error::Database(database_error) => {
            let message = database_error.message();
            let code = database_error
                .code()
                .map(|code| code.to_string())
                .unwrap_or_default();

            message.contains(column) && code == "1555"
        }
        _ => false,
    }
}

fn is_error_a_unique_constraint_violation(error: &sqlx::Error, column: &str) -> bool {
    match error {
        sqlx::Error::Database(database_error) => {
            let message = database_error.message();
            let code = database_error
                .code()
                .map(|code| code.to_string())
                .unwrap_or_default();

            message.contains(column) && code == "2067"
        }
        _ => false,
    }
}
