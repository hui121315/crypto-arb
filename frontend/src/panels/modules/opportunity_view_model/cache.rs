use super::model::{OpportunityListViewModel, OpportunityListViewRow};
use shared_types::{is_p0_executable_strategy, OpportunityListRow};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

thread_local! {
    static LIST_VM_CACHE: RefCell<OpportunityListVmCache> = RefCell::new(OpportunityListVmCache::default());
}

pub(crate) fn view_models_from_rows(
    snapshot_id: &str,
    rows: Vec<OpportunityListRow>,
) -> Vec<OpportunityListViewRow> {
    let rows = rows.into_iter().filter(is_main_p0_row).collect();
    LIST_VM_CACHE.with(|cache| cache.borrow_mut().view_models(snapshot_id, rows))
}

#[cfg(test)]
pub(crate) fn rejected_main_p0_ids(rows: &[OpportunityListRow]) -> Vec<String> {
    rows.iter()
        .filter(|row| !is_main_p0_row(row))
        .map(|row| row.id.clone())
        .collect()
}

fn is_main_p0_row(row: &OpportunityListRow) -> bool {
    row.strategy_kind.is_some_and(is_p0_executable_strategy)
}

#[derive(Default)]
struct OpportunityListVmCache {
    snapshot_id: String,
    rows: BTreeMap<String, OpportunityListViewRow>,
}

impl OpportunityListVmCache {
    fn view_models(
        &mut self,
        snapshot_id: &str,
        rows: Vec<OpportunityListRow>,
    ) -> Vec<OpportunityListViewRow> {
        if self.snapshot_id != snapshot_id {
            self.snapshot_id = snapshot_id.to_owned();
            self.rows.clear();
        }
        rows.into_iter()
            .map(|row| self.view_model(row, snapshot_id))
            .collect()
    }

    fn view_model(&mut self, row: OpportunityListRow, snapshot_id: &str) -> OpportunityListViewRow {
        let id = row.id.clone();
        let next = OpportunityListViewModel::from_row(row, snapshot_id);
        if let Some(cached) = self.rows.get(&id) {
            if cached.as_ref() == &next {
                return Arc::clone(cached);
            }
        }
        let view = Arc::new(next);
        self.rows.insert(id, Arc::clone(&view));
        view
    }
}
