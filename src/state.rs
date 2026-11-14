//! Process-local, application-owned state shared by handler contexts.
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

#[derive(Default)]
pub(crate) struct State {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    duplicate: Option<&'static str>,
}

impl State {
    pub(crate) fn insert<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        if self.values.insert(TypeId::of::<T>(), value).is_some() {
            self.duplicate = Some(std::any::type_name::<T>());
        }
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if let Some(name) = self.duplicate {
            return Err(crate::Error::Config(format!(
                "application state registered twice for {name}"
            )));
        }
        Ok(())
    }

    pub(crate) fn get<T: Send + Sync + 'static>(&self) -> crate::Result<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
            .ok_or_else(|| {
                crate::Error::Config(format!(
                    "register application state for {} before starting handlers",
                    std::any::type_name::<T>()
                ))
            })
    }
}
