use crate::inference::TextModel;
use std::ops::Deref;
use tokio::sync::{Mutex, MutexGuard};

type OptionalModel = Option<Box<dyn TextModel>>;
type Guard = Mutex<OptionalModel>;
pub struct TextModelGuard(Guard);

impl TextModelGuard {
    pub fn with_value(value: Option<Box<(dyn TextModel)>>) -> Self {
        Self(Mutex::new(value))
    }

    pub fn empty() -> Self {
        Self::with_value(None)
    }

    #[tracing::instrument(skip(self))]
    pub fn lock_now(&self) -> anyhow::Result<MutexGuard<OptionalModel>> {
        match self.try_lock() {
            Ok(lock) => {
                tracing::debug!("Locking model");
                Ok(lock)
            }
            Err(e) => {
                tracing::debug!("Model locked");
                Err(e.into())
            }
        }
    }
}

impl Deref for TextModelGuard {
    type Target = Guard;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
