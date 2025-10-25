use crate::renderer::{
    CustomRenderCallback, CustomRenderRequest, CustomRenderStage, RenderFrame, RenderRegion,
};

pub struct RenderParams {
    pub render_region: Option<RenderRegion>,
}

pub enum RenderResult {
    Skipped,
    Rendered(RenderFrame),
}

pub struct RenderHooks {
    custom_render_callback: Option<Box<CustomRenderCallback>>,
    custom_render_stage: CustomRenderStage,
    custom_render_in_shadows: bool,
    custom_render_shadow_query: Option<Box<dyn FnMut() -> bool>>,
}

impl RenderHooks {
    pub fn new() -> Self {
        Self {
            custom_render_callback: None,
            custom_render_stage: CustomRenderStage::BeforePostprocess,
            custom_render_in_shadows: false,
            custom_render_shadow_query: None,
        }
    }

    pub fn set_custom_render_callback(&mut self, callback: Box<CustomRenderCallback>) {
        self.custom_render_callback = Some(callback);
        self.custom_render_stage = CustomRenderStage::BeforePostprocess;
    }

    pub fn set_custom_render_stage(&mut self, stage: CustomRenderStage) {
        self.custom_render_stage = stage;
    }

    pub fn enable_custom_render_shadows(&mut self, enabled: bool) {
        self.custom_render_in_shadows = enabled;
    }

    pub fn set_custom_render_shadow_query<F>(&mut self, query: F)
    where
        F: FnMut() -> bool + 'static,
    {
        self.custom_render_shadow_query = Some(Box::new(query));
    }

    pub fn prepare_request(&mut self) -> Option<CustomRenderRequest<'_>> {
        if let Some(query) = self.custom_render_shadow_query.as_mut() {
            self.custom_render_in_shadows = query();
        }

        self.custom_render_callback
            .as_mut()
            .map(|callback| CustomRenderRequest {
                callback: &mut **callback,
                stage: self.custom_render_stage,
                render_in_shadow_pass: self.custom_render_in_shadows,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_callback(_: &mut crate::renderer::CustomRenderContext<'_>) {}

    #[test]
    fn shadow_query_updates_flag() {
        let mut hooks = RenderHooks::new();
        hooks.set_custom_render_callback(Box::new(dummy_callback));
        hooks.enable_custom_render_shadows(false);
        hooks.set_custom_render_shadow_query(|| true);

        let request = hooks.prepare_request();
        assert!(request.is_some());
        assert!(hooks.custom_render_in_shadows);
    }
}
