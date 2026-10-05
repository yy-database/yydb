use crate::format_v0;
use crate::journal::JournalMode;
use crate::lease;
use crate::Result;

use super::{is_yydx_main_path, Backend, Connection, State};

impl Connection {
    pub(crate) fn reconcile_leases_on_open(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            lease::reconcile_expired_leases(&mut state.records)?;
            Ok(())
        })
    }

    pub(crate) fn active_state_unlocked(&self) -> Result<State> {
        if let Some(state) = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            Ok(state.clone())
        } else {
            self.read_state()
        }
    }

    pub(crate) fn replace_active_state_unlocked(&self, state: State) -> Result<()> {
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            *slot = Some(state);
            Ok(())
        } else {
            drop(slot);
            self.write_state(&state)
        }
    }

    pub(crate) fn mutate_active_state_unlocked<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut State) -> Result<()>,
    {
        let mut state = self.active_state_unlocked()?;
        f(&mut state)?;
        self.replace_active_state_unlocked(state)
    }

    pub(crate) fn read_state(&self) -> Result<State> {
        match &self.backend {
            Backend::File { pager, .. } => {
                let mut pager = pager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                format_v0::read_state(&mut pager)
            }
            Backend::Memory { state } => Ok(state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()),
        }
    }

    pub(crate) fn write_state(&self, state: &State) -> Result<()> {
        match &self.backend {
            Backend::File { pager, .. } => {
                let mut pager = pager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let blob_objects = if self.path().is_some_and(is_yydx_main_path)
                    && self.journal_mode() == JournalMode::Wal
                {
                    Some(self.objects())
                } else {
                    None
                };
                format_v0::write_state_with_objects(&mut pager, state, blob_objects)
            }
            Backend::Memory { state: slot } => {
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state.clone();
                Ok(())
            }
        }
    }
}
