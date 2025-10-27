use crate::asset_browser::AssetBrowserState;

use super::system::EditorSystem;

#[derive(Default)]
pub(crate) struct AssetBrowserSystem {
    state: AssetBrowserState,
}

impl AssetBrowserSystem {
    pub(crate) fn new(state: AssetBrowserState) -> Self {
        Self { state }
    }

    pub(crate) fn state_mut(&mut self) -> &mut AssetBrowserState {
        &mut self.state
    }
}

impl EditorSystem for AssetBrowserSystem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
