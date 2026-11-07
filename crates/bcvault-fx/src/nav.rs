use crate::actions::{FxQuery, ViewMode};
use bcvault_core::types::ItemKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ViewState {
    pub view: ViewMode,
    pub query: FxQuery,
    pub scroll_y: f32,
    pub focus: Option<ItemKey>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub id: i64,
    pub name: String,
    pub state: ViewState,
    pub pos: i32,
}

#[derive(Clone, Debug)]
pub struct NavHistory {
    back: Vec<ViewState>,
    forward: Vec<ViewState>,
    max_entries: usize,
}

impl Default for NavHistory {
    fn default() -> Self {
        Self::new(64)
    }
}

impl NavHistory {
    pub const fn new(max_entries: usize) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            max_entries,
        }
    }

    pub fn push(&mut self, current: ViewState) {
        if self
            .back
            .last()
            .is_some_and(|last| last.view == current.view && last.query == current.query)
        {
            return;
        }
        self.back.push(current);
        if self.back.len() > self.max_entries {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    pub fn go_back(&mut self, current: ViewState) -> Option<ViewState> {
        let prev = self.back.pop()?;
        self.forward.push(current);
        Some(prev)
    }

    pub fn go_forward(&mut self, current: ViewState) -> Option<ViewState> {
        let next = self.forward.pop()?;
        self.back.push(current);
        Some(next)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bcvault_core::types::ReleaseFilter;

    #[test]
    fn nav_history_back_and_forward_roundtrip() {
        let mut nav = NavHistory::new(10);
        let s1 = ViewState {
            view: ViewMode::Wall,
            scroll_y: 120.0,
            ..Default::default()
        };
        let mut s2 = s1.clone();
        s2.query.base.filter = ReleaseFilter::Free;
        s2.scroll_y = 340.0;

        nav.push(s1.clone());
        assert!(nav.can_back());
        assert!(!nav.can_forward());

        let restored = nav.go_back(s2.clone()).unwrap();
        assert_eq!(restored, s1);
        assert!(nav.can_forward());

        let forward = nav.go_forward(restored).unwrap();
        assert_eq!(forward, s2);
    }
}
